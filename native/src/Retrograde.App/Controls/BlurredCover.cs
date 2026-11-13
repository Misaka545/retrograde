using Avalonia;
using Avalonia.Controls;
using Avalonia.Media;
using Avalonia.Media.Imaging;
using Avalonia.Threading;
using SkiaSharp;

namespace Retrograde.App.Controls;

/// <summary>
/// Full-screen backdrop coloured by the cover. Off the UI thread, once per track, the cover is
/// blurred into a small bitmap (the Electron build uses CSS blur(120px)) and its dominant colour
/// is picked. The backdrop is then a gradient in that colour, the blurred cover over it, a glow of
/// the colour behind the artwork and a shade that keeps the text readable. A new track cross-fades
/// in over half a second. Stretching a 96px bitmap and a few gradients costs little per frame,
/// unlike a live blur over the whole window in the software renderer.
/// </summary>
public sealed class BlurredCover : Control
{
    public static readonly StyledProperty<string?> PathProperty =
        AvaloniaProperty.Register<BlurredCover, string?>(nameof(Path));

    /// <summary>The colour picked from the current cover (for accents elsewhere).</summary>
    public static readonly StyledProperty<Color> AccentProperty =
        AvaloniaProperty.Register<BlurredCover, Color>(nameof(Accent), Color.Parse("#FF6B35"));

    private sealed record Layer(Bitmap? Image, Color Accent) : IDisposable
    {
        public void Dispose() => Image?.Dispose();
    }

    private Layer? _current, _previous;
    private double _fade = 1;               // 0 → 1 while the new layer fades in
    private DispatcherTimer? _fadeTimer;
    private long _fadeStart;
    private int _version;
    private const double FadeMs = 500;

    static BlurredCover() => AffectsRender<BlurredCover>(PathProperty);

    public string? Path
    {
        get => GetValue(PathProperty);
        set => SetValue(PathProperty, value);
    }

    public Color Accent
    {
        get => GetValue(AccentProperty);
        set => SetValue(AccentProperty, value);
    }

    protected override void OnPropertyChanged(AvaloniaPropertyChangedEventArgs change)
    {
        base.OnPropertyChanged(change);
        if (change.Property == PathProperty) Reload();
    }

    private void Reload()
    {
        var version = ++_version;
        var path = Path;
        if (string.IsNullOrEmpty(path) || !File.Exists(path))
        {
            Show(null);
            return;
        }
        _ = Task.Run(() => Build(path)).ContinueWith(t =>
            Dispatcher.UIThread.Post(() =>
            {
                if (version == _version) Show(t.Result);
                else t.Result?.Dispose();
            }),
            TaskScheduler.Default);
    }

    // The composed backdrop of the current layer, kept as one bitmap at the control's size: while
    // nothing changes (most of the time, and during the reveal animation) a frame is one image draw
    // instead of a blurred cover plus three full-window gradients.
    private RenderTargetBitmap? _cache;
    private Layer? _cacheLayer;
    private Size _cacheSize;
    private double _cacheScale;

    private void DropCache()
    {
        _cache?.Dispose();
        _cache = null;
        _cacheLayer = null;
    }

    protected override void OnDetachedFromVisualTree(VisualTreeAttachmentEventArgs e)
    {
        base.OnDetachedFromVisualTree(e);
        DropCache();
    }

    private void Show(Layer? layer)
    {
        DropCache();
        _previous?.Dispose();
        _previous = _current;
        _current = layer;
        if (layer is not null) Accent = layer.Accent;
        if (!IsEffectivelyVisible)
        {
            // The player is closed: nobody sees a fade, so switch at once. The next opening then finds
            // the new backdrop settled instead of a timer invalidating a hidden control 30 times.
            _previous?.Dispose();
            _previous = null;
            _fade = 1;
            _fadeTimer?.Stop();
            InvalidateVisual();
            return;
        }
        // Cross-fade: a short, one-off animation (not a loop), so a plain 60 fps timer is fine.
        _fade = _previous is null && layer is null ? 1 : 0;
        _fadeStart = Environment.TickCount64;
        _fadeTimer ??= new DispatcherTimer(TimeSpan.FromMilliseconds(16), DispatcherPriority.Render, (_, _) => StepFade());
        _fadeTimer.Start();
        InvalidateVisual();
    }

    private void StepFade()
    {
        _fade = Math.Clamp((Environment.TickCount64 - _fadeStart) / FadeMs, 0, 1);
        if (_fade >= 1)
        {
            _fadeTimer?.Stop();
            _previous?.Dispose();
            _previous = null;
        }
        InvalidateVisual();
    }

    // ---------- Off-thread: blur + dominant colour ----------

    private static Layer? Build(string path)
    {
        try
        {
            using var src = SKBitmap.Decode(path);
            if (src is null) return null;
            var accent = DominantColor(src);

            const int size = 96;
            using var small = src.Resize(new SKImageInfo(size, size), new SKSamplingOptions(SKFilterMode.Linear, SKMipmapMode.Linear));
            if (small is null) return new Layer(null, accent);
            using var surface = SKSurface.Create(new SKImageInfo(size, size, SKColorType.Bgra8888, SKAlphaType.Premul));
            using var paint = new SKPaint { ImageFilter = SKImageFilter.CreateBlur(8, 8, SKShaderTileMode.Clamp) };
            surface.Canvas.DrawBitmap(small, 0, 0, paint);
            using var image = surface.Snapshot();
            using var data = image.Encode(SKEncodedImageFormat.Png, 100);
            using var ms = new MemoryStream(data.ToArray());
            return new Layer(new Bitmap(ms), accent);
        }
        catch
        {
            return null;
        }
    }

    /// <summary>
    /// The most prominent vivid colour: pixels are grouped by hue (24 bins), each weighted by its
    /// saturation and brightness, and the heaviest bin's average colour wins. Near-black, near-white
    /// and grey pixels count for little, so a dark cover with a red logo comes out red, not black.
    /// Grey-only covers fall back to their average tone. The result is brightened so it reads as a glow.
    /// </summary>
    private static Color DominantColor(SKBitmap src)
    {
        using var px = src.Resize(new SKImageInfo(48, 48), new SKSamplingOptions(SKFilterMode.Linear)) ?? src;
        const int bins = 24;
        var weight = new double[bins];
        var r = new double[bins];
        var g = new double[bins];
        var b = new double[bins];
        double ar = 0, ag = 0, ab = 0;
        int n = 0;
        for (int y = 0; y < px.Height; y++)
        for (int x = 0; x < px.Width; x++)
        {
            var c = px.GetPixel(x, y);
            ar += c.Red; ag += c.Green; ab += c.Blue; n++;
            c.ToHsv(out var h, out var s, out var v);
            s /= 100f; v /= 100f;
            if (v < 0.12 || s < 0.15) continue;
            var w = s * s * v;              // favour vivid over pale
            var i = (int)(h / 360f * bins) % bins;
            weight[i] += w;
            r[i] += c.Red * w; g[i] += c.Green * w; b[i] += c.Blue * w;
        }
        var best = 0;
        for (int i = 1; i < bins; i++) if (weight[i] > weight[best]) best = i;
        SKColor pick;
        if (weight[best] > n * 0.01)
        {
            pick = new SKColor((byte)(r[best] / weight[best]), (byte)(g[best] / weight[best]), (byte)(b[best] / weight[best]));
            pick.ToHsv(out var h, out var s, out var v);
            pick = SKColor.FromHsv(h, Math.Clamp(s, 45, 85), Math.Clamp(v, 60, 85));
        }
        else
        {
            pick = new SKColor((byte)(ar / n), (byte)(ag / n), (byte)(ab / n));
            pick.ToHsv(out var h, out var s, out var v);
            pick = SKColor.FromHsv(h, Math.Min(s, 20), Math.Clamp(v, 35, 55));
        }
        return Color.FromRgb(pick.Red, pick.Green, pick.Blue);
    }

    // ---------- Drawing ----------

    public override void Render(DrawingContext context)
    {
        var bounds = new Rect(Bounds.Size);
        if (_current is not null && (_previous is null || _fade >= 1) && bounds.Width > 0 && bounds.Height > 0)
        {
            var scale = TopLevel.GetTopLevel(this)?.RenderScaling ?? 1;
            if (_cache is null || !ReferenceEquals(_cacheLayer, _current) || _cacheSize != bounds.Size || _cacheScale != scale)
            {
                _cache?.Dispose();
                _cache = new RenderTargetBitmap(
                    new PixelSize((int)Math.Ceiling(bounds.Width * scale), (int)Math.Ceiling(bounds.Height * scale)),
                    new Vector(96 * scale, 96 * scale));
                using (var dc = _cache.CreateDrawingContext()) DrawLayer(dc, bounds, _current, 1);
                _cacheLayer = _current;
                _cacheSize = bounds.Size;
                _cacheScale = scale;
            }
            // From its pixel rectangle: the source rectangle is in pixels, and the Size of a bitmap whose
            // DPI is not 96 is smaller, which would draw a zoomed-in corner of the backdrop.
            context.DrawImage(_cache, new Rect(0, 0, _cache.PixelSize.Width, _cache.PixelSize.Height), bounds);
            return;
        }
        if (_previous is not null && _fade < 1) DrawLayer(context, bounds, _previous, 1);
        if (_current is not null) DrawLayer(context, bounds, _current, _previous is null ? 1 : _fade);
        else if (_previous is not null) DrawLayer(context, bounds, _previous, 1 - _fade);
    }

    private static Color Mix(Color a, Color b, double t) => Color.FromArgb(
        (byte)(a.A + (b.A - a.A) * t), (byte)(a.R + (b.R - a.R) * t), (byte)(a.G + (b.G - a.G) * t), (byte)(a.B + (b.B - a.B) * t));

    private static Color WithAlpha(Color c, byte a) => Color.FromArgb(a, c.R, c.G, c.B);

    private static void DrawLayer(DrawingContext context, Rect b, Layer layer, double opacity)
    {
        if (opacity <= 0) return;
        using var _ = context.PushOpacity(opacity);
        var accent = layer.Accent;
        var near = Color.FromRgb(0x0B, 0x0B, 0x0D);

        // 1. Base: the cover colour, deep at the top left, fading to near black at the bottom right.
        context.FillRectangle(new LinearGradientBrush
        {
            StartPoint = new RelativePoint(0, 0, RelativeUnit.Relative),
            EndPoint = new RelativePoint(1, 1, RelativeUnit.Relative),
            GradientStops =
            {
                new GradientStop(Mix(accent, near, 0.30), 0),
                new GradientStop(Mix(accent, near, 0.68), 0.55),
                new GradientStop(near, 1),
            },
        }, b);

        // 2. The blurred cover, object-cover at scale(1.5), centred.
        if (layer.Image is { } img)
        {
            var side = Math.Max(b.Width, b.Height) * 1.5;
            var dest = new Rect((b.Width - side) / 2, (b.Height - side) / 2, side, side);
            using (context.PushOpacity(0.40)) context.DrawImage(img, new Rect(img.Size), dest);
        }

        // 3. A glow of the colour behind the artwork (left half).
        context.FillRectangle(new RadialGradientBrush
        {
            Center = new RelativePoint(0.27, 0.48, RelativeUnit.Relative),
            GradientOrigin = new RelativePoint(0.27, 0.48, RelativeUnit.Relative),
            RadiusX = new RelativeScalar(0.55, RelativeUnit.Relative),
            RadiusY = new RelativeScalar(0.75, RelativeUnit.Relative),
            GradientStops =
            {
                new GradientStop(WithAlpha(accent, 0x80), 0),
                new GradientStop(WithAlpha(accent, 0x00), 1),
            },
        }, b);

        // 4. Shade: lighter at the top, darker towards the queue and the bottom edge, for readable text.
        context.FillRectangle(new LinearGradientBrush
        {
            StartPoint = new RelativePoint(0, 0, RelativeUnit.Relative),
            EndPoint = new RelativePoint(0, 1, RelativeUnit.Relative),
            GradientStops =
            {
                new GradientStop(Color.FromArgb(0x1A, 0, 0, 0), 0),
                new GradientStop(Color.FromArgb(0x59, 0, 0, 0), 0.6),
                new GradientStop(Color.FromArgb(0xB3, 0, 0, 0), 1),
            },
        }, b);
    }
}
