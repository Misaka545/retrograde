using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Media;

namespace Retrograde.App.Controls;

/// <summary>
/// Thin progress strip on top of the player bar: 4px track, orange→gold→teal fill spanning the
/// played part, teal handle on hover. Seeks once on release so dragging never floods mpv.
/// </summary>
public sealed class SeekBar : Control
{
    public static readonly StyledProperty<double> ValueProperty =
        AvaloniaProperty.Register<SeekBar, double>(nameof(Value));

    public static readonly StyledProperty<double> MaximumProperty =
        AvaloniaProperty.Register<SeekBar, double>(nameof(Maximum), 1);

    public static readonly StyledProperty<bool> IsPlainProperty =
        AvaloniaProperty.Register<SeekBar, bool>(nameof(IsPlain));

    /// <summary>Full-screen style: rounded white fill on #333, no handle.</summary>
    public bool IsPlain
    {
        get => GetValue(IsPlainProperty);
        set => SetValue(IsPlainProperty, value);
    }

    private static readonly IBrush TrackBrush = new SolidColorBrush(Color.Parse("#1A1A1A"));
    private static readonly IBrush PlainTrack = new SolidColorBrush(Color.Parse("#333333"));
    private static readonly IBrush HandleBrush = new SolidColorBrush(Color.Parse("#4FD6BE"));
    private static readonly GradientStops FillStops =
    [
        new GradientStop(Color.Parse("#FF6B35"), 0),
        new GradientStop(Color.Parse("#E8C060"), 0.5),
        new GradientStop(Color.Parse("#4FD6BE"), 1),
    ];

    private bool _dragging;
    private bool _hover;
    private double _dragValue;

    public event Action<double>? SeekRequested;

    static SeekBar()
    {
        AffectsRender<SeekBar>(ValueProperty, MaximumProperty, IsPlainProperty);
    }

    public SeekBar()
    {
        Cursor = new Cursor(StandardCursorType.Hand);
        Height = 12;
    }

    public double Value
    {
        get => GetValue(ValueProperty);
        set => SetValue(ValueProperty, value);
    }

    public double Maximum
    {
        get => GetValue(MaximumProperty);
        set => SetValue(MaximumProperty, value);
    }

    public override void Render(DrawingContext context)
    {
        var w = Bounds.Width;
        if (IsPlain)
        {
            context.FillRectangle(Brushes.Transparent, new Rect(Bounds.Size));
            var y = (Bounds.Height - 4) / 2;
            context.DrawRectangle(PlainTrack, null, new Rect(0, y, w, 4), 2, 2);
            var m = Maximum > 0 ? Maximum : 1;
            var f = Math.Clamp((_dragging ? _dragValue : Value) / m, 0, 1) * w;
            if (f >= 1)
            {
                using (context.PushClip(new RoundedRect(new Rect(0, y, w, 4), 2)))
                    context.FillRectangle(Brushes.White, new Rect(0, y, f, 4));
            }
            return;
        }
        // Transparent hit area over the full 12px height, visible track 4px tall at y=4.
        context.FillRectangle(Brushes.Transparent, new Rect(0, 0, w, Bounds.Height));
        context.FillRectangle(TrackBrush, new Rect(0, 4, w, 4));
        var max = Maximum > 0 ? Maximum : 1;
        var value = _dragging ? _dragValue : Value;
        var fill = Math.Clamp(value / max, 0, 1) * w;
        if (fill >= 1)
        {
            var brush = new LinearGradientBrush
            {
                StartPoint = new RelativePoint(0, 0, RelativeUnit.Relative),
                EndPoint = new RelativePoint(1, 0, RelativeUnit.Relative),
                GradientStops = FillStops,
            };
            context.FillRectangle(brush, new Rect(0, 4, fill, 4));
        }
        if (_hover || _dragging)
            context.FillRectangle(HandleBrush, new Rect(Math.Clamp(fill - 4, 0, Math.Max(0, w - 8)), 0, 8, 12));
    }

    private double ValueAt(PointerEventArgs e)
    {
        var x = e.GetPosition(this).X;
        var max = Maximum > 0 ? Maximum : 1;
        return Math.Clamp(x / Math.Max(1, Bounds.Width), 0, 1) * max;
    }

    protected override void OnPointerEntered(PointerEventArgs e)
    {
        base.OnPointerEntered(e);
        _hover = true;
        InvalidateVisual();
    }

    protected override void OnPointerExited(PointerEventArgs e)
    {
        base.OnPointerExited(e);
        _hover = false;
        InvalidateVisual();
    }

    protected override void OnPointerPressed(PointerPressedEventArgs e)
    {
        base.OnPointerPressed(e);
        if (!e.GetCurrentPoint(this).Properties.IsLeftButtonPressed) return;
        _dragging = true;
        _dragValue = ValueAt(e);
        e.Pointer.Capture(this);
        e.Handled = true;
        InvalidateVisual();
    }

    protected override void OnPointerMoved(PointerEventArgs e)
    {
        base.OnPointerMoved(e);
        if (!_dragging) return;
        _dragValue = ValueAt(e);
        InvalidateVisual();
    }

    protected override void OnPointerReleased(PointerReleasedEventArgs e)
    {
        base.OnPointerReleased(e);
        if (!_dragging) return;
        _dragging = false;
        _dragValue = ValueAt(e);
        e.Pointer.Capture(null);
        SetCurrentValue(ValueProperty, _dragValue);
        SeekRequested?.Invoke(_dragValue);
        InvalidateVisual();
    }

    protected override void OnPointerCaptureLost(PointerCaptureLostEventArgs e)
    {
        base.OnPointerCaptureLost(e);
        _dragging = false;
        InvalidateVisual();
    }
}
