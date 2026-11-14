using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Interactivity;
using Avalonia.Threading;

namespace Retrograde.App.Controls;

/// <summary>
/// Shows its child (a TextBlock) trimmed to the space it has; while <see cref="IsActive"/> is on and the text
/// (or <see cref="FollowWindowPointer"/> and the pointer is over its window) and the text
/// is longer than that space, the text glides left to its end, rests, comes back to the start, rests, and
/// repeats. Text that fits never moves.
/// </summary>
public sealed class Marquee : Decorator
{
    public static readonly StyledProperty<bool> IsActiveProperty =
        AvaloniaProperty.Register<Marquee, bool>(nameof(IsActive));

    /// <summary>Scroll the text (bound to the pointer being over the mini player).</summary>
    public bool IsActive
    {
        get => GetValue(IsActiveProperty);
        set => SetValue(IsActiveProperty, value);
    }

    public static readonly StyledProperty<bool> FollowWindowPointerProperty =
        AvaloniaProperty.Register<Marquee, bool>(nameof(FollowWindowPointer));

    /// <summary>Scroll while the pointer moves over the window this is in, stop when it leaves the window.</summary>
    public bool FollowWindowPointer
    {
        get => GetValue(FollowWindowPointerProperty);
        set => SetValue(FollowWindowPointerProperty, value);
    }

    private TopLevel? _top;
    private bool _hover;

    private bool Active => IsActive || (FollowWindowPointer && _hover);

    protected override void OnAttachedToVisualTree(VisualTreeAttachmentEventArgs e)
    {
        base.OnAttachedToVisualTree(e);
        _top = TopLevel.GetTopLevel(this);
        if (_top is null) return;
        _top.AddHandler(PointerMovedEvent, OnWindowPointerMoved, RoutingStrategies.Tunnel, handledEventsToo: true);
        _top.PointerExited += OnWindowPointerExited;
    }

    private void OnWindowPointerMoved(object? sender, PointerEventArgs e)
    {
        if (_hover) return;
        _hover = true;
        Update();
    }

    private void OnWindowPointerExited(object? sender, PointerEventArgs e)
    {
        _hover = false;
        Update();
    }

    private const double Speed = 32;     // DIP per second
    private const double RestMs = 1200;  // at either end

    private DispatcherTimer? _timer;
    private double _offset;
    private double _overflow;
    private long _phaseStart;
    private int _phase; // 0 rest at start, 1 moving to the end, 2 rest at the end, 3 back to the start

    public Marquee()
    {
        ClipToBounds = true;
    }

    static Marquee()
    {
        IsActiveProperty.Changed.AddClassHandler<Marquee>((m, _) => m.Update());
    }

    protected override Size MeasureOverride(Size availableSize)
    {
        if (Child is null) return default;
        // The full text width, but never more than the space given: the parent decides how wide we are
        Child.Measure(new Size(double.PositiveInfinity, availableSize.Height));
        var natural = Child.DesiredSize;
        return new Size(double.IsInfinity(availableSize.Width) ? natural.Width : Math.Min(natural.Width, availableSize.Width), natural.Height);
    }

    protected override Size ArrangeOverride(Size finalSize)
    {
        if (Child is null) return finalSize;
        var natural = Child.DesiredSize.Width;
        _overflow = Math.Max(0, natural - finalSize.Width);
        var moving = Active && _overflow > 0.5;
        // At rest the text is laid out in the visible width, so it ends in an ellipsis
        var width = moving ? natural : finalSize.Width;
        Child.Arrange(new Rect(-(moving ? _offset : 0), 0, width, finalSize.Height));
        Dispatcher.UIThread.Post(Update, DispatcherPriority.Background);
        return finalSize;
    }

    private void Update()
    {
        var run = Active && _overflow > 0.5 && IsEffectivelyVisible;
        if (run && _timer is null)
        {
            _phase = 0;
            _offset = 0;
            _phaseStart = Environment.TickCount64;
            _timer = new DispatcherTimer(TimeSpan.FromMilliseconds(16), DispatcherPriority.Render, (_, _) => Step());
            _timer.Start();
            InvalidateArrange();
        }
        else if (!run && _timer is not null)
        {
            _timer.Stop();
            _timer = null;
            _offset = 0;
            InvalidateArrange();
        }
    }

    private void Step()
    {
        var elapsed = Environment.TickCount64 - _phaseStart;
        var travelMs = _overflow / Speed * 1000;
        switch (_phase)
        {
            case 0 or 2 when elapsed >= RestMs:
                _phase++;
                _phaseStart = Environment.TickCount64;
                break;
            case 1:
                _offset = Math.Min(_overflow, elapsed / 1000.0 * Speed);
                if (elapsed >= travelMs) { _phase = 2; _phaseStart = Environment.TickCount64; }
                break;
            case 3:
                // Back faster than out, so the start is soon readable again
                _offset = Math.Max(0, _overflow - elapsed / 1000.0 * Speed * 3);
                if (_offset <= 0) { _phase = 0; _phaseStart = Environment.TickCount64; }
                break;
        }
        InvalidateArrange();
    }

    protected override void OnDetachedFromVisualTree(VisualTreeAttachmentEventArgs e)
    {
        base.OnDetachedFromVisualTree(e);
        if (_top is not null)
        {
            _top.RemoveHandler(PointerMovedEvent, OnWindowPointerMoved);
            _top.PointerExited -= OnWindowPointerExited;
            _top = null;
        }
        _hover = false;
        _timer?.Stop();
        _timer = null;
        _offset = 0;
    }
}
