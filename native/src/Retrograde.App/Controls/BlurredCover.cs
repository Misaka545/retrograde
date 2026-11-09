using Avalonia;
using Avalonia.Controls;
using Avalonia.Media;
using Avalonia.Media.Imaging;
using Avalonia.Threading;
using SkiaSharp;

namespace Retrograde.App.Controls;

/// <summary>
/// Full-screen backdrop: the cover blurred once off-thread into a small bitmap (the Electron build
/// uses CSS blur(120px)). Stretching a pre-blurred 96px image costs nothing per frame, unlike a live
/// blur effect over the whole window in the software renderer.
/// </summary>
public sealed class BlurredCover : Control
{
    public static readonly StyledProperty<string?> PathProperty =
        AvaloniaProperty.Register<BlurredCover, string?>(nameof(Path));

    private Bitmap? _bitmap;
    private int _version;

    static BlurredCover()
    {
        AffectsRender<BlurredCover>(PathProperty);
    }

    public string? Path
    {
        get => GetValue(PathProperty);
        set => SetValue(PathProperty, value);
    }

    protected override void OnPropertyChanged(AvaloniaPropertyChangedEventArgs change)
    {
        base.OnPropertyChanged(change);
        if (change.Property == PathProperty) Reload();
    }

    private void Reload()
    {
        var version = ++_version;
        var path = Path;
        if (string.IsNullOrEmpty(path) || !File.Exists(path))
        {
            Swap(null);
            return;
        }
        _ = Task.Run(() => Blur(path)).ContinueWith(t =>
            Dispatcher.UIThread.Post(() => { if (version == _version) Swap(t.Result); else t.Result?.Dispose(); }),
            TaskScheduler.Default);
    }

    private void Swap(Bitmap? bmp)
    {
        var old = _bitmap;
        _bitmap = bmp;
        old?.Dispose();
        InvalidateVisual();
    }

    private static Bitmap? Blur(string path)
    {
        try
        {
            using var src = SKBitmap.Decode(path);
            if (src is null) return null;
            const int size = 96;
            using var small = src.Resize(new SKImageInfo(size, size), new SKSamplingOptions(SKFilterMode.Linear, SKMipmapMode.Linear));
            if (small is null) return null;
            using var surface = SKSurface.Create(new SKImageInfo(size, size, SKColorType.Bgra8888, SKAlphaType.Premul));
            using var paint = new SKPaint { ImageFilter = SKImageFilter.CreateBlur(6, 6, SKShaderTileMode.Clamp) };
            surface.Canvas.DrawBitmap(small, 0, 0, paint);
            using var image = surface.Snapshot();
            using var data = image.Encode(SKEncodedImageFormat.Png, 100);
            using var ms = new MemoryStream(data.ToArray());
            return new Bitmap(ms);
        }
        catch
        {
            return null;
        }
    }

    public override void Render(DrawingContext context)
    {
        if (_bitmap is null) return;
        // object-cover at scale(1.5), centred.
        var b = Bounds;
        var side = Math.Max(b.Width, b.Height) * 1.5;
        var dest = new Rect((b.Width - side) / 2, (b.Height - side) / 2, side, side);
        context.DrawImage(_bitmap, new Rect(_bitmap.Size), dest);
    }
}
