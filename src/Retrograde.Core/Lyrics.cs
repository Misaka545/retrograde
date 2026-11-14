using System.Globalization;
using System.Text;
using System.Text.RegularExpressions;

namespace Retrograde.Core;

/// <summary>A lyrics line; Time is in seconds, or -1 for lyrics without timestamps.</summary>
public sealed record LyricLine(double Time, string Text);

/// <summary>Lyrics of a track: synced (LRC timestamps) or plain text.</summary>
public sealed class LyricsDoc
{
    public required IReadOnlyList<LyricLine> Lines { get; init; }
    public bool IsSynced { get; init; }
    /// <summary>Where they came from: "LRC" (a .lrc file next to the track) or "TAG" (embedded).</summary>
    public string Source { get; init; } = "";

    /// <summary>Index of the line being sung at this time, or -1 before the first.</summary>
    public int IndexAt(double seconds)
    {
        if (!IsSynced) return -1;
        int lo = 0, hi = Lines.Count - 1, found = -1;
        while (lo <= hi)
        {
            var mid = (lo + hi) / 2;
            if (Lines[mid].Time <= seconds) { found = mid; lo = mid + 1; }
            else hi = mid - 1;
        }
        return found;
    }
}

/// <summary>
/// Finds lyrics for a track: a .lrc file with the same name next to it first, then the lyrics tag inside
/// the file. LRC timestamps ([mm:ss.xx], several per line allowed) make them synced; [offset:±ms] is honoured.
/// </summary>
public static partial class LyricsLoader
{
    [GeneratedRegex(@"\[(\d{1,3}):(\d{1,2}(?:[.:]\d{1,3})?)\]")]
    private static partial Regex StampRx();

    [GeneratedRegex(@"^\[(ar|ti|al|au|by|re|ve|length|id|#|offset)\s*:([^\]]*)\]\s*$", RegexOptions.IgnoreCase)]
    private static partial Regex MetaRx();

    public static LyricsDoc? Load(string audioPath)
    {
        try
        {
            var lrc = Path.ChangeExtension(audioPath, ".lrc");
            if (File.Exists(lrc) && Parse(ReadText(lrc), "LRC") is { } fromFile) return fromFile;
            using var file = TagLib.File.Create(audioPath);
            var tag = file.Tag.Lyrics;
            return string.IsNullOrWhiteSpace(tag) ? null : Parse(tag, "TAG");
        }
        catch
        {
            return null; // unreadable file or tag: no lyrics
        }
    }

    /// <summary>LRC files come in UTF-8 (with or without BOM) or UTF-16; anything else is read as UTF-8.</summary>
    private static string ReadText(string path)
    {
        var bytes = File.ReadAllBytes(path);
        if (bytes.Length >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE) return Encoding.Unicode.GetString(bytes, 2, bytes.Length - 2);
        if (bytes.Length >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF) return Encoding.BigEndianUnicode.GetString(bytes, 2, bytes.Length - 2);
        if (bytes.Length >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF) return Encoding.UTF8.GetString(bytes, 3, bytes.Length - 3);
        return Encoding.UTF8.GetString(bytes);
    }

    public static LyricsDoc? Parse(string text, string source)
    {
        var synced = new List<LyricLine>();
        var plain = new List<LyricLine>();
        double offset = 0;
        foreach (var raw in text.Replace("\r\n", "\n").Replace('\r', '\n').Split('\n'))
        {
            var line = raw.Trim();
            var meta = MetaRx().Match(line);
            if (meta.Success)
            {
                if (meta.Groups[1].Value.Equals("offset", StringComparison.OrdinalIgnoreCase)
                    && double.TryParse(meta.Groups[2].Value.Trim(), NumberStyles.Float, CultureInfo.InvariantCulture, out var ms))
                    offset = ms / 1000.0; // positive offset: lyrics come earlier
                continue;
            }
            var stamps = StampRx().Matches(line);
            if (stamps.Count == 0)
            {
                plain.Add(new LyricLine(-1, line));
                continue;
            }
            // Text after the last stamp; inline word stamps (<mm:ss.xx>) of enhanced LRC are dropped
            var body = line[(stamps[^1].Index + stamps[^1].Length)..];
            body = Regex.Replace(body, @"<\d{1,3}:\d{1,2}(?:[.:]\d{1,3})?>", "").Trim();
            foreach (Match m in stamps)
            {
                var min = int.Parse(m.Groups[1].Value, CultureInfo.InvariantCulture);
                var sec = double.Parse(m.Groups[2].Value.Replace(':', '.'), CultureInfo.InvariantCulture);
                synced.Add(new LyricLine(min * 60 + sec, body));
            }
        }
        if (synced.Count > 0)
        {
            var lines = synced
                .Select(l => l with { Time = Math.Max(0, l.Time - offset) })
                .OrderBy(l => l.Time)
                .ToList();
            return new LyricsDoc { Lines = lines, IsSynced = true, Source = source };
        }
        // Plain lyrics: trim blank lines at both ends, keep the stanza breaks inside
        while (plain.Count > 0 && plain[0].Text.Length == 0) plain.RemoveAt(0);
        while (plain.Count > 0 && plain[^1].Text.Length == 0) plain.RemoveAt(plain.Count - 1);
        return plain.Count == 0 ? null : new LyricsDoc { Lines = plain, IsSynced = false, Source = source };
    }
}
