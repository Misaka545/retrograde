using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.Documents;
using Avalonia.Media;

namespace Retrograde.App.Controls;

/// <summary>
/// Lucide-style stroke icon: path data authored on a 24x24 grid, stroked with the inherited
/// Foreground at width 2 with round caps, scaled to the control size (same as lucide-react).
/// </summary>
public sealed class Glyph : Control
{
    public static readonly StyledProperty<Geometry?> DataProperty =
        AvaloniaProperty.Register<Glyph, Geometry?>(nameof(Data));

    public static readonly StyledProperty<IBrush?> ForegroundProperty =
        TextElement.ForegroundProperty.AddOwner<Glyph>();

    public static readonly StyledProperty<bool> FilledProperty =
        AvaloniaProperty.Register<Glyph, bool>(nameof(Filled));

    public static readonly StyledProperty<double> StrokeThicknessProperty =
        AvaloniaProperty.Register<Glyph, double>(nameof(StrokeThickness), 2.0);

    static Glyph()
    {
        AffectsRender<Glyph>(DataProperty, ForegroundProperty, FilledProperty, StrokeThicknessProperty);
    }

    public Geometry? Data
    {
        get => GetValue(DataProperty);
        set => SetValue(DataProperty, value);
    }

    public IBrush? Foreground
    {
        get => GetValue(ForegroundProperty);
        set => SetValue(ForegroundProperty, value);
    }

    public bool Filled
    {
        get => GetValue(FilledProperty);
        set => SetValue(FilledProperty, value);
    }

    public double StrokeThickness
    {
        get => GetValue(StrokeThicknessProperty);
        set => SetValue(StrokeThicknessProperty, value);
    }

    protected override Size MeasureOverride(Size availableSize) => new(
        double.IsNaN(Width) ? 16 : Width,
        double.IsNaN(Height) ? 16 : Height);

    public override void Render(DrawingContext context)
    {
        var geometry = Data;
        var brush = Foreground;
        if (geometry is null || brush is null) return;
        var size = Math.Min(Bounds.Width, Bounds.Height);
        if (size <= 0) return;
        var scale = size / 24.0;
        var dx = (Bounds.Width - size) / 2;
        var dy = (Bounds.Height - size) / 2;
        using (context.PushTransform(Matrix.CreateScale(scale, scale) * Matrix.CreateTranslation(dx, dy)))
        {
            var pen = new Pen(brush, StrokeThickness, lineCap: PenLineCap.Round, lineJoin: PenLineJoin.Round);
            context.DrawGeometry(Filled ? brush : null, pen, geometry);
        }
    }
}
