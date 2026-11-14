using System.Globalization;

namespace Retrograde.Core;

/// <summary>A field a smart playlist rule can test, with the comparisons that make sense for it.</summary>
public sealed record SmartField(string Key, string Label, IReadOnlyList<string> Ops, string Hint);

/// <summary>Rules of smart playlists: the fields, their comparisons, and matching library tracks against them.</summary>
public static class SmartPlaylists
{
    public static readonly IReadOnlyList<SmartField> Fields =
    [
        new("title", "TITLE", ["contains", "not_contains", "is"], "TEXT"),
        new("artist", "ARTIST", ["contains", "not_contains", "is"], "TEXT"),
        new("album", "ALBUM", ["contains", "not_contains", "is"], "TEXT"),
        new("codec", "FORMAT", ["is", "is_not"], "FLAC, MP3, AAC..."),
        new("bits", "BIT DEPTH", ["gte", "lte", "eq"], "16, 24..."),
        new("rate", "SAMPLE RATE", ["gte", "lte", "eq"], "KHZ: 44.1, 96..."),
        new("year", "YEAR", ["gte", "lte", "eq"], "1999"),
        new("duration", "LENGTH", ["gte", "lte"], "MINUTES"),
        new("plays", "PLAYS", ["gte", "lte", "eq"], "COUNT"),
        new("played", "LAST PLAYED", ["within", "not_within", "never"], "DAYS"),
        new("added", "ADDED", ["within", "not_within"], "DAYS"),
        new("liked", "FAVORITE", ["yes", "no"], ""),
    ];

    public static readonly IReadOnlyDictionary<string, string> OpLabels = new Dictionary<string, string>
    {
        ["contains"] = "CONTAINS", ["not_contains"] = "DOESN'T CONTAIN", ["is"] = "IS", ["is_not"] = "IS NOT",
        ["gte"] = "AT LEAST", ["lte"] = "AT MOST", ["eq"] = "EXACTLY",
        ["within"] = "IN THE LAST", ["not_within"] = "NOT IN THE LAST", ["never"] = "NEVER",
        ["yes"] = "YES", ["no"] = "NO",
    };

    public static readonly IReadOnlyList<(string Key, string Label)> Sorts =
    [
        ("album", "ALBUM ORDER"), ("added_desc", "RECENTLY ADDED"), ("plays_desc", "MOST PLAYED"),
        ("played_desc", "RECENTLY PLAYED"), ("title", "TITLE [A-Z]"), ("random", "RANDOM"),
    ];

    public static SmartField Field(string key) => Fields.FirstOrDefault(f => f.Key == key) ?? Fields[0];

    /// <summary>Whether the comparison takes a typed value (NEVER, YES and NO do not).</summary>
    public static bool NeedsValue(string op) => op is not ("never" or "yes" or "no");

    /// <summary>The examples made on the first start that has smart playlists.</summary>
    public static List<SmartPlaylistData> Examples()
    {
        var id = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
        return
        [
            new() { Id = id, Name = "Hi-Res 24 bit", Rules = [new() { Field = "bits", Op = "gte", Value = "24" }] },
            new() { Id = id + 1, Name = "Added in 30 days", Sort = "added_desc", Rules = [new() { Field = "added", Op = "within", Value = "30" }] },
            new() { Id = id + 2, Name = "On Repeat", Sort = "plays_desc", Limit = 50, Rules = [new() { Field = "plays", Op = "gte", Value = "5" }] },
            new() { Id = id + 3, Name = "Forgotten Favorites", Sort = "random", Rules =
                [new() { Field = "liked", Op = "yes" }, new() { Field = "played", Op = "not_within", Value = "90" }] },
        ];
    }

    /// <summary>The library tracks (with their album) that match the playlist, in its order, cut at its limit.</summary>
    public static List<Track> Evaluate(SmartPlaylistData playlist, IEnumerable<(Track Track, Album Album)> library,
        Func<string, PlayStat?> stats, Func<string, bool> liked, DateTimeOffset now)
    {
        var nowMs = now.ToUnixTimeMilliseconds();
        var rules = playlist.Rules.Where(r => !NeedsValue(r.Op) || r.Value.Trim().Length > 0).ToList();
        var hits = new List<(Track Track, Album Album)>();
        foreach (var e in library)
        {
            if (rules.Count == 0) { hits.Add(e); continue; }
            var ok = playlist.MatchAll
                ? rules.All(r => Matches(r, e.Track, e.Album, stats, liked, nowMs))
                : rules.Any(r => Matches(r, e.Track, e.Album, stats, liked, nowMs));
            if (ok) hits.Add(e);
        }
        IEnumerable<(Track Track, Album Album)> ordered = playlist.Sort switch
        {
            "added_desc" => hits.OrderByDescending(e => e.Album.AddedAt),
            "plays_desc" => hits.OrderByDescending(e => stats(e.Track.Id)?.Count ?? 0).ThenByDescending(e => stats(e.Track.Id)?.Last ?? 0),
            "played_desc" => hits.OrderByDescending(e => stats(e.Track.Id)?.Last ?? 0),
            "title" => hits.OrderBy(e => e.Track.Title, StringComparer.CurrentCultureIgnoreCase),
            // Shuffled, but the same order all day: liking a track or a refresh does not reshuffle the page
            "random" => hits.OrderBy(e => HashCode.Combine(StringComparer.OrdinalIgnoreCase.GetHashCode(e.Track.Id), playlist.Id, now.Date.DayOfYear)),
            _ => hits,
        };
        var list = ordered.Select(e => e.Track);
        if (playlist.Limit > 0) list = list.Take(playlist.Limit);
        return list.ToList();
    }

    private static bool Matches(SmartRule r, Track t, Album a, Func<string, PlayStat?> stats, Func<string, bool> liked, long nowMs)
    {
        var v = r.Value.Trim();
        switch (r.Field)
        {
            case "title": return Text(t.Title, r.Op, v);
            case "artist":
                // The track's own artist or the album's: "contains Adele" finds her duets and her albums
                return r.Op == "not_contains"
                    ? Text(t.Artist, r.Op, v) && Text(a.Artist, r.Op, v)
                    : Text(t.Artist, r.Op, v) || Text(a.Artist, r.Op, v);
            case "album": return Text(a.Name, r.Op, v);
            case "codec":
                var same = string.Equals(Codecs.Short(t.Codec, t.FilePath), Codecs.Short(v, ""), StringComparison.OrdinalIgnoreCase);
                return r.Op == "is_not" ? !same : same;
            case "bits": return Number(t.BitsPerSample, r.Op, v);
            case "rate": return Number(t.SampleRate / 1000.0, r.Op, v, tolerance: 0.05);
            case "year": return Number(a.Year, r.Op, v);
            case "duration": return Number(t.Duration / 60.0, r.Op, v);
            case "plays": return Number(stats(t.Id)?.Count ?? 0, r.Op, v);
            case "played":
                var last = stats(t.Id)?.Last ?? 0;
                return r.Op == "never" ? last == 0 : Within(last, nowMs, r.Op, v);
            case "added": return Within(a.AddedAt, nowMs, r.Op, v);
            case "liked": return liked(t.Id) == (r.Op == "yes");
            default: return false;
        }
    }

    private static bool Text(string field, string op, string v) => op switch
    {
        "contains" => field.Contains(v, StringComparison.CurrentCultureIgnoreCase),
        "not_contains" => !field.Contains(v, StringComparison.CurrentCultureIgnoreCase),
        "is" => string.Equals(field.Trim(), v, StringComparison.CurrentCultureIgnoreCase),
        _ => false,
    };

    private static bool Number(double field, string op, string v, double tolerance = 0.001)
    {
        if (!double.TryParse(v.Replace(',', '.'), NumberStyles.Float, CultureInfo.InvariantCulture, out var n)) return false;
        return op switch
        {
            "gte" => field >= n - tolerance,
            "lte" => field <= n + tolerance,
            "eq" => Math.Abs(field - n) <= tolerance,
            _ => false,
        };
    }

    /// <summary>"In the last N days": the time is set and no older than N days. "Not in": never, or older.</summary>
    private static bool Within(long whenMs, long nowMs, string op, string v)
    {
        if (!double.TryParse(v.Replace(',', '.'), NumberStyles.Float, CultureInfo.InvariantCulture, out var days)) return false;
        var inside = whenMs > 0 && nowMs - whenMs <= days * 86_400_000;
        return op == "within" ? inside : !inside;
    }
}

/// <summary>Short format names (FLAC, MP3, AAC...) from TagLib codec descriptions.</summary>
public static class Codecs
{
    public static string Short(string codec, string filePath)
    {
        if (string.IsNullOrEmpty(codec))
        {
            var e = Path.GetExtension(filePath).TrimStart('.');
            return e.Length > 0 ? e.ToUpperInvariant() : "AUDIO";
        }
        var c = codec.ToUpperInvariant();
        if (c.Contains("MPEG") || c == "MP3") return "MP3";
        if (c.Contains("VORBIS") || c == "OGG") return "OGG";
        if (c.Contains("PCM") || c == "WAV") return "WAV";
        if (c.Contains("WINDOWS MEDIA")) return "WMA";
        if (c.Contains("ALAC") || c.Contains("APPLE LOSSLESS")) return "ALAC";
        if (c.Contains("AAC")) return "AAC";
        if (c.Contains("OPUS")) return "OPUS";
        if (c.Contains("FLAC")) return "FLAC";
        var ext = Path.GetExtension(filePath).TrimStart('.');
        if (ext.Length > 0) return ext.ToUpperInvariant();
        return c.Length > 8 ? c[..8].Trim() : c;
    }
}
