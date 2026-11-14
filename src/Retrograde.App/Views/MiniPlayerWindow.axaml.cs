using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.LogicalTree;
using Retrograde.App.ViewModels;

namespace Retrograde.App.Views;

/// <summary>The mini player window: placed where it was last left (bottom right the first time), always on top.</summary>
public partial class MiniPlayerWindow : Window
{
    public MiniPlayerWindow()
    {
        InitializeComponent();
        MiniSeek.SeekRequested += async seconds =>
        {
            if (DataContext is MainViewModel vm) await vm.Player.SeekAsync(seconds);
        };
        KeyDown += OnKeyDown;
        Opened += (_, _) => Place();
        PositionChanged += (_, _) => _moved = true;
        Closing += (_, _) => Remember();
    }

    private bool _moved;

    private MainViewModel? Vm => DataContext as MainViewModel;

    /// <summary>The saved spot if it is still on a screen, else the bottom right of the main window's screen.</summary>
    private void Place()
    {
        if (Vm is not { } vm) return;
        var (x, y) = vm.MiniPosition;
        var size = new PixelSize((int)(Width * DesktopScaling), (int)(Height * DesktopScaling));
        if (x is { } sx && y is { } sy && Screens.All.Any(s => s.WorkingArea.Contains(new PixelPoint(sx + 20, sy + 20))))
        {
            Position = new PixelPoint(sx, sy);
        }
        else if ((Screens.Primary ?? Screens.All.FirstOrDefault()) is { } screen)
        {
            var area = screen.WorkingArea;
            var margin = (int)(24 * screen.Scaling);
            Position = new PixelPoint(area.Right - size.Width - margin, area.Bottom - size.Height - margin);
        }
        _moved = false;
    }

    private void Remember()
    {
        if (_moved && Vm is { } vm) vm.MiniPosition = (Position.X, Position.Y);
    }

    private void OnBackgroundPressed(object? sender, PointerPressedEventArgs e)
    {
        if (!e.GetCurrentPoint(this).Properties.IsLeftButtonPressed) return;
        if (e.Source is Visual v && v.FindLogicalAncestorOfType<Button>(includeSelf: true) is not null) return;
        if (e.Source is Visual s && s.FindLogicalAncestorOfType<Controls.SeekBar>(includeSelf: true) is not null) return;
        BeginMoveDrag(e);
    }

    private void OnKeyDown(object? sender, KeyEventArgs e)
    {
        if (Vm is not { } vm) return;
        var p = vm.Player;
        if (e.KeyModifiers == KeyModifiers.Control && e.Key == Key.M)
        {
            vm.ToggleMiniPlayerCommand.Execute(null);
            e.Handled = true;
            return;
        }
        if (e.KeyModifiers != KeyModifiers.None) return;
        switch (e.Key)
        {
            case Key.Space: p.TogglePlayCommand.Execute(null); e.Handled = true; break;
            case Key.Right: p.NextCommand.Execute(null); e.Handled = true; break;
            case Key.Left: p.PrevCommand.Execute(null); e.Handled = true; break;
            case Key.Escape: vm.ToggleMiniPlayerCommand.Execute(null); e.Handled = true; break;
        }
    }
}
