using System.Collections.Concurrent;
using System.Text.RegularExpressions;
using SkiaSharp;

namespace Retrograde.Core;

public sealed record ScanProgress(string Phase, int Scanned, int Total, string CurrentFile);

/// <param name="MissingTracks">Library tracks under the scanned folders whose files are gone; removed only with pruneMissing.</param>
/// <param name="MergedAlbums">Albums an earlier scan had split by artist, put back together.</param>
public sealed record ScanResult(int AddedTracks, int NewAlbums, int Covers, int UpdatedTracks = 0, int RemovedTracks = 0, int MissingTracks = 0, int MergedAlbums = 0)
{
    public bool Changed => AddedTracks > 0 || Covers > 0 || UpdatedTracks > 0 || RemovedTracks > 0 || MergedAlbums > 0;
}

/// <summary>
/// Reads audio files into library.json (the file shared with the Electron build) and keeps it in step
/// with the disk: new files are added, files whose size or date changed are read again, and files that
/// are gone from a scanned folder are dropped. One cover per album goes into the shared cover cache.
/// </summary>
public static partial class Scanner
{
    public static readonly HashSet<string> AudioExtensions = new(StringComparer.OrdinalIgnoreCase)
    {
        ".flac", ".mp3", ".wav", ".aac", ".ogg", ".m4a", ".wma", ".aiff", ".alac", ".opus",
    };

    private const StringComparison PathCmp = StringComparison.OrdinalIgnoreCase;

    /// <summary>What a walk over the given paths found.</summary>
    private sealed class Discovery
    {
        public List<string> Files { get; } = [];
        /// <summary>Folders that were walked: a library track under one of them that was not found is gone.</summary>
        public List<string> WalkedDirs { get; } = [];
        /// <summary>Folders that could not be listed (access denied, I/O error): nothing under them is judged.</summary>
        public List<string> UnreadableDirs { get; } = [];
        /// <summary>Folders that no longer exist although their drive does: everything under them is gone.</summary>
        public List<string> GoneDirs { get; } = [];
    }

    /// <summary>Expands folders recursively and keeps audio files (folders and files may be mixed, e.g. drag and drop).</summary>
    public static List<string> Discover(IEnumerable<string> paths) => Walk(paths).Files;

    private static Discovery Walk(IEnumerable<string> paths)
    {
        var found = new Discovery();
        var stack = new Stack<string>();
        foreach (var p in paths)
        {
            if (Directory.Exists(p))
            {
                var dir = Path.GetFullPath(p);
                stack.Push(dir);
                found.WalkedDirs.Add(dir);
            }
            else if (File.Exists(p))
            {
                if (AudioExtensions.Contains(Path.GetExtension(p))) found.Files.Add(Path.GetFullPath(p));
            }
            else if (Path.IsPathFullyQualified(p) && Path.GetPathRoot(p) is { Length: > 0 } root && Directory.Exists(root))
            {
                // Neither a file nor a folder any more, but the drive is there: the folder was deleted or moved.
                // With the drive unplugged nothing is concluded, or a rescan would empty the library.
                found.GoneDirs.Add(Path.GetFullPath(p));
            }
        }
        while (stack.Count > 0)
        {
            var dir = stack.Pop();
            try
            {
                foreach (var d in Directory.EnumerateDirectories(dir)) stack.Push(d);
                foreach (var f in Directory.EnumerateFiles(dir))
                    if (AudioExtensions.Contains(Path.GetExtension(f))) found.Files.Add(f);
            }
            catch (UnauthorizedAccessException) { found.UnreadableDirs.Add(dir); }
            catch (IOException) { found.UnreadableDirs.Add(dir); }
        }
        return found;
    }

    private sealed class Parsed
    {
        public required string FilePath { get; init; }
        public required string Title { get; init; }
        public required string Artist { get; init; }
        public string? AlbumArtist { get; init; }
        public required string Album { get; init; }
        public double Duration { get; init; }
        public int TrackNumber { get; init; }
        public int DiscNumber { get; init; }
        public int Year { get; init; }
        public string Codec { get; init; } = "";
        public int SampleRate { get; init; }
        public int BitsPerSample { get; init; }
        public int Channels { get; init; }
        public bool HasPicture { get; init; }
    }

    /// <param name="pruneMissing">
    /// Drop library tracks whose files are gone from the scanned folders. Off by default: a moved music
    /// folder would otherwise empty half the library in one go, so callers ask the user first.
    /// </param>
    public static async Task<ScanResult> ScanAsync(IEnumerable<string> inputPaths, IProgress<ScanProgress>? progress,
        CancellationToken ct = default, bool pruneMissing = false)
    {
        progress?.Report(new ScanProgress("discovering", 0, 0, "Discovering files..."));
        var found = await Task.Run(() => Walk(inputPaths), ct);
        // One read-modify-write of library.json at a time (see LibraryStore.WriteLock).
        await LibraryStore.WriteLock.WaitAsync(ct);
        try
        {
            return await ScanLockedAsync(found, progress, pruneMissing, ct);
        }
        finally
        {
            LibraryStore.WriteLock.Release();
        }
    }

    private static async Task<ScanResult> ScanLockedAsync(Discovery found, IProgress<ScanProgress>? progress, bool pruneMissing, CancellationToken ct)
    {
        var library = await LibraryStore.LoadRawAsync(ct);
        var dirty = false;
        var touched = new HashSet<string>(StringComparer.Ordinal);
        var mixed = new HashSet<AlbumDto>();

        // 0. Albums that an earlier scan split by artist although their files share a folder are put back together.
        var merged = MergeSplitAlbums(library, touched, mixed);

        var byPath = new Dictionary<string, (string Key, AlbumDto Album, TrackDto Track)>(StringComparer.OrdinalIgnoreCase);
        foreach (var (key, album) in library)
            foreach (var t in album.Tracks ?? [])
                if (t.FilePath is not null) byPath.TryAdd(t.FilePath, (key, album, t));

        // 1. Sort the files on disk into new, changed and unchanged, by size and modification time.
        var toParse = new List<string>();
        var changed = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        var stamps = new Dictionary<string, (long Mtime, long Size)>(StringComparer.OrdinalIgnoreCase);
        var inScan = new HashSet<string>(StringComparer.Ordinal); // albums with files in the scanned set
        await Task.Run(() =>
        {
            foreach (var f in found.Files)
            {
                if (Stamp(f) is not { } stamp) continue; // vanished between listing and now
                stamps[f] = stamp;
                if (!byPath.TryGetValue(f, out var e))
                {
                    toParse.Add(f);
                    continue;
                }
                inScan.Add(e.Key);
                if (e.Track.Mtime is null || e.Track.Size is null)
                {
                    // Indexed before stamps were kept (or by the Electron build): taken as current, tracked from now on.
                    e.Track.Mtime = stamp.Mtime;
                    e.Track.Size = stamp.Size;
                    dirty = true;
                }
                else if (e.Track.Mtime != stamp.Mtime || e.Track.Size != stamp.Size)
                {
                    toParse.Add(f);
                    changed.Add(f);
                }
            }
        }, ct);

        // 2. Library tracks that should have been found and were not.
        int removed = 0, missing = 0;
        if (found.WalkedDirs.Count > 0 || found.GoneDirs.Count > 0)
        {
            var seen = new HashSet<string>(found.Files, StringComparer.OrdinalIgnoreCase);
            var gone = await Task.Run(() => byPath.Values
                .Where(e => !seen.Contains(e.Track.FilePath!)
                            && !IsUnderAny(e.Track.FilePath!, found.UnreadableDirs)
                            && (IsUnderAny(e.Track.FilePath!, found.WalkedDirs) || IsUnderAny(e.Track.FilePath!, found.GoneDirs))
                            && !File.Exists(e.Track.FilePath!))
                .ToList(), ct);
            missing = gone.Count;
            if (!pruneMissing) gone.Clear();
            foreach (var e in gone)
            {
                e.Album.Tracks!.Remove(e.Track);
                byPath.Remove(e.Track.FilePath!);
                touched.Add(e.Key);
                removed++;
            }
        }

        // 3. Tags of the new and changed files, 4 threads like the Electron worker pool.
        var results = new ConcurrentBag<Parsed>();
        if (toParse.Count > 0)
        {
            int done = 0;
            int step = Math.Max(1, toParse.Count / 100);
            progress?.Report(new ScanProgress("scanning", 0, toParse.Count, ""));
            await Task.Run(() => Parallel.ForEach(toParse, new ParallelOptions { MaxDegreeOfParallelism = Math.Min(4, Environment.ProcessorCount), CancellationToken = ct }, f =>
            {
                var p = Parse(f);
                if (p is not null) results.Add(p);
                var n = Interlocked.Increment(ref done);
                if (n % step == 0 || n == toParse.Count)
                    progress?.Report(new ScanProgress("scanning", n, toParse.Count, Path.GetFileName(f)));
            }), ct);
        }

        // 4. Merge. Order by path so album creation is deterministic.
        var folders = new Dictionary<AlbumDto, AlbumFolders>();
        var pictureSource = new Dictionary<string, string>(StringComparer.Ordinal);
        int added = 0, updated = 0, newAlbums = 0;
        var now = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
        foreach (var r in results.OrderBy(r => r.FilePath, StringComparer.OrdinalIgnoreCase))
        {
            var isUpdate = false;
            if (changed.Contains(r.FilePath) && byPath.TryGetValue(r.FilePath, out var old))
            {
                // The tags may have moved it to another album, so it is taken out and merged like a new file.
                old.Album.Tracks!.Remove(old.Track);
                byPath.Remove(r.FilePath);
                folders.Remove(old.Album);
                touched.Add(old.Key);
                isUpdate = true;
            }

            var (key, album, created) = ResolveAlbum(library, folders, r, now);
            if (created) newAlbums++;
            album.Tracks ??= [];
            if (album.Year == 0 && r.Year > 0) album.Year = r.Year;
            if (!created && !string.IsNullOrWhiteSpace(r.AlbumArtist) && !string.IsNullOrWhiteSpace(album.AlbumArtist)
                && !SameArtist(album.AlbumArtist, r.AlbumArtist))
                mixed.Add(album); // an EP / compilation tagged with each track's own singer as album artist
            else if (string.IsNullOrEmpty(album.AlbumArtist) && !string.IsNullOrEmpty(r.AlbumArtist) && !mixed.Contains(album)) album.AlbumArtist = r.AlbumArtist;
            if (r.HasPicture && !pictureSource.ContainsKey(key)) pictureSource[key] = r.FilePath;
            if (byPath.ContainsKey(r.FilePath)) continue;

            var stamp = stamps.TryGetValue(r.FilePath, out var st) ? st : ((long, long)?)null;
            var track = new TrackDto
            {
                Id = r.FilePath,
                Title = r.Title,
                Artist = r.Artist,
                Album = key,
                Duration = r.Duration,
                FilePath = r.FilePath,
                Src = "file://" + r.FilePath.Replace('\\', '/'),
                TrackNumber = r.TrackNumber,
                DiscNumber = r.DiscNumber <= 0 ? 1 : r.DiscNumber,
                Codec = r.Codec,
                SampleRate = r.SampleRate,
                BitsPerSample = r.BitsPerSample,
                Channels = r.Channels,
                Mtime = stamp?.Item1,
                Size = stamp?.Item2,
            };
            album.Tracks.Add(track);
            byPath[r.FilePath] = (key, album, track);
            FoldersOf(folders, album).Add(r.FilePath);
            touched.Add(key);
            inScan.Add(key);
            if (isUpdate) updated++; else added++;
        }

        // Albums whose tracks name different album artists: show the artist every track shares
        // ("Cö shu Nie & Wuthering Waves" + "Wuthering Waves, SoYeon" -> "Wuthering Waves"), else Various Artists.
        // Same for albums without an album-artist tag whose tracks are by different artists (a compilation).
        foreach (var key in touched)
            if (library.TryGetValue(key, out var a) && string.IsNullOrWhiteSpace(a.AlbumArtist) && a.Tracks is { Count: > 1 }
                && a.Tracks.Select(t => NormalizeArtist(t.Artist ?? "")).Distinct(StringComparer.Ordinal).Count() > 1)
                mixed.Add(a);
        foreach (var album in mixed)
        {
            if (album.Tracks is not { Count: > 0 }) continue;
            album.AlbumArtist = null;
            album.Artist = CommonArtist(album.Tracks.Select(t => t.Artist));
        }

        // 5. Albums left without tracks go, with their covers; the others are put back in order.
        foreach (var key in touched)
        {
            if (!library.TryGetValue(key, out var album)) continue;
            if (album.Tracks is null || album.Tracks.Count == 0)
            {
                library.Remove(key);
                LibraryStore.TryDelete(CoverPaths.ThumbPath(key));
                LibraryStore.TryDelete(CoverPaths.FullPath(key));
                continue;
            }
            album.Tracks.Sort((a, b) =>
            {
                var d = (a.DiscNumber <= 0 ? 1 : a.DiscNumber) - (b.DiscNumber <= 0 ? 1 : b.DiscNumber);
                return d != 0 ? d : (a.TrackNumber ?? 0) - (b.TrackNumber ?? 0);
            });
        }

        // 6. One cover per album that still has none: embedded art first, then an image file in its folder.
        var needing = inScan.Where(k => library.TryGetValue(k, out var a) && string.IsNullOrEmpty(a.CoverArt)).ToList();
        int covers = 0;
        for (int i = 0; i < needing.Count; i++)
        {
            ct.ThrowIfCancellationRequested();
            var key = needing[i];
            var album = library[key];
            progress?.Report(new ScanProgress("covers", i, needing.Count, key));
            pictureSource.TryGetValue(key, out var source);
            var saved = await Task.Run(() => FindAndSaveCover(key, album, source), ct);
            if (saved is null) continue;
            album.CoverArt = saved.Value.Thumb;
            album.CoverArtFull = saved.Value.Full;
            covers++;
        }

        var result = new ScanResult(added, newAlbums, covers, updated, removed, pruneMissing ? 0 : missing, merged);
        if (mixed.Count > 0) dirty = true;
        if (result.Changed || dirty) LibraryStore.SaveRaw(library);
        return result;
    }

    private static (long Mtime, long Size)? Stamp(string path)
    {
        try
        {
            var info = new FileInfo(path);
            return info.Exists ? (new DateTimeOffset(info.LastWriteTimeUtc).ToUnixTimeMilliseconds(), info.Length) : null;
        }
        catch { return null; }
    }

    private static bool IsUnderAny(string path, List<string> dirs)
    {
        foreach (var d in dirs)
        {
            var dir = d.TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
            if (path.Length > dir.Length + 1 && path.StartsWith(dir, PathCmp)
                && path[dir.Length] is '\\' or '/')
                return true;
        }
        return false;
    }

    // ---------- Which album a file belongs to ----------

    [GeneratedRegex(@"^(cd|disc|disk|dvd|vol|volume)[\s._-]*\d+", RegexOptions.IgnoreCase | RegexOptions.CultureInvariant)]
    private static partial Regex DiscFolder();

    /// <summary>The folder an album lives in: the file's folder, or the one above a "Disc 1" style folder.</summary>
    private static string AlbumFolder(string filePath)
    {
        var dir = Path.GetDirectoryName(filePath) ?? "";
        if (DiscFolder().IsMatch(Path.GetFileName(dir)) && Path.GetDirectoryName(dir) is { Length: > 0 } parent) return parent;
        return dir;
    }

    /// <summary>The folders an album's files are in, and the folders above those.</summary>
    private sealed class AlbumFolders
    {
        private readonly HashSet<string> _own = new(StringComparer.OrdinalIgnoreCase);
        private readonly HashSet<string> _parents = new(StringComparer.OrdinalIgnoreCase);

        public void Add(string filePath)
        {
            var folder = AlbumFolder(filePath);
            if (_own.Add(folder) && Path.GetDirectoryName(folder) is { Length: > 0 } parent) _parents.Add(parent);
        }

        /// <summary>The file sits in one of this album's own folders (a "Disc N" folder counts as its parent).</summary>
        public bool IsSameFolder(string filePath) => _own.Contains(AlbumFolder(filePath));

        /// <summary>Same folder, a neighbouring one, or one directly above or below.</summary>
        public bool IsNear(string filePath)
        {
            var folder = AlbumFolder(filePath);
            if (_own.Contains(folder) || _parents.Contains(folder)) return true;
            return Path.GetDirectoryName(folder) is { Length: > 0 } parent && (_parents.Contains(parent) || _own.Contains(parent));
        }
    }

    private static AlbumFolders FoldersOf(Dictionary<AlbumDto, AlbumFolders> cache, AlbumDto album)
    {
        if (cache.TryGetValue(album, out var f)) return f;
        f = new AlbumFolders();
        foreach (var t in album.Tracks ?? [])
            if (t.FilePath is not null) f.Add(t.FilePath);
        cache[album] = f;
        return f;
    }

    private static bool SameText(string? a, string? b) => SameArtist(a, b);

    /// <summary>Artist names compared without case and spacing: "Wuthering Waves / jixwang" = "Wuthering Waves/ jixwang".</summary>
    private static bool SameArtist(string? a, string? b) =>
        a is not null && b is not null && string.Equals(NormalizeArtist(a), NormalizeArtist(b), StringComparison.Ordinal);

    private static string NormalizeArtist(string s) =>
        new string(s.Where(c => !char.IsWhiteSpace(c)).Select(char.ToLowerInvariant).ToArray());

    [GeneratedRegex(@"\s*(?:/|&|,|;|、|，|＆|\s+feat\.?\s+|\s+ft\.?\s+|\s+x\s+|\s+with\s+)\s*", RegexOptions.IgnoreCase | RegexOptions.CultureInvariant)]
    private static partial Regex ArtistSeparator();

    /// <summary>The artist named on every track (split on / & , feat. and the like), else "Various Artists".</summary>
    internal static string CommonArtist(IEnumerable<string?> trackArtists)
    {
        List<string>? common = null;
        foreach (var artist in trackArtists)
        {
            var names = ArtistSeparator().Split(artist ?? "").Select(n => n.Trim()).Where(n => n.Length > 0).ToList();
            if (common is null) { common = names; continue; }
            var keys = names.Select(NormalizeArtist).ToHashSet(StringComparer.Ordinal);
            common = common.Where(n => keys.Contains(NormalizeArtist(n))).ToList();
            if (common.Count == 0) break;
        }
        return common is { Count: > 0 } ? common[0] : "Various Artists";
    }

    [GeneratedRegex(@"^(?<base>.+) \((?<who>[^()]+)\)(?: \d+)?$", RegexOptions.CultureInvariant)]
    private static partial Regex SplitAlbumKey();

    /// <summary>
    /// Undoes splits made by earlier scans: "Name (Artist)" next to "Name", when the two share a folder,
    /// was one album whose tracks carry different album-artist tags. Only keys in the exact shape this
    /// scanner produced are considered, so a real "(Deluxe Edition)" album is left alone.
    /// </summary>
    private static int MergeSplitAlbums(Dictionary<string, AlbumDto> library, HashSet<string> touched, HashSet<AlbumDto> mixed)
    {
        var merged = 0;
        foreach (var key in library.Keys.ToList())
        {
            if (!library.TryGetValue(key, out var split) || split.Tracks is not { Count: > 0 }) continue;
            var m = SplitAlbumKey().Match(key);
            if (!m.Success) continue;
            var baseKey = m.Groups["base"].Value;
            if (!library.TryGetValue(baseKey, out var target) || ReferenceEquals(target, split) || target.Tracks is not { Count: > 0 }) continue;
            var who = m.Groups["who"].Value;
            if (!SameArtist(who, split.AlbumArtist) && !SameArtist(who, split.Artist)) continue; // not one of ours
            var targetFolders = target.Tracks.Where(t => t.FilePath is not null).Select(t => AlbumFolder(t.FilePath!)).ToHashSet(StringComparer.OrdinalIgnoreCase);
            if (!split.Tracks.Any(t => t.FilePath is not null && targetFolders.Contains(AlbumFolder(t.FilePath!)))) continue;

            foreach (var t in split.Tracks)
            {
                t.Album = baseKey;
                target.Tracks.Add(t);
            }
            if (string.IsNullOrEmpty(target.CoverArt) && !string.IsNullOrEmpty(split.CoverArt))
            {
                target.CoverArt = split.CoverArt; // keep the split's cover file: it is the only one there is
                target.CoverArtFull = split.CoverArtFull;
            }
            else
            {
                LibraryStore.TryDelete(CoverPaths.ThumbPath(key));
                LibraryStore.TryDelete(CoverPaths.FullPath(key));
            }
            if (target.Year == 0) target.Year = split.Year;
            target.AddedAt = Math.Min(target.AddedAt == 0 ? long.MaxValue : target.AddedAt, split.AddedAt == 0 ? long.MaxValue : split.AddedAt) is var at && at != long.MaxValue ? at : 0;
            library.Remove(key);
            // Tags that only differ in spacing ("A / B" vs "A/ B") keep the album artist as tagged.
            if (!SameArtist(split.AlbumArtist ?? split.Artist, target.AlbumArtist ?? target.Artist)) mixed.Add(target);
            touched.Add(baseKey);
            merged++;
        }
        return merged;
    }

    /// <summary>
    /// The album name alone is not an identity: "Greatest Hits" exists once per artist. Two files with
    /// the same album name are the same album when they sit in the same folder (whatever their tags say:
    /// an EP may tag every track with its own singer as album artist); otherwise when their album-artist
    /// tags agree, or, where a tag is missing, when they are in a neighbouring folder or by the same artist.
    /// </summary>
    private static bool SameAlbum(Dictionary<AlbumDto, AlbumFolders> cache, AlbumDto album, Parsed r)
    {
        if (album.Tracks is null || album.Tracks.Count == 0) return true;
        // Same name in the same folder is the same album, whatever the artist tags say: EPs and
        // compilations often carry each track's own singer as its album artist.
        if (FoldersOf(cache, album).IsSameFolder(r.FilePath)) return true;
        if (!string.IsNullOrWhiteSpace(album.AlbumArtist) && !string.IsNullOrWhiteSpace(r.AlbumArtist))
            return SameText(album.AlbumArtist, r.AlbumArtist);
        if (FoldersOf(cache, album).IsNear(r.FilePath)) return true;
        return SameText(album.Artist, r.AlbumArtist ?? r.Artist) || SameText(album.AlbumArtist, r.Artist);
    }

    /// <summary>
    /// Finds the album for a file, creating it if needed. A second album of the same name is stored as
    /// "Name (Artist)": the name stays the key of library.json and of the cover cache, which is all the
    /// Electron build knows about, so both builds keep reading the same file.
    /// </summary>
    private static (string Key, AlbumDto Album, bool Created) ResolveAlbum(
        Dictionary<string, AlbumDto> library, Dictionary<AlbumDto, AlbumFolders> cache, Parsed r, long now)
    {
        var artist = string.IsNullOrWhiteSpace(r.AlbumArtist) ? r.Artist : r.AlbumArtist!;
        for (int attempt = 0; ; attempt++)
        {
            var key = attempt switch
            {
                0 => r.Album,
                1 => $"{r.Album} ({artist})",
                _ => $"{r.Album} ({artist}) {attempt}",
            };
            if (!library.TryGetValue(key, out var album))
            {
                album = new AlbumDto
                {
                    Name = key,
                    Artist = artist,
                    AlbumArtist = string.IsNullOrWhiteSpace(r.AlbumArtist) ? null : r.AlbumArtist,
                    Year = 0,
                    AddedAt = now,
                    Tracks = [],
                };
                library[key] = album;
                return (key, album, true);
            }
            if (attempt >= 20 || SameAlbum(cache, album, r)) return (key, album, false);
        }
    }

    // ---------- Tags ----------

    private static Parsed? Parse(string path)
    {
        try
        {
            // PictureLazy: note that a picture exists without reading its bytes. Covers are several MB per
            // file and only one per album is ever used (ReadEmbeddedPicture reads that one in full).
            using var file = TagLib.File.Create(path, TagLib.ReadStyle.Average | TagLib.ReadStyle.PictureLazy);
            var tag = file.Tag;
            var props = file.Properties;
            var albumArtist = FirstNonEmpty(tag.FirstAlbumArtist);
            return new Parsed
            {
                FilePath = path,
                Title = FirstNonEmpty(tag.Title) ?? Path.GetFileNameWithoutExtension(path),
                Artist = FirstNonEmpty(tag.FirstPerformer, albumArtist) ?? "Unknown Artist",
                AlbumArtist = albumArtist,
                Album = FirstNonEmpty(tag.Album) ?? "Unknown Album",
                Duration = props?.Duration.TotalSeconds ?? 0,
                TrackNumber = (int)tag.Track,
                DiscNumber = tag.Disc == 0 ? 1 : (int)tag.Disc,
                Year = (int)tag.Year,
                Codec = CodecName(file, path),
                SampleRate = props?.AudioSampleRate ?? 0,
                BitsPerSample = props?.BitsPerSample ?? 0,
                Channels = props?.AudioChannels ?? 0,
                HasPicture = tag.Pictures is { Length: > 0 },
            };
        }
        catch
        {
            return null; // unreadable / unsupported file: skipped like a worker error
        }
    }

    private static string? FirstNonEmpty(params string?[] values)
    {
        foreach (var v in values) if (!string.IsNullOrWhiteSpace(v)) return v.Trim();
        return null;
    }

    /// <summary>Codec names in the shape the Electron build stores (music-metadata / FLAC parser).</summary>
    private static string CodecName(TagLib.File file, string path)
    {
        var ext = Path.GetExtension(path).ToLowerInvariant();
        var desc = file.Properties?.Codecs.FirstOrDefault(c => c is not null)?.Description ?? "";
        return ext switch
        {
            ".flac" => "FLAC",
            ".mp3" => "MPEG 1 Layer 3",
            ".opus" => "Opus",
            ".ogg" => desc.Contains("opus", StringComparison.OrdinalIgnoreCase) ? "Opus" : "Vorbis I",
            ".wav" or ".aiff" => "PCM",
            ".wma" => "Windows Media Audio",
            ".m4a" or ".alac" or ".aac" => desc.Contains("lossless", StringComparison.OrdinalIgnoreCase) || desc.Contains("alac", StringComparison.OrdinalIgnoreCase) ? "ALAC" : "AAC",
            _ => ext.TrimStart('.').ToUpperInvariant(),
        };
    }

    // ---------- Covers ----------

    private static readonly string[] FolderCoverNames = ["cover", "folder", "front", "album", "albumart"];
    private const long MaxCoverFileBytes = 30 * 1024 * 1024;

    private static (string Thumb, string Full)? FindAndSaveCover(string albumKey, AlbumDto album, string? pictureSource)
    {
        try
        {
            byte[]? data = pictureSource is null ? null : ReadEmbeddedPicture(pictureSource);
            var tracks = (album.Tracks ?? []).Where(t => t.FilePath is not null).Select(t => t.FilePath!).ToList();
            // Already-indexed albums were not parsed in this pass: look into their first files.
            foreach (var path in tracks.Take(3))
            {
                if (data is not null) break;
                if (!string.Equals(path, pictureSource, PathCmp)) data = ReadEmbeddedPicture(path);
            }
            // No embedded art: cover.jpg, folder.jpg and the like next to the files.
            foreach (var folder in tracks.Select(p => Path.GetDirectoryName(p) ?? "").Concat(tracks.Select(AlbumFolder))
                         .Where(f => f.Length > 0).Distinct(StringComparer.OrdinalIgnoreCase).Take(8))
            {
                if (data is not null) break;
                data = ReadFolderImage(folder);
            }
            return data is null ? null : SaveCover(albumKey, data);
        }
        catch
        {
            return null;
        }
    }

    private static byte[]? ReadEmbeddedPicture(string sourceFile)
    {
        try
        {
            using var file = TagLib.File.Create(sourceFile);
            var pic = file.Tag.Pictures.FirstOrDefault(p => p.Type == TagLib.PictureType.FrontCover) ?? file.Tag.Pictures.FirstOrDefault();
            return pic is null || pic.Data.Count == 0 ? null : pic.Data.Data;
        }
        catch
        {
            return null;
        }
    }

    private static byte[]? ReadFolderImage(string folder)
    {
        try
        {
            if (!Directory.Exists(folder)) return null;
            var images = Directory.EnumerateFiles(folder)
                .Where(f => PlaylistCovers.Extensions.Contains(Path.GetExtension(f).ToLowerInvariant()))
                .ToList();
            foreach (var name in FolderCoverNames)
            {
                var match = images.FirstOrDefault(f => string.Equals(Path.GetFileNameWithoutExtension(f), name, StringComparison.OrdinalIgnoreCase));
                if (match is not null && new FileInfo(match).Length <= MaxCoverFileBytes) return File.ReadAllBytes(match);
            }
            return null;
        }
        catch
        {
            return null;
        }
    }

    /// <summary>saveCoverArtBuffer: full image as-is, 400px-wide JPEG(80) thumb. Existing files are kept.</summary>
    private static (string Thumb, string Full)? SaveCover(string albumName, byte[] data)
    {
        // Art comes from files of any origin: refuse what is not an image or would not fit in memory.
        if (!ImageLimits.IsSane(data)) return null;
        Directory.CreateDirectory(CoverPaths.CoverDir);
        var thumb = CoverPaths.ThumbPath(albumName);
        var full = CoverPaths.FullPath(albumName);
        if (!File.Exists(full)) File.WriteAllBytes(full, data);
        if (!File.Exists(thumb))
        {
            byte[] thumbBytes = data;
            using var bmp = SKBitmap.Decode(data);
            if (bmp is not null && bmp.Width > 400)
            {
                var h = (int)Math.Round(bmp.Height * (400.0 / bmp.Width));
                using var resized = bmp.Resize(new SKImageInfo(400, h), new SKSamplingOptions(SKCubicResampler.Mitchell));
                if (resized is not null)
                {
                    using var img = SKImage.FromBitmap(resized);
                    using var enc = img.Encode(SKEncodedImageFormat.Jpeg, 80);
                    thumbBytes = enc.ToArray();
                }
            }
            File.WriteAllBytes(thumb, thumbBytes);
        }
        return (ToUrl(thumb), ToUrl(full));
    }

    private static string ToUrl(string path) => "file://" + path.Replace('\\', '/');
}
