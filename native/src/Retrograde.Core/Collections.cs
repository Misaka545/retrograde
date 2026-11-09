using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace Retrograde.Core;

public sealed class PlaylistData
{
    public long Id { get; set; }
    public string Name { get; set; } = "";
    public string? CoverArt { get; set; }
    public List<string> TrackIds { get; set; } = [];
}

/// <summary>Liked songs and playlists. Tracks are referenced by id (= file path), as in the Electron build.</summary>
public sealed class CollectionsData
{
    public List<string> Liked { get; set; } = [];
    public List<PlaylistData> Playlists { get; set; } = [];
    /// <summary>Set when this file was seeded from the Electron build's localStorage.</summary>
    public string? ImportedFrom { get; set; }
}

public static class CollectionsStore
{
    public static string FilePath => Path.Combine(CoverPaths.AppDir, "native-collections.json");

    /// <summary>Loads native-collections.json; on first run, imports the Electron playlists / liked songs.</summary>
    public static CollectionsData Load()
    {
        try
        {
            if (File.Exists(FilePath))
                return JsonSerializer.Deserialize(File.ReadAllText(FilePath), CollectionsJsonContext.Default.CollectionsData) ?? new();
        }
        catch { /* corrupt file: start empty rather than crash */ }

        var imported = ElectronImport.TryRead();
        if (imported is not null) Save(imported);
        return imported ?? new CollectionsData();
    }

    public static void Save(CollectionsData data)
    {
        try
        {
            Directory.CreateDirectory(CoverPaths.AppDir);
            var tmp = FilePath + ".tmp";
            File.WriteAllText(tmp, JsonSerializer.Serialize(data, WriteContext.CollectionsData));
            File.Move(tmp, FilePath, overwrite: true);
        }
        catch { /* best effort */ }
    }

    private static readonly CollectionsJsonContext WriteContext = new(new JsonSerializerOptions
    {
        WriteIndented = true,
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping,
    });
}

/// <summary>Reads 'my_playlists' and 'liked_songs' from the Electron build's localStorage (LevelDB).</summary>
public static class ElectronImport
{
    public static string? FindLocalStorageDir()
    {
        var appData = Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData);
        foreach (var name in new[] { "retrograde", "Retrograde" })
        {
            var dir = Path.Combine(appData, name, "Local Storage", "leveldb");
            if (Directory.Exists(dir)) return dir;
        }
        return null;
    }

    public static CollectionsData? TryRead()
    {
        try
        {
            var dir = FindLocalStorageDir();
            if (dir is null) return null;
            string? playlistsJson = null, likedJson = null;
            foreach (var (k64, raw) in LevelDbReader.ReadAll(dir))
            {
                var key = Encoding.Latin1.GetString(Convert.FromBase64String(k64));
                // Chromium key: "_<origin>\0\x01<name>"; value: 0x00 + UTF-16LE or 0x01 + Latin-1.
                if (!key.StartsWith('_')) continue;
                if (key.EndsWith("\u0001my_playlists", StringComparison.Ordinal)) playlistsJson = DecodeValue(raw);
                else if (key.EndsWith("\u0001liked_songs", StringComparison.Ordinal)) likedJson = DecodeValue(raw);
            }
            if (playlistsJson is null && likedJson is null) return null;

            var data = new CollectionsData { ImportedFrom = dir };
            if (likedJson is not null)
            {
                using var doc = JsonDocument.Parse(likedJson);
                foreach (var t in doc.RootElement.EnumerateArray())
                    if (TrackId(t) is { } id && !data.Liked.Contains(id)) data.Liked.Add(id);
            }
            if (playlistsJson is not null)
            {
                using var doc = JsonDocument.Parse(playlistsJson);
                foreach (var pl in doc.RootElement.EnumerateArray())
                {
                    var p = new PlaylistData
                    {
                        Id = pl.TryGetProperty("id", out var id) && id.ValueKind == JsonValueKind.Number ? id.GetInt64() : DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(),
                        Name = pl.TryGetProperty("name", out var n) ? n.GetString() ?? "" : "",
                        CoverArt = pl.TryGetProperty("coverArt", out var c) && c.ValueKind == JsonValueKind.String && c.GetString()!.StartsWith("file:", StringComparison.OrdinalIgnoreCase) ? c.GetString() : null,
                    };
                    if (pl.TryGetProperty("tracks", out var tracks) && tracks.ValueKind == JsonValueKind.Array)
                        foreach (var t in tracks.EnumerateArray())
                            if (TrackId(t) is { } tid && !p.TrackIds.Contains(tid)) p.TrackIds.Add(tid);
                    data.Playlists.Add(p);
                }
            }
            return data;
        }
        catch
        {
            return null;
        }
    }

    private static string? TrackId(JsonElement t)
    {
        if (t.ValueKind != JsonValueKind.Object) return null;
        // filePath first: some Electron entries carry ids with a timestamp suffix (path + Date.now() + random).
        if (t.TryGetProperty("filePath", out var fp) && fp.ValueKind == JsonValueKind.String) return fp.GetString();
        if (t.TryGetProperty("id", out var id) && id.ValueKind == JsonValueKind.String) return id.GetString();
        return null;
    }

    private static string DecodeValue(byte[] raw)
    {
        if (raw.Length == 0) return "";
        return raw[0] == 0 ? Encoding.Unicode.GetString(raw, 1, raw.Length - 1) : Encoding.Latin1.GetString(raw, 1, raw.Length - 1);
    }
}

[JsonSourceGenerationOptions(PropertyNameCaseInsensitive = true, PropertyNamingPolicy = JsonKnownNamingPolicy.CamelCase)]
[JsonSerializable(typeof(CollectionsData))]
internal partial class CollectionsJsonContext : JsonSerializerContext;
