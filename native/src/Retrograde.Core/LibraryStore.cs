using System.Text.Json;
using System.Text.Json.Serialization;

namespace Retrograde.Core;

/// <summary>
/// Reads the library written by the Electron build (~/.retrograde/library.json),
/// so the native app starts with the existing catalog and cover cache.
/// </summary>
public static class LibraryStore
{
    public static string LibraryPath => Path.Combine(CoverPaths.AppDir, "library.json");
    private static string BackupPath => LibraryPath + ".bak";
    /// <summary>Where a library.json that could not be parsed is kept for manual recovery.</summary>
    public static string DamagedPath => LibraryPath + ".corrupt";

    /// <summary>
    /// Held for every read-modify-write of library.json (scan, delete), so two of them
    /// cannot both start from the same file and overwrite the changes of the other.
    /// </summary>
    public static SemaphoreSlim WriteLock { get; } = new(1, 1);

    /// <summary>Set when library.json was unreadable and the previous version (.bak) was put back.</summary>
    public static bool RestoredFromBackup { get; private set; }

    // Keep CJK titles as UTF-8 text (like JSON.stringify) instead of \uXXXX escapes.
    private static readonly LibraryJsonContext WriteContext = new(new JsonSerializerOptions
    {
        PropertyNameCaseInsensitive = true,
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping,
    });

    public static async Task<IReadOnlyList<Album>> LoadAsync(CancellationToken ct = default)
    {
        var dto = await LoadRawAsync(ct);
        return dto.Values.Select(Map).ToList();
    }

    /// <summary>The library.json dictionary as stored (album name → album), for merging scans.</summary>
    public static async Task<Dictionary<string, AlbumDto>> LoadRawAsync(CancellationToken ct = default)
    {
        if (!File.Exists(LibraryPath)) return new Dictionary<string, AlbumDto>();
        try
        {
            return await ReadAsync(LibraryPath, ct);
        }
        catch (JsonException)
        {
            // Truncated or garbled (a crash or power cut mid-write in the Electron build, which writes in place).
            // Never treat that as "empty": the next save would then replace the catalog with almost nothing.
        }

        File.Copy(LibraryPath, DamagedPath, overwrite: true);
        if (File.Exists(BackupPath))
        {
            try
            {
                var previous = await ReadAsync(BackupPath, ct);
                File.Copy(BackupPath, LibraryPath, overwrite: true);
                RestoredFromBackup = true;
                return previous;
            }
            catch (JsonException) { /* the backup is unreadable too */ }
        }
        throw new InvalidDataException("library.json is damaged and there is no usable backup. It was copied to library.json.corrupt; remove library.json to start a new library.");
    }

    private static async Task<Dictionary<string, AlbumDto>> ReadAsync(string path, CancellationToken ct)
    {
        await using var fs = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite, 1 << 16, useAsync: true);
        return await JsonSerializer.DeserializeAsync(fs, LibraryJsonContext.Default.DictionaryStringAlbumDto, ct)
               ?? new Dictionary<string, AlbumDto>();
    }

    /// <summary>
    /// Writes library.json in the Electron layout (camelCase, file:// cover URLs), so both builds
    /// keep sharing it. Written to a temp file first; the previous file is kept as library.json.bak.
    /// </summary>
    public static void SaveRaw(Dictionary<string, AlbumDto> library)
    {
        Directory.CreateDirectory(CoverPaths.AppDir);
        var tmp = LibraryPath + ".tmp";
        using (var fs = new FileStream(tmp, FileMode.Create, FileAccess.Write, FileShare.None, 1 << 16))
            JsonSerializer.Serialize(fs, library, WriteContext.DictionaryStringAlbumDto);
        if (File.Exists(LibraryPath)) File.Replace(tmp, LibraryPath, BackupPath, ignoreMetadataErrors: true);
        else File.Move(tmp, LibraryPath);
    }

    /// <summary>DELETE_ALBUM / batch delete: drops the albums from library.json and removes their cover files.</summary>
    public static async Task<int> DeleteAlbumsAsync(IReadOnlyCollection<string> names, CancellationToken ct = default)
    {
        await WriteLock.WaitAsync(ct);
        try
        {
            var library = await LoadRawAsync(ct);
            var removed = 0;
            foreach (var name in names)
            {
                if (!library.Remove(name)) continue;
                removed++;
                TryDelete(CoverPaths.ThumbPath(name));
                TryDelete(CoverPaths.FullPath(name));
            }
            if (removed > 0) SaveRaw(library);
            return removed;
        }
        finally
        {
            WriteLock.Release();
        }
    }

    internal static void TryDelete(string path)
    {
        try { if (File.Exists(path)) File.Delete(path); } catch { /* in use: leave it, like the Electron build */ }
    }

    private static Album Map(AlbumDto a)
    {
        var name = a.Name ?? "Unknown Album";
        var thumb = CoverPaths.ToLocalPath(a.CoverArt);
        var full = CoverPaths.ToLocalPath(a.CoverArtFull) ?? thumb;
        var tracks = (a.Tracks ?? [])
            .Where(t => !string.IsNullOrEmpty(t.FilePath))
            .Select(t => new Track
            {
                Id = t.Id ?? t.FilePath!,
                Title = string.IsNullOrEmpty(t.Title) ? Path.GetFileNameWithoutExtension(t.FilePath!) : t.Title,
                Artist = t.Artist ?? a.Artist ?? "Unknown Artist",
                Album = name,
                Duration = t.Duration,
                FilePath = t.FilePath!,
                TrackNumber = t.TrackNumber ?? 0,
                DiscNumber = t.DiscNumber <= 0 ? 1 : t.DiscNumber,
                Codec = t.Codec ?? "",
                SampleRate = t.SampleRate,
                BitsPerSample = t.BitsPerSample,
                Channels = t.Channels,
                CoverThumbPath = thumb,
                CoverFullPath = full,
            })
            .ToList();
        return new Album
        {
            Name = name,
            Artist = a.Artist ?? "Unknown Artist",
            CoverThumbPath = thumb,
            CoverFullPath = full,
            Year = a.Year,
            AddedAt = a.AddedAt,
            Tracks = tracks,
        };
    }
}

public sealed class AlbumDto
{
    public string? Name { get; set; }
    public string? Artist { get; set; }
    public string? CoverArt { get; set; }
    public string? CoverArtFull { get; set; }
    public int Year { get; set; }
    public long AddedAt { get; set; }
    /// <summary>The album-artist tag, when the files have one. Tells apart albums that share a name.</summary>
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public string? AlbumArtist { get; set; }
    public List<TrackDto>? Tracks { get; set; }
    /// <summary>Fields this build does not know, kept so rewriting library.json loses nothing.</summary>
    [JsonExtensionData] public Dictionary<string, JsonElement>? Extra { get; set; }
}

public sealed class TrackDto
{
    public string? Id { get; set; }
    public string? Title { get; set; }
    public string? Artist { get; set; }
    public string? Album { get; set; }
    public double Duration { get; set; }
    public string? FilePath { get; set; }
    public string? Src { get; set; }
    /// <summary>Absent on many Electron-written tracks; kept absent when rewriting.</summary>
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public int? TrackNumber { get; set; }
    public int DiscNumber { get; set; }
    public string? Codec { get; set; }
    public int SampleRate { get; set; }
    public int BitsPerSample { get; set; }
    public int Channels { get; set; }
    /// <summary>File modification time (Unix ms) and size when the tags were read; a rescan reads the file again when they differ.</summary>
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public long? Mtime { get; set; }
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public long? Size { get; set; }
    [JsonExtensionData] public Dictionary<string, JsonElement>? Extra { get; set; }
}

// Source-generated serializer: no reflection, so it works with trimming / Native AOT.
[JsonSourceGenerationOptions(PropertyNameCaseInsensitive = true, PropertyNamingPolicy = JsonKnownNamingPolicy.CamelCase,
    NumberHandling = JsonNumberHandling.AllowReadingFromString)]
[JsonSerializable(typeof(Dictionary<string, AlbumDto>))]
internal partial class LibraryJsonContext : JsonSerializerContext;
