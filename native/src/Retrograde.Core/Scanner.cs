using System.Collections.Concurrent;
using SkiaSharp;

namespace Retrograde.Core;

public sealed record ScanProgress(string Phase, int Scanned, int Total, string CurrentFile);

public sealed record ScanResult(int AddedTracks, int NewAlbums, int Covers);

/// <summary>
/// Port of electron/scanner.js: discover audio files, read tags (TagLib#), merge into
/// library.json by album name, extract one cover per album into the shared cover cache.
/// Existing tracks are never rewritten, same as the Electron build.
/// </summary>
public static class Scanner
{
    public static readonly HashSet<string> AudioExtensions = new(StringComparer.OrdinalIgnoreCase)
    {
        ".flac", ".mp3", ".wav", ".aac", ".ogg", ".m4a", ".wma", ".aiff", ".alac", ".opus",
    };

    /// <summary>Expands folders recursively and keeps audio files (folders and files may be mixed, e.g. drag and drop).</summary>
    public static List<string> Discover(IEnumerable<string> paths)
    {
        var result = new List<string>();
        var stack = new Stack<string>();
        foreach (var p in paths)
        {
            if (Directory.Exists(p)) stack.Push(p);
            else if (File.Exists(p) && AudioExtensions.Contains(Path.GetExtension(p))) result.Add(Path.GetFullPath(p));
        }
        while (stack.Count > 0)
        {
            var dir = stack.Pop();
            try
            {
                foreach (var d in Directory.EnumerateDirectories(dir)) stack.Push(d);
                foreach (var f in Directory.EnumerateFiles(dir))
                    if (AudioExtensions.Contains(Path.GetExtension(f))) result.Add(f);
            }
            catch (UnauthorizedAccessException) { }
            catch (IOException) { }
        }
        return result;
    }

    private sealed class Parsed
    {
        public required string FilePath { get; init; }
        public required string Title { get; init; }
        public required string Artist { get; init; }
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

    public static async Task<ScanResult> ScanAsync(IEnumerable<string> inputPaths, IProgress<ScanProgress>? progress, CancellationToken ct = default)
    {
        progress?.Report(new ScanProgress("discovering", 0, 0, "Discovering files..."));
        var all = await Task.Run(() => Discover(inputPaths), ct);
        // One read-modify-write of library.json at a time (see LibraryStore.WriteLock).
        await LibraryStore.WriteLock.WaitAsync(ct);
        try
        {
            return await ScanLockedAsync(all, progress, ct);
        }
        finally
        {
            LibraryStore.WriteLock.Release();
        }
    }

    private static async Task<ScanResult> ScanLockedAsync(List<string> all, IProgress<ScanProgress>? progress, CancellationToken ct)
    {
        var library = await LibraryStore.LoadRawAsync(ct);

        // Skip files already indexed in albums that have a cover (filterKnownFiles).
        var known = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        foreach (var album in library.Values)
        {
            if (string.IsNullOrEmpty(album.CoverArt)) continue;
            foreach (var t in album.Tracks ?? []) if (t.FilePath is not null) known.Add(t.FilePath);
        }
        var files = all.Where(f => !known.Contains(f)).ToList();
        if (files.Count == 0) return new ScanResult(0, 0, 0);

        // 1. Tags, 4 threads like the Electron worker pool.
        var results = new ConcurrentBag<Parsed>();
        int done = 0;
        int step = Math.Max(1, files.Count / 100);
        progress?.Report(new ScanProgress("scanning", 0, files.Count, ""));
        await Task.Run(() => Parallel.ForEach(files, new ParallelOptions { MaxDegreeOfParallelism = Math.Min(4, Environment.ProcessorCount), CancellationToken = ct }, f =>
        {
            var p = Parse(f);
            if (p is not null) results.Add(p);
            var n = Interlocked.Increment(ref done);
            if (n % step == 0 || n == files.Count)
                progress?.Report(new ScanProgress("scanning", n, files.Count, Path.GetFileName(f)));
        }), ct);

        // 2. Merge by album (processResults). Order by path so album creation is deterministic.
        var coverSource = new Dictionary<string, string>(StringComparer.Ordinal);
        var paths = new Dictionary<string, HashSet<string>>(StringComparer.Ordinal);
        int added = 0, newAlbums = 0;
        var now = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
        foreach (var r in results.OrderBy(r => r.FilePath, StringComparer.OrdinalIgnoreCase))
        {
            if (!library.TryGetValue(r.Album, out var album))
            {
                album = new AlbumDto { Name = r.Album, Artist = r.Artist, Year = 0, AddedAt = now, Tracks = [] };
                library[r.Album] = album;
                newAlbums++;
            }
            album.Tracks ??= [];
            if (album.Year == 0 && r.Year > 0) album.Year = r.Year;
            if (r.HasPicture && !coverSource.ContainsKey(r.Album)) coverSource[r.Album] = r.FilePath;

            if (!paths.TryGetValue(r.Album, out var set))
            {
                set = new HashSet<string>(album.Tracks.Where(t => t.FilePath is not null).Select(t => t.FilePath!), StringComparer.OrdinalIgnoreCase);
                paths[r.Album] = set;
            }
            if (!set.Add(r.FilePath)) continue;
            album.Tracks.Add(new TrackDto
            {
                Id = r.FilePath,
                Title = r.Title,
                Artist = r.Artist,
                Album = r.Album,
                Duration = r.Duration,
                FilePath = r.FilePath,
                Src = "file://" + r.FilePath.Replace('\\', '/'),
                TrackNumber = r.TrackNumber,
                DiscNumber = r.DiscNumber <= 0 ? 1 : r.DiscNumber,
                Codec = r.Codec,
                SampleRate = r.SampleRate,
                BitsPerSample = r.BitsPerSample,
                Channels = r.Channels,
            });
            added++;
        }
        foreach (var name in paths.Keys)
            library[name].Tracks!.Sort((a, b) =>
            {
                var d = (a.DiscNumber <= 0 ? 1 : a.DiscNumber) - (b.DiscNumber <= 0 ? 1 : b.DiscNumber);
                return d != 0 ? d : (a.TrackNumber ?? 0) - (b.TrackNumber ?? 0);
            });

        // 3. One cover per album that still has none.
        var needing = coverSource.Keys.Where(n => string.IsNullOrEmpty(library[n].CoverArt)).ToList();
        int covers = 0;
        for (int i = 0; i < needing.Count; i++)
        {
            ct.ThrowIfCancellationRequested();
            var name = needing[i];
            progress?.Report(new ScanProgress("covers", i, needing.Count, name));
            var saved = await Task.Run(() => SaveCover(name, coverSource[name]), ct);
            if (saved is not null)
            {
                library[name].CoverArt = saved.Value.Thumb;
                library[name].CoverArtFull = saved.Value.Full;
                covers++;
            }
        }

        LibraryStore.SaveRaw(library);
        return new ScanResult(added, newAlbums, covers);
    }

    private static Parsed? Parse(string path)
    {
        try
        {
            // PictureLazy: note that a picture exists without reading its bytes. Covers are several MB per
            // file and only one per album is ever used (SaveCover reads that one in full).
            using var file = TagLib.File.Create(path, TagLib.ReadStyle.Average | TagLib.ReadStyle.PictureLazy);
            var tag = file.Tag;
            var props = file.Properties;
            var artist = FirstNonEmpty(tag.FirstPerformer, tag.FirstAlbumArtist) ?? "Unknown Artist";
            return new Parsed
            {
                FilePath = path,
                Title = FirstNonEmpty(tag.Title) ?? Path.GetFileNameWithoutExtension(path),
                Artist = artist,
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

    /// <summary>saveCoverArtBuffer: full image as-is, 400px-wide JPEG(80) thumb. Existing files are kept.</summary>
    private static (string Thumb, string Full)? SaveCover(string albumName, string sourceFile)
    {
        try
        {
            byte[] data;
            using (var file = TagLib.File.Create(sourceFile))
            {
                var pic = file.Tag.Pictures.FirstOrDefault(p => p.Type == TagLib.PictureType.FrontCover) ?? file.Tag.Pictures.FirstOrDefault();
                if (pic is null || pic.Data.Count == 0) return null;
                data = pic.Data.Data;
            }
            // Embedded art comes from files of any origin: refuse what is not an image or would not fit in memory.
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
        catch
        {
            return null;
        }
    }

    private static string ToUrl(string path) => "file://" + path.Replace('\\', '/');
}
