using Avalonia;
using Avalonia.Controls;
using Avalonia.Data;
using Avalonia.Input;
using Avalonia.Media;

namespace Retrograde.App.Controls;

/// <summary>
/// One equalizer band as a vertical segment meter, like the volume bars: a cell per dB from the 0 dB
/// line, boosts upwards in gold (orange near the top), cuts downwards in teal. Drag or wheel in 1 dB
/// steps; double-click resets to 0.
/// </summary>
public sealed class EqSlider : Control
{
    public static readonly StyledProperty<double> ValueProperty =
        AvaloniaProperty.Register<EqSlider, double>(nameof(Value), defaultBindingMode: BindingMode.TwoWay);

    public static readonly StyledProperty<double> RangeProperty =
        AvaloniaProperty.Register<EqSlider, double>(nameof(Range), 12);

    public static readonly StyledProperty<bool> IsActiveProperty =
        AvaloniaProperty.Register<EqSlider, bool>(nameof(IsActive), true);

    private static readonly IBrush Empty = new SolidColorBrush(Color.Parse("#222225"));
    private static readonly IBrush Zero = new SolidColorBrush(Color.Parse("#56554F"));
    private static readonly IBrush Teal = new SolidColorBrush(Color.Parse("#4FD6BE"));
    private static readonly IBrush Gold = new SolidColorBrush(Color.Parse("#E8C060"));
    private static readonly IBrush Orange = new SolidColorBrush(Color.Parse("#FF6B35"));
    private static readonly IBrush Off = new SolidColorBrush(Color.Parse("#46464C"));

    private bool _dragging;

    static EqSlider()
    {
        AffectsRender<EqSlider>(ValueProperty, RangeProperty, IsActiveProperty);
    }

    public EqSlider()
    {
        Cursor = new Cursor(StandardCursorType.SizeNorthSouth);
    }

    public double Value { get => GetValue(ValueProperty); set => SetValue(ValueProperty, value); }
    public double Range { get => GetValue(RangeProperty); set => SetValue(RangeProperty, value); }
    /// <summary>False while the equalizer is off: the curve shows greyed.</summary>
    public bool IsActive { get => GetValue(IsActiveProperty); set => SetValue(IsActiveProperty, value); }

    public override void Render(DrawingContext context)
    {
        var range = (int)Math.Max(1, Range);
        var cells = range * 2; // one per dB, above and below the 0 dB line
        const double gap = 1, zeroLine = 2;
        var w = Math.Min(14, Bounds.Width);
        var left = (Bounds.Width - w) / 2;
        context.FillRectangle(Brushes.Transparent, new Rect(Bounds.Size));
        var cell = (Bounds.Height - zeroLine - gap * cells) / cells;
        if (cell <= 0) return;
        var mid = Bounds.Height / 2;
        var steps = (int)Math.Round(Math.Clamp(Value, -range, range));
        for (int i = 1; i <= range; i++)
        {
            // i-th cell above and below the zero line
            var up = mid - zeroLine / 2 - i * (cell + gap);
            var down = mid + zeroLine / 2 + gap + (i - 1) * (cell + gap);
            var upBrush = steps >= i ? (IsActive ? (i > range * 2 / 3 ? Orange : Gold) : Off) : Empty;
            var downBrush = steps <= -i ? (IsActive ? Teal : Off) : Empty;
            context.FillRectangle(upBrush, new Rect(left, up + gap, w, cell));
            context.FillRectangle(downBrush, new Rect(left, down, w, cell));
        }
        context.FillRectangle(Zero, new Rect(left - 2, mid - zeroLine / 2, w + 4, zeroLine));
    }

    private void SetFrom(PointerEventArgs e)
    {
        var y = e.GetPosition(this).Y;
        var mid = Bounds.Height / 2;
        var range = Math.Max(1, Range);
        var value = Math.Round((mid - y) / Math.Max(1, mid) * range);
        SetCurrentValue(ValueProperty, Math.Clamp(value, -range, range));
    }

    protected override void OnPointerPressed(PointerPressedEventArgs e)
    {
        base.OnPointerPressed(e);
        if (!e.GetCurrentPoint(this).Properties.IsLeftButtonPressed) return;
        if (e.ClickCount == 2)
        {
            SetCurrentValue(ValueProperty, 0d);
            e.Handled = true;
            return;
        }
        _dragging = true;
        e.Pointer.Capture(this);
        SetFrom(e);
        e.Handled = true;
    }

    protected override void OnPointerMoved(PointerEventArgs e)
    {
        base.OnPointerMoved(e);
        if (_dragging) SetFrom(e);
    }

    protected override void OnPointerReleased(PointerReleasedEventArgs e)
    {
        base.OnPointerReleased(e);
        _dragging = false;
        e.Pointer.Capture(null);
    }

    protected override void OnPointerWheelChanged(PointerWheelEventArgs e)
    {
        base.OnPointerWheelChanged(e);
        var range = Math.Max(1, Range);
        SetCurrentValue(ValueProperty, Math.Clamp(Math.Round(Value) + (e.Delta.Y > 0 ? 1 : -1), -range, range));
        e.Handled = true;
    }
}
