using Avalonia;
using Avalonia.Media;
using Avalonia.Threading;
using Retrograde.App.Controls;

namespace Retrograde.App.Services;

/// <summary>
/// The highlight colour of what is playing (seek strip, playing row, sung lyric line). With
/// "accent from cover" on it is the vivid colour of the playing track's cover, brightened to read on the
/// dark panels (a grey cover gives a silver of its tone); otherwise the Orbit orange. Changes fade over 400 ms.
/// The brushes are shared resources (NowAccent, NowAccentSoft): styles use them, this changes their colour.
/// </summary>
public static class AccentTheme
{
    public static readonly Color Default = Color.Parse("#FF6B35");

    private static SolidColorBrush? _brush;
    private static SolidColorBrush? _soft;
    private static bool _enabled;
    private static string? _coverPath;
    private static int _generation;
    private static DispatcherTimer? _fade;
    private static Color _from, _to;
    private static long _fadeStart;

    /// <summary>True while the colour comes from a cover (the seek strip then drops its orange-gold-teal gradient).</summary>
    public static bool FromCover { get; private set; }

    /// <summary>The colour now on screen (mid-fade too).</summary>
    public static Color Current { get; private set; } = Default;

    /// <summary>Raised on the UI thread whenever <see cref="Current"/> changes (custom-drawn controls repaint).</summary>
    public static event Action? Changed;

    public static void Init(bool enabled)
    {
        _brush = Resource("NowAccent") as SolidColorBrush;
        _soft = Resource("NowAccentSoft") as SolidColorBrush;
        _enabled = enabled;
    }

    private static object? Resource(string key) =>
        Application.Current is { } app && app.TryGetResource(key, null, out var value) ? value : null;

    public static void SetEnabled(bool enabled)
    {
        if (_enabled == enabled) return;
        _enabled = enabled;
        _ = FollowAsync(_coverPath, force: true);
    }

    /// <summary>The playing track changed: take the colour of its cover.</summary>
    public static void Follow(string? coverPath) => _ = FollowAsync(coverPath, force: false);

    private static async Task FollowAsync(string? coverPath, bool force)
    {
        if (!force && string.Equals(coverPath, _coverPath, StringComparison.OrdinalIgnoreCase)) return;
        _coverPath = coverPath;
        var gen = ++_generation;
        var target = Default;
        if (_enabled && coverPath is not null)
        {
            target = await Task.Run(() =>
            {
                try
                {
                    using var bmp = CoverColors.DecodeSmall(coverPath);
                    if (bmp is null) return Default;
                    // A small coloured detail is enough (pale covers: a pink sky, a blue logo). Vivid colours are
                    // lifted to read on #121212; a cover with no colour at all gives a silver of its own tone.
                    var c = CoverColors.Dominant(bmp, out var vivid, minShare: 0.002);
                    return vivid ? CoverColors.Clamp(c, 45, 85, 88, 100) : CoverColors.Clamp(c, 0, 18, 80, 88);
                }
                catch { return Default; }
            });
        }
        if (gen != _generation) return; // a newer track came in while decoding
        FromCover = _enabled && coverPath is not null && target != Default;
        FadeTo(target);
    }

    private static void FadeTo(Color target)
    {
        if (target == Current && _fade is null) return;
        _from = Current;
        _to = target;
        _fadeStart = Environment.TickCount64;
        if (_fade is null)
        {
            _fade = new DispatcherTimer(TimeSpan.FromMilliseconds(16), DispatcherPriority.Render, (_, _) => Step());
            _fade.Start();
        }
    }

    private static void Step()
    {
        var p = Math.Clamp((Environment.TickCount64 - _fadeStart) / 400.0, 0, 1);
        var e = 1 - Math.Pow(1 - p, 3); // ease out
        Apply(Lerp(_from, _to, e));
        if (p >= 1)
        {
            _fade?.Stop();
            _fade = null;
        }
    }

    private static void Apply(Color c)
    {
        Current = c;
        if (_brush is not null) _brush.Color = c;
        if (_soft is not null) _soft.Color = Color.FromArgb(0x1A, c.R, c.G, c.B);
        Changed?.Invoke();
    }

    public static Color Lerp(Color a, Color b, double t) => Color.FromArgb(
        (byte)(a.A + (b.A - a.A) * t), (byte)(a.R + (b.R - a.R) * t),
        (byte)(a.G + (b.G - a.G) * t), (byte)(a.B + (b.B - a.B) * t));
}
