using Avalonia;
using Avalonia.Controls;
using Avalonia.Media;
using Avalonia.Threading;
using Avalonia.VisualTree;

namespace Retrograde.App.Controls;

/// <summary>
/// Shared low-rate clock for the animations that can run for a long time (equalizer, spinning disc,
/// pulsing dot). With software rendering every animated frame repaints the window, and a 60 fps loop
/// costs about a quarter of a CPU core; at 12 or 4 fps the same motion costs a few percent.
/// Controls on one ticker are updated in the same frame, and nothing runs while the window is idle
/// (not in front, hidden or minimized: the Window.idle class), like the Electron build's app-idle pause.
/// </summary>
public sealed class Ticker
{
    /// <summary>12 fps: about the frame rate of the equalizer GIF the Electron build shows.</summary>
    public static readonly Ticker Fast = new(TimeSpan.FromMilliseconds(1000.0 / 12));
    /// <summary>
    /// 4 fps: enough for a slow 2-second breathing pulse. The logo dot is always on screen, so this
    /// rate is what it costs whenever the window is in front (about 2% of a core, measured).
    /// </summary>
    public static readonly Ticker Slow = new(TimeSpan.FromMilliseconds(1000.0 / 4));

    private readonly DispatcherTimer _timer;
    private readonly Dictionary<Control, Action> _subscribers = [];

    private Ticker(TimeSpan interval)
    {
        _timer = new DispatcherTimer(DispatcherPriority.Render) { Interval = interval };
        _timer.Tick += (_, _) =>
        {
            foreach (var (control, update) in _subscribers.ToArray())
                if (!IsIdle(control)) update();
        };
    }

    public void Add(Control control, Action update)
    {
        _subscribers[control] = update;
        if (!_timer.IsEnabled) _timer.Start();
    }

    public void Remove(Control control)
    {
        _subscribers.Remove(control);
        if (_subscribers.Count == 0) _timer.Stop();
    }

    /// <summary>
    /// When set, only controls inside this visual are animated: the full-screen player covers the rest
    /// of the window, so an equalizer or a pulse under it would repaint for nobody.
    /// </summary>
    public static Visual? Only { get; set; }

    private static bool IsIdle(Control c) =>
        TopLevel.GetTopLevel(c) is not { } top || top.Classes.Contains("idle") || (Only is { } only && !only.IsVisualAncestorOf(c));

    /// <summary>Seconds on a steady clock, for phase-based motion that does not drift with frame timing.</summary>
    public static double Now => Environment.TickCount64 / 1000.0;
}

/// <summary>
/// Three bars bouncing next to the playing track (the animated equalizer GIF of the Electron build).
/// Still while paused.
/// </summary>
public sealed class Equalizer : Control
{
    public static readonly StyledProperty<bool> IsPlayingProperty =
        AvaloniaProperty.Register<Equalizer, bool>(nameof(IsPlaying));

    public static readonly StyledProperty<IBrush?> FillProperty =
        AvaloniaProperty.Register<Equalizer, IBrush?>(nameof(Fill), Brushes.Gray);

    public bool IsPlaying
    {
        get => GetValue(IsPlayingProperty);
        set => SetValue(IsPlayingProperty, value);
    }

    public IBrush? Fill
    {
        get => GetValue(FillProperty);
        set => SetValue(FillProperty, value);
    }

    static Equalizer() => AffectsRender<Equalizer>(FillProperty, IsPlayingProperty);

    protected override void OnPropertyChanged(AvaloniaPropertyChangedEventArgs change)
    {
        base.OnPropertyChanged(change);
        if (change.Property == IsPlayingProperty) Subscribe();
    }

    protected override void OnAttachedToVisualTree(VisualTreeAttachmentEventArgs e)
    {
        base.OnAttachedToVisualTree(e);
        Subscribe();
    }

    protected override void OnDetachedFromVisualTree(VisualTreeAttachmentEventArgs e)
    {
        base.OnDetachedFromVisualTree(e);
        Ticker.Fast.Remove(this);
    }

    private void Subscribe()
    {
        if (IsPlaying && VisualRoot is not null) Ticker.Fast.Add(this, InvalidateVisual);
        else Ticker.Fast.Remove(this);
    }

    public override void Render(DrawingContext context)
    {
        var w = Bounds.Width;
        var h = Bounds.Height;
        if (w <= 0 || h <= 0 || Fill is null) return;
        double a, b, c;
        if (IsPlaying)
        {
            var t = Ticker.Now;
            a = 0.65 + 0.35 * Math.Sin(t * 2 * Math.PI / 0.9);
            b = 0.675 - 0.325 * Math.Sin(t * 2 * Math.PI / 0.7);
            c = 0.7 + 0.2 * Math.Sin(t * 2 * Math.PI / 1.1);
        }
        else
        {
            a = 0.5; b = 1; c = 0.7;
        }
        var bar = w / 4;               // three bars, two gaps of half a bar
        var gap = (w - 3 * bar) / 2;
        Draw(context, 0, bar, h, a);
        Draw(context, bar + gap, bar, h, b);
        Draw(context, 2 * (bar + gap), bar, h, c);
    }

    private void Draw(DrawingContext context, double x, double width, double height, double share) =>
        context.FillRectangle(Fill!, new Rect(x, height * (1 - share), width, height * share));
}

/// <summary>Slow opacity breathing (animate-pulse / animate-pulse-tech): 1 → 0.5 → 1 every 2 seconds.</summary>
public static class Pulse
{
    public static readonly AttachedProperty<bool> IsEnabledProperty =
        AvaloniaProperty.RegisterAttached<Control, bool>("IsEnabled", typeof(Pulse));

    public static bool GetIsEnabled(Control c) => c.GetValue(IsEnabledProperty);
    public static void SetIsEnabled(Control c, bool v) => c.SetValue(IsEnabledProperty, v);

    static Pulse()
    {
        IsEnabledProperty.Changed.AddClassHandler<Control>((c, e) =>
        {
            c.AttachedToVisualTree -= OnAttached;
            c.DetachedFromVisualTree -= OnDetached;
            if (e.NewValue is not true)
            {
                Ticker.Slow.Remove(c);
                c.Opacity = 1;
                return;
            }
            c.AttachedToVisualTree += OnAttached;
            c.DetachedFromVisualTree += OnDetached;
            if (TopLevel.GetTopLevel(c) is not null) Start(c);
        });
    }

    private static void OnAttached(object? sender, VisualTreeAttachmentEventArgs e)
    {
        if (sender is Control c) Start(c);
    }

    private static void OnDetached(object? sender, VisualTreeAttachmentEventArgs e)
    {
        if (sender is Control c) Ticker.Slow.Remove(c);
    }

    // Hidden controls (a closed overlay) do not tick.
    private static void Start(Control c) => Ticker.Slow.Add(c, () =>
    {
        if (c.IsEffectivelyVisible) c.Opacity = 0.75 + 0.25 * Math.Cos(Ticker.Now * Math.PI);
    });
}
