using System.Collections.ObjectModel;
using Avalonia.Media;
using Avalonia.Media.Imaging;
using Avalonia.Platform;
using Avalonia.Threading;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using SkiaSharp;

namespace Retrograde.App.ViewModels;

/// <summary>
/// One bar of the sidebar visualizer. Pressing it types a Morse symbol (Sidebar.jsx):
/// teal bars are ".", gold bars a space, orange bars "-".
/// </summary>
public sealed partial class VisualizerBar : ObservableObject
{
    private static readonly IBrush Idle = new SolidColorBrush(Color.Parse("#333333"));
    private readonly IBrush _active;
    private readonly double _height;
    private readonly Action<VisualizerBar> _press;

    public VisualizerBar(double height, string activeColor, char symbol, Action<VisualizerBar> press)
    {
        _height = height;
        _active = new SolidColorBrush(Color.Parse(activeColor));
        Symbol = symbol;
        _press = press;
    }

    public char Symbol { get; }

    [ObservableProperty] [NotifyPropertyChangedFor(nameof(BarHeight), nameof(BarBrush))]
    private bool _isActive;

    /// <summary>The bar jumps to 80% of the 32px strip while lit.</summary>
    public double BarHeight => IsActive ? 25.6 : _height;
    public IBrush BarBrush => IsActive ? _active : Idle;

    [RelayCommand] private void Press() => _press(this);
}

/// <summary>The hidden PRIVATE_MEMORY_BANK: typing LONETRAIL in Morse on the visualizer opens it.</summary>
public partial class MainViewModel
{
    private const string TargetCode = ".-.. --- -. . - .-. .- .. .-.."; // LONETRAIL
    private string _morse = "";
    private DispatcherTimer? _morseReset;
    private Bitmap? _eggColor, _eggGray;

    private IReadOnlyList<VisualizerBar>? _visualizerBars;

    /// <summary>Heights as a share of the 32px strip, same order as the Electron bars.</summary>
    public IReadOnlyList<VisualizerBar> VisualizerBars => _visualizerBars ??= new[] { 25, 45, 20, 55, 30, 60, 15, 40, 25, 50, 20, 35 }
        .Select((pct, i) => (i % 3) switch
        {
            0 => new VisualizerBar(pct * 0.32, "#4FD6BE", '.', OnBarPressed),
            1 => new VisualizerBar(pct * 0.32, "#E8C060", ' ', OnBarPressed),
            _ => new VisualizerBar(pct * 0.32, "#FF6B35", '-', OnBarPressed),
        })
        .ToList();

    [ObservableProperty] private bool _isEggOpen;

    public Bitmap? EggColor => _eggColor ??= LoadEgg();
    /// <summary>Grey version shown until the pointer rests on the picture (grayscale → colour on hover).</summary>
    public Bitmap? EggGray => _eggGray ??= MakeGray();

    private void OnBarPressed(VisualizerBar bar)
    {
        bar.IsActive = true;
        DispatcherTimer.RunOnce(() => bar.IsActive = false, TimeSpan.FromMilliseconds(150));

        _morse += bar.Symbol;
        if (_morse.Contains(TargetCode, StringComparison.Ordinal))
        {
            _morse = "";
            _morseReset?.Stop();
            IsEggOpen = true;
            return;
        }
        // The buffer forgets what was typed after 3 seconds without a press.
        _morseReset ??= new DispatcherTimer { Interval = TimeSpan.FromSeconds(3) };
        _morseReset.Tick -= OnMorseTimeout;
        _morseReset.Tick += OnMorseTimeout;
        _morseReset.Stop();
        _morseReset.Start();
    }

    private void OnMorseTimeout(object? sender, EventArgs e)
    {
        _morseReset?.Stop();
        _morse = "";
    }

    [RelayCommand] private void CloseEgg() => IsEggOpen = false;

    private static Bitmap? LoadEgg()
    {
        try
        {
            using var s = AssetLoader.Open(new Uri("avares://Retrograde/Assets/kristen.png"));
            return new Bitmap(s);
        }
        catch { return null; }
    }

    private static Bitmap? MakeGray()
    {
        try
        {
            using var s = AssetLoader.Open(new Uri("avares://Retrograde/Assets/kristen.png"));
            using var src = SKBitmap.Decode(s);
            if (src is null) return null;
            using var gray = new SKBitmap(src.Width, src.Height);
            using (var canvas = new SKCanvas(gray))
            using (var paint = new SKPaint())
            {
                // Rec. 709 luma, like CSS grayscale(1).
                paint.ColorFilter = SKColorFilter.CreateColorMatrix(
                [
                    0.2126f, 0.7152f, 0.0722f, 0, 0,
                    0.2126f, 0.7152f, 0.0722f, 0, 0,
                    0.2126f, 0.7152f, 0.0722f, 0, 0,
                    0, 0, 0, 1, 0,
                ]);
                canvas.DrawBitmap(src, 0, 0, paint);
            }
            using var img = SKImage.FromBitmap(gray);
            using var data = img.Encode(SKEncodedImageFormat.Png, 100);
            using var ms = new MemoryStream(data.ToArray());
            return new Bitmap(ms);
        }
        catch { return null; }
    }
}
