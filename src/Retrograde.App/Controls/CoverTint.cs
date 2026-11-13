using Avalonia;
using Avalonia.Animation;
using Avalonia.Controls;
using Avalonia.Media;
using Avalonia.Threading;

namespace Retrograde.App.Controls;

/// <summary>
/// Spotify-style album page background: the cover's colour behind the header, deepening towards its
/// bottom, then a darker band from the action row that fades into the page over <see cref="FadeHeight"/>.
/// Laid out over the whole header and stretched <see cref="FadeHeight"/> below it with a negative
/// bottom margin, so the fade runs behind the first track rows. The colour is picked off the UI thread
/// once per cover (cached) and fades in.
/// </summary>
public sealed class CoverTint : Control
{
    public static readonly StyledProperty<string?> PathProperty =
        AvaloniaProperty.Register<CoverTint, string?>(nameof(Path));

    /// <summary>Colour used without a cover (or until one is picked), e.g. Liked Songs.</summary>
    public static readonly StyledProperty<Color?> FixedColorProperty =
        AvaloniaProperty.Register<CoverTint, Color?>(nameof(FixedColor));

    /// <summary>Height of the action row at the bottom of the header, where the darker band starts.</summary>
    public static readonly StyledProperty<double> ActionHeightProperty =
        AvaloniaProperty.Register<CoverTint, double>(nameof(ActionHeight), 57);

    public static readonly StyledProperty<double> FadeHeightProperty =
        AvaloniaProperty.Register<CoverTint, double>(nameof(FadeHeight), 232);

    /// <summary>The page colour the fade ends in.</summary>
    public static readonly StyledProperty<Color> BaseColorProperty =
        AvaloniaProperty.Register<CoverTint, Color>(nameof(BaseColor), Color.Parse("#121212"));

    private static readonly Dictionary<string, Color> Cache = new(StringComparer.OrdinalIgnoreCase);
    private static readonly Color NoCover = Color.Parse("#404044");
    private Color? _color;
    private int _version;

    static CoverTint()
    {
        AffectsRender<CoverTint>(PathProperty, FixedColorProperty, ActionHeightProperty, FadeHeightProperty, BaseColorProperty);
        IsHitTestVisibleProperty.OverrideDefaultValue<CoverTint>(false);
    }

    public CoverTint()
    {
        Transitions = [new DoubleTransition { Property = OpacityProperty, Duration = TimeSpan.FromMilliseconds(250) }];
    }

    public string? Path { get => GetValue(PathProperty); set => SetValue(PathProperty, value); }
    public Color? FixedColor { get => GetValue(FixedColorProperty); set => SetValue(FixedColorProperty, value); }
    public double ActionHeight { get => GetValue(ActionHeightProperty); set => SetValue(ActionHeightProperty, value); }
    public double FadeHeight { get => GetValue(FadeHeightProperty); set => SetValue(FadeHeightProperty, value); }
    public Color BaseColor { get => GetValue(BaseColorProperty); set => SetValue(BaseColorProperty, value); }

    protected override void OnPropertyChanged(AvaloniaPropertyChangedEventArgs change)
    {
        base.OnPropertyChanged(change);
        if (change.Property == PathProperty || change.Property == FixedColorProperty) Reload();
    }

    protected override void OnAttachedToVisualTree(VisualTreeAttachmentEventArgs e)
    {
        base.OnAttachedToVisualTree(e);
        Reload();
    }

    private void Reload()
    {
        var version = ++_version;
        if (FixedColor is { } fixedColor) { Show(fixedColor, animate: false); return; }
        var path = Path;
        if (string.IsNullOrEmpty(path)) { Show(NoCover, animate: false); return; }
        lock (Cache)
            if (Cache.TryGetValue(path, out var cached)) { Show(cached, animate: false); return; }

        _color = null;
        SetCurrentValue(OpacityProperty, 0d);
        Task.Run(() =>
        {
            var color = Pick(path);
            lock (Cache) Cache[path] = color;
            Dispatcher.UIThread.Post(() => { if (version == _version) Show(color, animate: true); });
        });
    }

    /// <summary>
    /// A mid-dark tone of the cover's main colour, as on Spotify: saturated enough to read as the
    /// cover's colour, dark enough for white text.
    /// </summary>
    private static Color Pick(string path)
    {
        try
        {
            using var src = CoverColors.DecodeSmall(path);
            if (src is null) return NoCover;
            var c = CoverColors.Dominant(src, out var vivid);
            var tone = vivid ? CoverColors.Clamp(c, 35, 70, 38, 52) : CoverColors.Clamp(c, 0, 12, 24, 34);
            // Same HSV brightness looks much lighter in yellow and green than in blue or red: cap the
            // perceived brightness so every cover gets an equally deep header.
            var luma = 0.2126 * tone.R + 0.7152 * tone.G + 0.0722 * tone.B;
            const double maxLuma = 78;
            if (luma <= maxLuma) return tone;
            var k = maxLuma / luma;
            return Color.FromRgb((byte)(tone.R * k), (byte)(tone.G * k), (byte)(tone.B * k));
        }
        catch
        {
            return NoCover;
        }
    }

    private void Show(Color color, bool animate)
    {
        _color = color;
        if (!animate)
        {
            // Jump straight to visible: no fade when the colour is known already
            var transitions = Transitions;
            Transitions = null;
            SetCurrentValue(OpacityProperty, 1d);
            Transitions = transitions;
        }
        else SetCurrentValue(OpacityProperty, 1d);
        InvalidateVisual();
    }

    private static Color Mix(Color a, Color b, double t) => Color.FromRgb(
        (byte)(a.R + (b.R - a.R) * t), (byte)(a.G + (b.G - a.G) * t), (byte)(a.B + (b.B - a.B) * t));

    public override void Render(DrawingContext context)
    {
        if (_color is not { } c) return;
        var h = Bounds.Height;
        if (h <= 0 || Bounds.Width <= 0) return;
        var baseColor = BaseColor;
        var black = Colors.Black;
        // y where the header body ends and the action row starts
        var split = Math.Max(0, h - FadeHeight - ActionHeight);
        double At(double y) => Math.Clamp(y / h, 0, 1);

        // One continuous fall-off, no step at the action row: the colour holds through the top of the
        // header, darkens through its lower part and keeps fading, eased, into the page below.
        var brush = new LinearGradientBrush
        {
            StartPoint = new RelativePoint(0, 0, RelativeUnit.Relative),
            EndPoint = new RelativePoint(0, 1, RelativeUnit.Relative),
        };
        brush.GradientStops.Add(new GradientStop(c, 0));
        brush.GradientStops.Add(new GradientStop(Mix(c, black, 0.06), At(split * 0.45)));
        // From 45% of the header to the end: smoothstep from the colour to the page, shaded a little on
        // the way so the middle does not turn muddy
        var from = split * 0.45;
        const int steps = 10;
        for (int i = 1; i <= steps; i++)
        {
            var t = i / (double)steps;
            var e = t * t * (3 - 2 * t);
            var shade = Mix(c, black, 0.06 + 0.34 * Math.Sin(t * Math.PI));
            brush.GradientStops.Add(new GradientStop(Mix(shade, baseColor, e), At(from + (h - from) * t)));
        }
        context.FillRectangle(brush, new Rect(Bounds.Size));
    }
}
