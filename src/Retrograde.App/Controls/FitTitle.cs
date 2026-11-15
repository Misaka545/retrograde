using Avalonia;
using Avalonia.Controls;
using Avalonia.Media;

namespace Retrograde.App.Controls;

/// <summary>
/// A page title that keeps the header as tall as the cover: the largest size step at which the text fits in
/// that step's line count, measured at the width it really has. At the last step the text is cut with an
/// ellipsis and the full title shows in a tooltip.
/// </summary>
public sealed class FitTitle : TextBlock
{
    protected override Type StyleKeyOverride => typeof(TextBlock);

    /// <summary>Font size, line height, letter spacing, most lines.</summary>
    private static readonly (double Size, double Line, double Spacing, int Lines)[] Steps =
    [
        (60, 66, -3, 2),
        (46, 51, -2, 3),
        (36, 41, -1.4, 3),
    ];

    private double _fittedWidth = double.NaN;
    private string? _fittedText;

    public FitTitle()
    {
        TextWrapping = TextWrapping.Wrap;
        TextTrimming = TextTrimming.CharacterEllipsis;
    }

    protected override Size MeasureOverride(Size availableSize)
    {
        var width = availableSize.Width;
        if (!double.IsInfinity(width) && (width != _fittedWidth || Text != _fittedText))
        {
            _fittedWidth = width;
            _fittedText = Text;
            Fit(availableSize);
        }
        return base.MeasureOverride(availableSize);
    }

    /// <summary>Tries each step with the TextBlock's own layout, unlimited lines, and keeps the first that fits.</summary>
    private void Fit(Size availableSize)
    {
        var cut = false;
        for (var i = 0; i < Steps.Length; i++)
        {
            var s = Steps[i];
            SetCurrentValue(FontSizeProperty, s.Size);
            SetCurrentValue(LineHeightProperty, s.Line);
            SetCurrentValue(LetterSpacingProperty, s.Spacing);
            SetCurrentValue(MaxLinesProperty, 0);
            base.MeasureOverride(availableSize);
            var lines = TextLayout.TextLines.Count;
            var last = i == Steps.Length - 1;
            if (lines <= s.Lines || last)
            {
                cut = lines > s.Lines;
                SetCurrentValue(MaxLinesProperty, s.Lines);
                break;
            }
        }
        ToolTip.SetTip(this, cut ? Text : null);
    }
}
