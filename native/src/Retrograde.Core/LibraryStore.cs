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
        await using var fs = new FileStream(LibraryPath, FileMode.Open, FileAccess.Read, FileShare.ReadWrite, 1 << 16, useAsync: true);
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
        if (File.Exists(LibraryPath)) File.Replace(tmp, LibraryPath, LibraryPath + ".bak", ignoreMetadataErrors: true);
        else File.Move(tmp, LibraryPath);
    }

    /// <summary>DELETE_ALBUM / batch delete: drops the albums from library.json and removes their cover files.</summary>
    public static async Task<int> DeleteAlbumsAsync(IReadOnlyCollection<string> names, CancellationToken ct = default)
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
    [JsonExtensionData] public Dictionary<string, JsonElement>? Extra { get; set; }
}

// Source-generated serializer: no reflection, so it works with trimming / Native AOT.
[JsonSourceGenerationOptions(PropertyNameCaseInsensitive = true, PropertyNamingPolicy = JsonKnownNamingPolicy.CamelCase,
    NumberHandling = JsonNumberHandling.AllowReadingFromString)]
[JsonSerializable(typeof(Dictionary<string, AlbumDto>))]
internal partial class LibraryJsonContext : JsonSerializerContext;
