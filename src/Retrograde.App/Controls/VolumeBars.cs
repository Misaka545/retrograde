using Avalonia;
using Avalonia.Controls;
using Avalonia.Data;
using Avalonia.Input;
using Avalonia.Media;

namespace Retrograde.App.Controls;

/// <summary>15-segment volume meter (5 teal, 5 gold, 5 orange). Click/drag or wheel, snapped to 1/15.</summary>
public sealed class VolumeBars : Control
{
    public const int Segments = 15;

    public static readonly StyledProperty<double> ValueProperty =
        AvaloniaProperty.Register<VolumeBars, double>(nameof(Value), defaultBindingMode: BindingMode.TwoWay);

    public static readonly StyledProperty<bool> IsMutedProperty =
        AvaloniaProperty.Register<VolumeBars, bool>(nameof(IsMuted));

    private static readonly IBrush Empty = new SolidColorBrush(Color.Parse("#222222"));
    private static readonly IBrush Teal = new SolidColorBrush(Color.Parse("#4FD6BE"));
    private static readonly IBrush Gold = new SolidColorBrush(Color.Parse("#E8C060"));
    private static readonly IBrush Orange = new SolidColorBrush(Color.Parse("#FF6B35"));

    private bool _dragging;

    static VolumeBars()
    {
        AffectsRender<VolumeBars>(ValueProperty, IsMutedProperty);
    }

    public VolumeBars()
    {
        Cursor = new Cursor(StandardCursorType.Hand);
    }

    public double Value
    {
        get => GetValue(ValueProperty);
        set => SetValue(ValueProperty, value);
    }

    public bool IsMuted
    {
        get => GetValue(IsMutedProperty);
        set => SetValue(IsMutedProperty, value);
    }

    public override void Render(DrawingContext context)
    {
        const double gap = 2;
        var w = Bounds.Width;
        var h = Math.Min(24, Bounds.Height);
        var top = (Bounds.Height - h) / 2;
        context.FillRectangle(Brushes.Transparent, new Rect(Bounds.Size));
        var seg = (w - gap * (Segments - 1)) / Segments;
        var active = IsMuted ? 0 : (int)Math.Round(Value * Segments);
        for (int i = 0; i < Segments; i++)
        {
            var brush = i >= active ? Empty : i < 5 ? Teal : i < 10 ? Gold : Orange;
            context.FillRectangle(brush, new Rect(i * (seg + gap), top, seg, h));
        }
    }

    private void SetFrom(PointerEventArgs e)
    {
        var x = e.GetPosition(this).X;
        var pct = Math.Clamp(x / Math.Max(1, Bounds.Width), 0, 1);
        SetCurrentValue(ValueProperty, Math.Round(pct * Segments) / Segments);
    }

    protected override void OnPointerPressed(PointerPressedEventArgs e)
    {
        base.OnPointerPressed(e);
        if (!e.GetCurrentPoint(this).Properties.IsLeftButtonPressed) return;
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
        var step = e.Delta.Y > 0 ? 1 : -1;
        var next = Math.Round((Value + step / (double)Segments) * Segments) / Segments;
        SetCurrentValue(ValueProperty, Math.Clamp(next, 0, 1));
        e.Handled = true;
    }
}
