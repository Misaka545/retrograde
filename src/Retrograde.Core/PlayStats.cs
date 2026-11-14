using System.Text.Json;
using System.Text.Json.Serialization;

namespace Retrograde.Core;

/// <summary>How often and when a track was last played (keyed by track id, the file path).</summary>
public sealed class PlayStat
{
    public int Count { get; set; }
    /// <summary>Unix milliseconds of the last counted play.</summary>
    public long Last { get; set; }
}

/// <summary>
/// Play counts and history (native-stats.json). A play counts once a track has been heard for half its
/// length or four minutes, whichever comes first (the usual scrobbling rule), so skipping through
/// tracks does not inflate them.
/// </summary>
public sealed class PlayStats
{
    public Dictionary<string, PlayStat> Tracks { get; set; } = new(StringComparer.OrdinalIgnoreCase);

    private static string FilePath => Path.Combine(CoverPaths.AppDir, "native-stats.json");

    public static PlayStats Load()
    {
        try
        {
            if (File.Exists(FilePath)
                && JsonSerializer.Deserialize(File.ReadAllText(FilePath), StatsJsonContext.Default.PlayStats) is { } stats)
            {
                // The serializer builds a case-sensitive dictionary: ids are paths, compared without case
                stats.Tracks = new Dictionary<string, PlayStat>(stats.Tracks, StringComparer.OrdinalIgnoreCase);
                return stats;
            }
        }
        catch { /* unreadable stats start over; they are not worth an error */ }
        return new PlayStats();
    }

    public PlayStat? Get(string id) => Tracks.GetValueOrDefault(id);

    public void Record(string id, DateTimeOffset when)
    {
        if (!Tracks.TryGetValue(id, out var stat)) Tracks[id] = stat = new PlayStat();
        stat.Count++;
        stat.Last = when.ToUnixTimeMilliseconds();
    }

    /// <summary>Seconds a track must be heard before it counts as played.</summary>
    public static double Threshold(double duration) =>
        duration <= 0 ? 30 : Math.Clamp(duration * 0.5, Math.Min(duration, 15), 240);

    public void Save()
    {
        try
        {
            Directory.CreateDirectory(CoverPaths.AppDir);
            var tmp = FilePath + ".tmp";
            File.WriteAllText(tmp, JsonSerializer.Serialize(this, StatsJsonContext.Default.PlayStats));
            File.Move(tmp, FilePath, overwrite: true);
        }
        catch { /* best-effort */ }
    }
}

[JsonSerializable(typeof(PlayStats))]
internal partial class StatsJsonContext : JsonSerializerContext;
