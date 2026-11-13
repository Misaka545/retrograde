using Avalonia;
using Avalonia.Controls;

namespace Retrograde.App.Controls;

/// <summary>
/// Border whose height always equals its width (CSS aspect-square), measured in one pass.
/// Replaces Height="{Binding $self.Bounds.Width}", which needed a second layout pass per card.
/// </summary>
public sealed class SquareBorder : Border
{
    protected override Size MeasureOverride(Size availableSize)
    {
        var w = double.IsInfinity(availableSize.Width) ? 200 : availableSize.Width;
        if (!double.IsInfinity(availableSize.Height)) w = Math.Min(w, availableSize.Height); // fit both ways when height is bounded
        base.MeasureOverride(new Size(w, w));
        return new Size(w, w);
    }

    protected override Size ArrangeOverride(Size finalSize)
    {
        var side = Math.Min(finalSize.Width, finalSize.Height);
        var s = new Size(side, side);
        base.ArrangeOverride(s);
        return s;
    }
}
