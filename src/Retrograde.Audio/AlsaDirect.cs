using System.Text.RegularExpressions;

namespace Retrograde.Audio;

/// <summary>A sound card output opened straight through ALSA (hw:), bypassing PipeWire's mixer: Card is the kernel's card number.</summary>
public sealed record DirectDevice(string Name, string Description, int Card);

/// <summary>What the kernel says an open ALSA device is really running at (/proc/asound/cardN/pcmXp/sub0/hw_params).</summary>
public readonly record struct AlsaHwParams(string Format, int Rate, int Channels);

/// <summary>
/// Direct ALSA output for bit-perfect playback: finds the sound cards, names them the way mpv takes them
/// (alsa/hw:CARD=id,DEV=n) and reads back what the driver is running at. Everything comes from /proc/asound,
/// so no ALSA library is needed and listing the devices opens none of them.
/// </summary>
public static partial class AlsaDirect
{
    /// <summary>The mpv --audio-device prefix of a plain hw: device (no plug conversion layer).</summary>
    public const string Prefix = "alsa/hw:";

    public static bool IsDirect(string? device) => device is not null && device.StartsWith(Prefix, StringComparison.Ordinal);

    [GeneratedRegex(@"^\s*(\d+)\s+\[(\S+)\s*\]:\s*(.*?)\s+-\s+(.*?)\s*$")]
    private static partial Regex CardLine();

    [GeneratedRegex(@"CARD=([^,]+)")]
    private static partial Regex CardId();

    [GeneratedRegex(@"^hw:(\d+)")]
    private static partial Regex CardNumber();

    private sealed record Card(int Index, string Id, string Name);

    private static List<Card> ReadCards()
    {
        var cards = new List<Card>();
        try
        {
            foreach (var line in File.ReadLines("/proc/asound/cards"))
                if (CardLine().Match(line) is { Success: true } m)
                    cards.Add(new Card(int.Parse(m.Groups[1].Value), m.Groups[2].Value, m.Groups[4].Value));
        }
        catch { /* no ALSA: no direct devices */ }
        return cards;
    }

    /// <summary>Playback outputs that can be opened directly. HDMI outputs are listed only on cards with a monitor attached.</summary>
    public static IReadOnlyList<DirectDevice> List()
    {
        var list = new List<DirectDevice>();
        foreach (var card in ReadCards())
        {
            var dir = $"/proc/asound/card{card.Index}";
            bool? monitor = null;
            try
            {
                foreach (var pcm in Directory.GetDirectories(dir, "pcm*p").OrderBy(p => p, StringComparer.Ordinal))
                {
                    var devText = Path.GetFileName(pcm)[3..^1];
                    if (!int.TryParse(devText, out var dev)) continue;
                    var pcmName = ReadField(Path.Combine(pcm, "info"), "name") ?? $"Device {dev}";
                    if (pcmName.StartsWith("HDMI", StringComparison.OrdinalIgnoreCase) && !(monitor ??= HasMonitor(dir))) continue;
                    list.Add(new DirectDevice($"{Prefix}CARD={card.Id},DEV={dev}", $"{card.Name}, {pcmName}", card.Index));
                }
            }
            catch { /* a card that vanished while listing */ }
        }
        // Sound cards that are not HDMI first
        return list.OrderBy(d => d.Description.Contains("HDMI", StringComparison.OrdinalIgnoreCase) ? 1 : 0).ToList();
    }

    private static bool HasMonitor(string cardDir)
    {
        try
        {
            foreach (var eld in Directory.GetFiles(cardDir, "eld#*"))
                if (File.ReadLines(eld).Any(l => l.StartsWith("monitor_present", StringComparison.Ordinal) && l.TrimEnd().EndsWith('1')))
                    return true;
        }
        catch { }
        return false;
    }

    private static string? ReadField(string file, string field)
    {
        try
        {
            foreach (var line in File.ReadLines(file))
                if (line.StartsWith(field + ":", StringComparison.Ordinal)) return line[(field.Length + 1)..].Trim();
        }
        catch { }
        return null;
    }

    /// <summary>The card number behind an mpv device name (alsa/hw:CARD=id,DEV=n or alsa/hw:N,n); null if there is none.</summary>
    public static int? CardOf(string device)
    {
        if (!IsDirect(device)) return null;
        var name = device["alsa/".Length..];
        if (CardNumber().Match(name) is { Success: true } n) return int.Parse(n.Groups[1].Value);
        if (CardId().Match(name) is { Success: true } m)
            return ReadCards().FirstOrDefault(c => c.Id == m.Groups[1].Value)?.Index;
        return null;
    }

    [GeneratedRegex(@"DEV=(\d+)")]
    private static partial Regex DevNumber();

    [GeneratedRegex(@"^hw:\d+,(\d+)")]
    private static partial Regex NumberedDev();

    /// <summary>The playback device number of an mpv device name (DEV=n or hw:card,n); 0 when none is named.</summary>
    private static int DeviceNumber(string device)
    {
        var name = device["alsa/".Length..];
        if (DevNumber().Match(name) is { Success: true } m) return int.Parse(m.Groups[1].Value);
        if (NumberedDev().Match(name) is { Success: true } n) return int.Parse(n.Groups[1].Value);
        return 0;
    }

    /// <summary>
    /// What the driver runs the given direct device at, null when its playback stream is not open. This is the
    /// kernel's own answer, so it shows resampling or format conversion that happened anywhere before the hardware.
    /// </summary>
    public static AlsaHwParams? ReadHwParams(string device)
    {
        if (CardOf(device) is not { } card) return null;
        try
        {
            var text = File.ReadAllText($"/proc/asound/card{card}/pcm{DeviceNumber(device)}p/sub0/hw_params");
            if (text.StartsWith("closed", StringComparison.Ordinal)) return null;
            var format = ReadValue(text, "format");
            var rate = ReadValue(text, "rate")?.Split(' ')[0];
            var channels = ReadValue(text, "channels");
            if (format is not null && int.TryParse(rate, out var r) && int.TryParse(channels, out var c))
                return new AlsaHwParams(format, r, c);
        }
        catch { }
        return null;
    }

    private static string? ReadValue(string text, string key)
    {
        foreach (var line in text.Split('\n'))
            if (line.StartsWith(key + ":", StringComparison.Ordinal)) return line[(key.Length + 1)..].Trim();
        return null;
    }
}
