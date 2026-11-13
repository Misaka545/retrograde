using Avalonia;
using Avalonia.Controls;
using Avalonia.Layout;
using Avalonia.Media;
using Avalonia.Threading;
using Retrograde.App.Services;

namespace Retrograde.App.Controls;

/// <summary>
/// Square cover image that loads asynchronously when it enters the visual tree.
/// Containers are recycled by virtualization, so a late result for an old path is dropped.
/// </summary>
public sealed class AsyncCover : Border
{
    public static readonly StyledProperty<string?> PathProperty =
        AvaloniaProperty.Register<AsyncCover, string?>(nameof(Path));

    /// <summary>The Disc placeholder turns while this is true (animate-spin-slow on a playing track without a cover).</summary>
    public static readonly StyledProperty<bool> IsSpinningProperty =
        AvaloniaProperty.Register<AsyncCover, bool>(nameof(IsSpinning));

    public bool IsSpinning
    {
        get => GetValue(IsSpinningProperty);
        set => SetValue(IsSpinningProperty, value);
    }

    public static readonly StyledProperty<int> DecodeWidthProperty =
        AvaloniaProperty.Register<AsyncCover, int>(nameof(DecodeWidth), 240);

    private readonly Image _image = new() { Stretch = Stretch.UniformToFill };
    private static readonly Geometry DiscGeometry =
        Geometry.Parse("M22,12 A10,10 0 1 1 2,12 A10,10 0 1 1 22,12 Z M14,12 A2,2 0 1 1 10,12 A2,2 0 1 1 14,12 Z");

    // Same fallback as the Electron CoverImage: a dim Disc icon, inset 32px on large covers.
    private readonly Glyph _placeholder = new()
    {
        Data = DiscGeometry,
        Foreground = new SolidColorBrush(Color.Parse("#333333")),
        HorizontalAlignment = HorizontalAlignment.Stretch,
        VerticalAlignment = VerticalAlignment.Stretch,
        RenderTransform = new RotateTransform(),
    };
    private int _version;

    public string? Path
    {
        get => GetValue(PathProperty);
        set => SetValue(PathProperty, value);
    }

    public int DecodeWidth
    {
        get => GetValue(DecodeWidthProperty);
        set => SetValue(DecodeWidthProperty, value);
    }

    public AsyncCover()
    {
        Background = new SolidColorBrush(Color.Parse("#222222"));
        ClipToBounds = true;
        var grid = new Grid();
        grid.Children.Add(_placeholder);
        grid.Children.Add(_image);
        Child = grid;
        SizeChanged += (_, e) => _placeholder.Margin = new Thickness(e.NewSize.Width > 64 ? 32 : Math.Max(0, (e.NewSize.Width - 32) / 2));
    }

    protected override void OnPropertyChanged(AvaloniaPropertyChangedEventArgs change)
    {
        base.OnPropertyChanged(change);
        if (change.Property == PathProperty || change.Property == DecodeWidthProperty) Reload();
        else if (change.Property == IsSpinningProperty) UpdateSpin();
    }

    protected override void OnAttachedToVisualTree(VisualTreeAttachmentEventArgs e)
    {
        base.OnAttachedToVisualTree(e);
        Reload();
        UpdateSpin();
    }

    protected override void OnDetachedFromVisualTree(VisualTreeAttachmentEventArgs e)
    {
        base.OnDetachedFromVisualTree(e);
        Ticker.Fast.Remove(this);
    }

    /// <summary>animate-spin-slow: one turn every 6 seconds, on the low-rate ticker, only while the Disc shows.</summary>
    private void UpdateSpin()
    {
        if (IsSpinning && VisualRoot is not null)
            Ticker.Fast.Add(this, () =>
            {
                if (_placeholder.IsVisible && _placeholder.RenderTransform is RotateTransform r) r.Angle = Ticker.Now % 6 / 6 * 360;
            });
        else
        {
            Ticker.Fast.Remove(this);
            if (_placeholder.RenderTransform is RotateTransform r) r.Angle = 0;
        }
    }

    private void Reload()
    {
        var version = ++_version;
        var path = Path;
        if (string.IsNullOrEmpty(path) || VisualRoot is null)
        {
            _image.Source = null;
            _placeholder.IsVisible = string.IsNullOrEmpty(path);
            return;
        }

        var cached = CoverLoader.TryGetCached(path, DecodeWidth);
        if (cached is not null)
        {
            _image.Source = cached;
            _placeholder.IsVisible = false;
            return;
        }

        _image.Source = null;
        _placeholder.IsVisible = false;
        _ = LoadAsync(path, version);
    }

    private async Task LoadAsync(string path, int version)
    {
        var bmp = await CoverLoader.LoadAsync(path, DecodeWidth);
        await Dispatcher.UIThread.InvokeAsync(() =>
        {
            if (version != _version) return;
            _image.Source = bmp;
            _placeholder.IsVisible = bmp is null;
        });
    }
}
