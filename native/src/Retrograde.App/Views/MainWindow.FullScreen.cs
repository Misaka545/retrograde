using System.Diagnostics;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Media;
using Avalonia.Media.Imaging;
using Avalonia.Threading;
using Retrograde.App.Controls;

namespace Retrograde.App.Views;

/// <summary>
/// Full-screen player reveal: a liquid blot that spreads from the full-screen button like paint or
/// water soaking outward, with a few droplets splashed ahead that the front swallows, and folds back
/// into the button on close.
/// The blot edge is a smooth closed curve whose radius ripples with a few slow sine waves.
/// The ripples calm down as it grows, so it settles into full cover instead of popping.
/// Frames follow the renderer (RequestAnimationFrame), not a timer, so motion is paced to the display.
/// </summary>
public partial class MainWindow
{
    private const double FsOpenMs = 700, FsCloseMs = 420;
    private const int BlotPoints = 18;

    private bool _fsRunning;
    private bool _fsOpening;
    private bool _fsFirstFrame;     // the next frame is the first of this run: take the pictures, start the clocks
    private long _fsLegTicks;       // start of the current leg (a reversal midway starts a new leg)
    private long _fsRunTicks;       // start of the whole run: the ripples keep their time through a reversal
    private double _fsStartShare;   // radius share (0..1) the current leg starts from
    private double _fsShare;        // radius share now
    private Point _fsOrigin;
    private double _fsMaxRadius;
    private readonly double[] _fsPhases = new double[6];
    private (double Angle, double Distance, double Size, double Phase)[] _fsDrops = [];

    // RETRO_FPS=1 or RETRO_FSLOG=1 (no overlay): frame gaps of each reveal go to %TEMP%\retro-fs-frames.txt (dev measurement).
    private static readonly bool LogFsFrames = Environment.GetEnvironmentVariable("RETRO_FPS") == "1" || Environment.GetEnvironmentVariable("RETRO_FSLOG") == "1";
    private readonly List<double> _fsGaps = [];
    private long _fsLastFrame;
    private string _fsSnapLog = "";

    // While the blot moves, the live UI is replaced by two still pictures taken on the first frame:
    // the window without the full-screen player underneath, the player on top, clipped by the blot.
    // A frame is then two image draws instead of re-laying out and repainting both whole UIs in the
    // software renderer (measured: ~25-33 ms per frame live, which is why it looked uneven).
    // The pictures serve both directions, so a reversal midway keeps them.
    private RevealLayer? _fsLayer;
    private RenderTargetBitmap? _fsUnderBmp, _fsOverBmp;
    private readonly List<(Control Control, double Opacity)> _fsHidden = [];

    /// <summary>Draws the two pictures: the window, and over it the player inside the blot.</summary>
    private sealed class RevealLayer : Control
    {
        public IImage? Under, Over;
        public Geometry? Blot;
        public Rect Source; // the pictures' pixels

        public override void Render(DrawingContext context)
        {
            var all = new Rect(Bounds.Size);
            if (Under is not null) context.DrawImage(Under, Source, all);
            if (Over is null || Blot is null) return;
            using (context.PushGeometryClip(Blot)) context.DrawImage(Over, Source, all);
        }
    }

    private void InitFullScreen()
    {
        // The first opening builds the whole player (templates, text, backdrop): about 0.2 s. Do that once
        // shortly after startup, while nobody is waiting for it.
        Opened += (_, _) => DispatcherTimer.RunOnce(WarmFullScreen, TimeSpan.FromSeconds(2), DispatcherPriority.Background);
    }

    private void WarmFullScreen()
    {
        if (_fsRunning || FullScreen.IsVisible || !IsVisible || Content is not Grid root || root.Bounds.Width < 1) return;
        try
        {
            FullScreen.Clip = new RectangleGeometry(default);
            FullScreen.IsVisible = true;
            root.UpdateLayout();
            // Painting it into a 1-pixel bitmap runs every Render (text layout, the composed backdrop)
            // without the cost of filling a window of pixels.
            FullScreen.Clip = null;
            using var probe = new RenderTargetBitmap(new PixelSize(1, 1));
            probe.Render(FullScreen);
        }
        catch (Exception ex)
        {
            Retrograde.Core.ErrorLog.Write("full-screen warm-up", ex);
        }
        finally
        {
            if (!_fsRunning)
            {
                FullScreen.IsVisible = _vm?.IsFullScreen == true;
                FullScreen.Clip = null;
            }
        }
    }

    private void AnimateFullScreen(bool open)
    {
        if (_fsRunning)
        {
            // Reversal midway: same pictures, same origin, same ripples; only the direction and the leg change.
            if (open == _fsOpening) return;
            _fsOpening = open;
            _fsStartShare = _fsShare;
            _fsLegTicks = Stopwatch.GetTimestamp();
            FullScreen.IsHitTestVisible = open;
            if (!open) Ticker.Only = null;
            return;
        }

        var size = Bounds.Size;
        // Nothing to watch (in the tray, minimized, not laid out yet): just switch.
        if (!IsVisible || WindowState == WindowState.Minimized || size.Width < 1 || size.Height < 1)
        {
            _fsOpening = open;
            EndFullScreenRun();
            return;
        }

        // The blot starts at the full-screen button (also when F opened it). The full-screen panel covers
        // the whole window, so window coordinates are its coordinates.
        _fsOrigin = FullScreenButton.TranslatePoint(new Point(FullScreenButton.Bounds.Width / 2, FullScreenButton.Bounds.Height / 2), this)
                    ?? new Point(size.Width / 2, size.Height / 2);
        _fsMaxRadius = new[] { new Point(0, 0), new Point(size.Width, 0), new Point(0, size.Height), new Point(size.Width, size.Height) }
            .Max(c => Math.Sqrt(Math.Pow(c.X - _fsOrigin.X, 2) + Math.Pow(c.Y - _fsOrigin.Y, 2)));

        var rng = Random.Shared;
        for (int i = 0; i < _fsPhases.Length; i++) _fsPhases[i] = rng.NextDouble() * Math.PI * 2;
        // Droplets splash towards the middle of the window, ahead of the front.
        var toCentre = Math.Atan2(size.Height / 2 - _fsOrigin.Y, size.Width / 2 - _fsOrigin.X);
        _fsDrops = Enumerable.Range(0, 4).Select(i => (
            Angle: toCentre + (rng.NextDouble() - 0.5) * 1.6,
            Distance: 0.22 + 0.16 * i + rng.NextDouble() * 0.08,
            Size: 0.035 + rng.NextDouble() * 0.03,
            Phase: rng.NextDouble() * Math.PI * 2)).ToArray();

        _fsOpening = open;
        _fsShare = _fsStartShare = open ? 0 : 1;
        // The clocks start on the first rendered frame: showing the panel costs a layout pass, and timing
        // from before it would make the blot jump ahead on its first visible frame.
        _fsFirstFrame = true;
        _fsLastFrame = 0;
        // Opening: nothing of the player shows until the first frame takes over with the pictures.
        if (open) FullScreen.Clip = new RectangleGeometry(default);
        FullScreen.IsVisible = true;
        // The side that is going away does not take clicks: opening, the player does; closing, the window under it.
        FullScreen.IsHitTestVisible = open;
        if (!open) Ticker.Only = null;
        _fsRunning = true;
        RequestAnimationFrame(_ => Frame());
    }

    /// <summary>The window went to the tray or was minimized: no frames come any more, so finish at once.</summary>
    private void SkipFullScreenAnimation()
    {
        if (_fsRunning) EndFullScreenRun();
    }

    private void Frame()
    {
        if (!_fsRunning) return;
        var now = Stopwatch.GetTimestamp();
        if (LogFsFrames)
        {
            if (_fsLastFrame != 0) _fsGaps.Add(Stopwatch.GetElapsedTime(_fsLastFrame, now).TotalMilliseconds);
            _fsLastFrame = now;
        }

        if (_fsFirstFrame)
        {
            _fsFirstFrame = false;
            // Reversed before anything showed: there is nothing to animate.
            if (_fsOpening ? _fsShare >= 1 : _fsShare <= 0) { EndFullScreenRun(); return; }
            TakeSnapshots();
            now = Stopwatch.GetTimestamp(); // the pictures take a moment: the motion starts after them
            _fsLegTicks = _fsRunTicks = _fsLastFrame = now;
        }

        var elapsed = Stopwatch.GetElapsedTime(_fsLegTicks, now).TotalMilliseconds;
        // A reversal midway runs only the remaining part, at the same pace.
        var span = (_fsOpening ? 1 - _fsStartShare : _fsStartShare) * (_fsOpening ? FsOpenMs : FsCloseMs);
        var t = span <= 0 ? 1 : Math.Clamp(elapsed / span, 0, 1);
        if (_fsOpening)
        {
            // ease-out quart: a quick first spill, then a long soft spread
            var eased = 1 - Math.Pow(1 - t, 4);
            _fsShare = _fsStartShare + (1 - _fsStartShare) * eased;
        }
        else
        {
            // ease-in cubic: draws back slowly, then drains into the button
            _fsShare = _fsStartShare * (1 - t * t * t);
        }

        if (t >= 1)
        {
            EndFullScreenRun();
            return;
        }

        var blot = BuildBlot(_fsShare, Stopwatch.GetElapsedTime(_fsRunTicks, now).TotalSeconds);
        if (_fsLayer is { IsVisible: true })
        {
            _fsLayer.Blot = blot;
            _fsLayer.InvalidateVisual();
        }
        else FullScreen.Clip = blot; // no pictures (they could not be taken): animate the live panel
        RequestAnimationFrame(_ => Frame());
    }

    /// <summary>Back to the live UI, in the state the last request asked for.</summary>
    private void EndFullScreenRun()
    {
        _fsRunning = false;
        _fsFirstFrame = false;
        _fsShare = _fsOpening ? 1 : 0;
        ReleaseSnapshots();
        FullScreen.Clip = null;
        FullScreen.IsHitTestVisible = true;
        FullScreen.IsVisible = _fsOpening;
        // The player covers the whole window: the equalizers and pulses under it need not run.
        Ticker.Only = _fsOpening ? FullScreen : null;
        if (!_fsOpening) _vm?.FullScreenHidden();
        FlushFrameLog();
    }

    private void TakeSnapshots()
    {
        if (Content is not Grid root || root.Bounds.Width < 1 || root.Bounds.Height < 1) return;
        RenderTargetBitmap? under = null, over = null;
        var clip = FullScreen.Clip;
        var opacity = FullScreen.Opacity;
        try
        {
            var t0 = Stopwatch.GetTimestamp();
            root.UpdateLayout(); // the player was just made visible: lay it out before taking its picture
            var t1 = Stopwatch.GetTimestamp();
            var scale = RenderScaling;
            // Rounded, not ceiled: at 150% a 666.67 DIP height is 1000 px, and 1000.0000001 must not become 1001
            // (one extra row makes the picture a hair smaller than the window, which shifts text by a pixel).
            var pixels = new PixelSize(Math.Max(1, (int)Math.Round(root.Bounds.Width * scale)), Math.Max(1, (int)Math.Round(root.Bounds.Height * scale)));
            // The bitmap has the window's device pixels and its DPI, so Render() fills it at the real scale and
            // text is hinted exactly as on screen. The layer draws it from its pixel rectangle: DrawImage's
            // source rectangle is in pixels, and the Size of a bitmap with a DPI other than 96 is not.
            // (An earlier version drew Size: a picture rendered 1.12x too small and stretched back, with
            // text a pixel off, which flickered when the live UI took over.)
            var dpi = new Vector(96 * scale, 96 * scale);

            FullScreen.Clip = null;
            over = new RenderTargetBitmap(pixels, dpi);
            over.Render(FullScreen);
            var t2 = Stopwatch.GetTimestamp();
            // The window underneath, without the player. Opacity 0 is skipped by the renderer and, unlike
            // IsVisible, costs no new layout pass.
            FullScreen.Opacity = 0;
            under = new RenderTargetBitmap(pixels, dpi);
            under.Render(root);
            if (LogFsFrames)
                _fsSnapLog = $" px={pixels.Width}x{pixels.Height} layout={Stopwatch.GetElapsedTime(t0, t1).TotalMilliseconds:0}ms over={Stopwatch.GetElapsedTime(t1, t2).TotalMilliseconds:0}ms under={Stopwatch.GetElapsedTime(t2).TotalMilliseconds:0}ms";
        }
        catch (Exception ex)
        {
            Retrograde.Core.ErrorLog.Write("full-screen pictures", ex);
            under?.Dispose();
            over?.Dispose();
            FullScreen.Opacity = opacity;
            FullScreen.Clip = clip;
            return; // the live panel is animated instead
        }

        if (_fsLayer is null)
        {
            _fsLayer = new RevealLayer { IsHitTestVisible = false, IsVisible = false };
            // The pictures are as large as the window in device pixels: copy them, do not resample.
            RenderOptions.SetBitmapInterpolationMode(_fsLayer, BitmapInterpolationMode.None);
            Grid.SetRowSpan(_fsLayer, Math.Max(1, root.RowDefinitions.Count));
            root.Children.Add(_fsLayer);
        }
        _fsUnderBmp = under;
        _fsOverBmp = over;
        _fsLayer.Under = under;
        _fsLayer.Over = over;
        _fsLayer.Source = new Rect(0, 0, over.PixelSize.Width, over.PixelSize.Height);
        _fsLayer.Blot = BuildBlot(_fsShare, 0);
        // Hide the live UI (Opacity 0 is not painted) so a frame only draws the two pictures.
        // It still takes the pointer: the player while opening, the window under it while closing.
        _fsHidden.Clear();
        foreach (var c in root.Children)
        {
            if (ReferenceEquals(c, _fsLayer)) continue;
            _fsHidden.Add((c, ReferenceEquals(c, FullScreen) ? opacity : c.Opacity));
            c.Opacity = 0;
        }
        _fsLayer.IsVisible = true;
        _fsLayer.InvalidateVisual();
    }

    private void ReleaseSnapshots()
    {
        foreach (var (c, opacity) in _fsHidden) c.Opacity = opacity;
        _fsHidden.Clear();
        if (_fsLayer is not null)
        {
            _fsLayer.IsVisible = false;
            _fsLayer.Under = _fsLayer.Over = null;
            _fsLayer.Blot = null;
        }
        _fsUnderBmp?.Dispose();
        _fsOverBmp?.Dispose();
        _fsUnderBmp = _fsOverBmp = null;
    }

    private void FlushFrameLog()
    {
        if (!LogFsFrames) return;
        if (_fsGaps.Count > 0)
        {
            var sorted = _fsGaps.OrderBy(g => g).ToList();
            File.AppendAllText(Path.Combine(Path.GetTempPath(), "retro-fs-frames.txt"),
                $"{(_fsOpening ? "open " : "close")} frames={_fsGaps.Count} avg={_fsGaps.Average():0.0}ms p95={sorted[(int)(sorted.Count * 0.95)]:0.0}ms max={sorted[^1]:0.0}ms{_fsSnapLog}{Environment.NewLine}");
        }
        _fsGaps.Clear();
        _fsSnapLog = "";
    }

    /// <summary>The blot (and droplets) for a radius share and a time in seconds.</summary>
    private Geometry BuildBlot(double share, double seconds)
    {
        // 1.12 x the farthest corner: the ripples dip inwards, and the end must leave no gap.
        var radius = Math.Max(1, _fsMaxRadius * 1.12 * share);
        // Ripples are strong while small (a fresh spill) and calm down as it covers the window.
        var wobble = 0.16 * Math.Pow(1 - share, 1.3) + 0.025;
        var g = new StreamGeometry();
        using (var ctx = g.Open())
        {
            ctx.SetFillRule(FillRule.NonZero);
            Lobe(ctx, _fsOrigin, radius, wobble, seconds, 0, BlotPoints);

            // In both directions: spreading, a droplet lands ahead and the front swallows it;
            // drawing back, the front leaves it behind and it dries up.
            foreach (var d in _fsDrops)
            {
                var dist = d.Distance * _fsMaxRadius;
                var lead = (radius / dist - 0.55) / 0.45;  // 0 when the front is at 55% of its distance, 1 at 100%
                if (lead <= 0 || lead >= 1.3) continue;
                var grow = Math.Sin(Math.Min(lead, 1) * Math.PI / 2);
                var r = d.Size * _fsMaxRadius * grow;
                if (r < 1) continue;
                var c = new Point(_fsOrigin.X + Math.Cos(d.Angle) * dist, _fsOrigin.Y + Math.Sin(d.Angle) * dist);
                Lobe(ctx, c, r, 0.12, seconds + d.Phase, 1, 10);
            }
        }
        return g;
    }

    /// <summary>A closed smooth curve (Catmull-Rom through points on a rippling circle).</summary>
    private void Lobe(StreamGeometryContext ctx, Point c, double radius, double wobble, double s, int seed, int points)
    {
        var p0 = _fsPhases[(seed * 3) % _fsPhases.Length];
        var p1 = _fsPhases[(seed * 3 + 1) % _fsPhases.Length];
        var p2 = _fsPhases[(seed * 3 + 2) % _fsPhases.Length];
        Span<Point> pts = stackalloc Point[points];
        for (int i = 0; i < points; i++)
        {
            var a = i * Math.PI * 2 / points;
            // three slow waves of different lobe counts drifting at different speeds: an organic, moving edge
            var ripple = 0.55 * Math.Sin(3 * a + s * 2.2 + p0)
                       + 0.30 * Math.Sin(5 * a - s * 1.5 + p1)
                       + 0.15 * Math.Sin(8 * a + s * 3.1 + p2);
            var r = radius * (1 + wobble * ripple);
            pts[i] = new Point(c.X + Math.Cos(a) * r, c.Y + Math.Sin(a) * r);
        }
        ctx.BeginFigure(pts[0], true);
        for (int i = 0; i < points; i++)
        {
            Point a = pts[(i - 1 + points) % points], b = pts[i], cc = pts[(i + 1) % points], d = pts[(i + 2) % points];
            var c1 = new Point(b.X + (cc.X - a.X) / 6, b.Y + (cc.Y - a.Y) / 6);
            var c2 = new Point(cc.X - (d.X - b.X) / 6, cc.Y - (d.Y - b.Y) / 6);
            ctx.CubicBezierTo(c1, c2, cc);
        }
        ctx.EndFigure(true);
    }
}
