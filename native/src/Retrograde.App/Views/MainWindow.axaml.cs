using System.ComponentModel;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.Primitives;
using Avalonia.Controls.Templates;
using Avalonia.Platform.Storage;
using Retrograde.Core;
using Avalonia.Input;
using Avalonia.Interactivity;
using Avalonia.LogicalTree;
using Avalonia.VisualTree;
using Avalonia.Threading;
using Retrograde.App.ViewModels;

namespace Retrograde.App.Views;

public partial class MainWindow : Window
{
    private MainViewModel? _vm;

    public MainWindow()
    {
        InitializeComponent();
        SeekBar.SeekRequested += async seconds =>
        {
            if (Player is not null) await Player.SeekAsync(seconds);
        };
        if (Environment.GetEnvironmentVariable("RETRO_FPS") == "1")
            RendererDiagnostics.DebugOverlays = Avalonia.Rendering.RendererDebugOverlays.Fps | Avalonia.Rendering.RendererDebugOverlays.LayoutTimeGraph | Avalonia.Rendering.RendererDebugOverlays.RenderTimeGraph;
        KeyDown += OnKeyDown;
        Closing += OnWindowClosing;
        AddHandler(DragDrop.DragOverEvent, OnDragOver);
        AddHandler(DragDrop.DropEvent, OnDrop);
        FullSeekBar.SeekRequested += async seconds =>
        {
            if (Player is not null) await Player.SeekAsync(seconds);
        };
        SizeChanged += (_, e) =>
        {
            _vm?.SetWindowWidth(e.NewSize.Width);
            QueuePopup.MaxHeight = Math.Max(200, e.NewSize.Height - 150); // calc(100vh - 150px)
        };
        // Queue popup closes on any press outside it (the press still reaches its target).
        AddHandler(PointerPressedEvent, (_, e) =>
        {
            if (_vm is not { IsQueueOpen: true } || e.Source is not Visual v) return;
            if (QueuePopup.IsVisualAncestorOf(v) || QueueButton.IsVisualAncestorOf(v) || v == QueueButton) return;
            _vm.IsQueueOpen = false;
        }, RoutingStrategies.Tunnel, handledEventsToo: true);
        PlayerGrid.SizeChanged += (_, e) =>
            PlayerGrid.ColumnDefinitions[0].Width = new GridLength(Math.Max(200, e.NewSize.Width * 0.25));
    }

    private PlayerViewModel? Player => _vm?.Player;

    /// <summary>Set by the app when it is really shutting down, so Closing is not intercepted.</summary>
    public bool AllowClose { get; set; }

    protected override void OnPropertyChanged(AvaloniaPropertyChangedEventArgs change)
    {
        base.OnPropertyChanged(change);
        if ((change.Property == IsVisibleProperty || change.Property == WindowStateProperty) && Player is not null)
            Player.IsUiActive = IsVisible && WindowState != WindowState.Minimized;
    }

    private void OnWindowClosing(object? sender, WindowClosingEventArgs e)
    {
        if (AllowClose || _vm is null || e.CloseReason is WindowCloseReason.OSShutdown or WindowCloseReason.ApplicationShutdown) return;
        e.Cancel = true;          // the close button decides between tray and quit
        _vm.RequestClose();
    }

    /// <summary>Show from the tray / a second launch: restore, show and bring to front.</summary>
    public void Reveal()
    {
        if (!IsVisible) Show();
        if (WindowState == WindowState.Minimized) WindowState = WindowState.Normal;
        Activate();
        Topmost = true;   // Windows only lets a background process take focus this way
        Topmost = false;
    }

    protected override void OnDataContextChanged(EventArgs e)
    {
        base.OnDataContextChanged(e);
        if (_vm is not null)
        {
            _vm.PropertyChanged -= OnVmPropertyChanged;
            _vm.DetailOpened -= OnDetailOpened;
        }
        _vm = DataContext as MainViewModel;
        if (_vm is null) return;
        _vm.PropertyChanged += OnVmPropertyChanged;
        _vm.DetailOpened += OnDetailOpened;
        _vm.SetWindowWidth(Bounds.Width > 0 ? Bounds.Width : Width);
    }

    private void OnVmPropertyChanged(object? sender, PropertyChangedEventArgs e)
    {
        // SEARCH autofocuses its input, like the Electron view.
        if (e.PropertyName == nameof(MainViewModel.CurrentView) && _vm?.IsSearchView == true)
            Dispatcher.UIThread.Post(() => SearchBox.Focus(), DispatcherPriority.Loaded);
        if (e.PropertyName == nameof(MainViewModel.IsCreatePlaylistOpen) && _vm?.IsCreatePlaylistOpen == true)
            Dispatcher.UIThread.Post(() => NewPlaylistBox.Focus(), DispatcherPriority.Loaded);
    }

    private void OnDetailOpened()
    {
        Dispatcher.UIThread.Post(() =>
        {
            DetailScroll.Offset = default;
        }, DispatcherPriority.Loaded);
    }

    private void OnTitleBarPointerPressed(object? sender, PointerPressedEventArgs e)
    {
        if (!e.GetCurrentPoint(this).Properties.IsLeftButtonPressed) return;
        if (e.Source is Visual v && v.FindLogicalAncestorOfType<Button>(includeSelf: true) is not null) return;
        if (e.ClickCount == 2) ToggleMaximize();
        else BeginMoveDrag(e);
    }

    private void OnMinimize(object? sender, RoutedEventArgs e) => WindowState = WindowState.Minimized;
    private void OnMaximize(object? sender, RoutedEventArgs e) => ToggleMaximize();
    private void OnClose(object? sender, RoutedEventArgs e) => Close();
    private void ToggleMaximize() => WindowState = WindowState == WindowState.Maximized ? WindowState.Normal : WindowState.Maximized;

    /// <summary>Closes the dropdown that hosts a menu button once an entry is picked.</summary>
    private void CloseFlyout(object? sender, RoutedEventArgs e)
    {
        if (sender is Control c && c.FindLogicalAncestorOfType<Popup>() is { } popup) popup.Close();
    }

    // ---------- ADD MUSIC / drag and drop ----------

    private async void OnScanFolder(object? sender, RoutedEventArgs e)
    {
        CloseFlyout(sender, e);
        if (_vm is null) return;
        var folders = await StorageProvider.OpenFolderPickerAsync(new FolderPickerOpenOptions { Title = "SCAN_FOLDER", AllowMultiple = true });
        var paths = folders.Select(f => f.TryGetLocalPath()).OfType<string>().ToList();
        await _vm.ScanPathsAsync(paths);
    }

    private async void OnScanFiles(object? sender, RoutedEventArgs e)
    {
        CloseFlyout(sender, e);
        if (_vm is null) return;
        var files = await StorageProvider.OpenFilePickerAsync(new FilePickerOpenOptions
        {
            Title = "SCAN_FILES",
            AllowMultiple = true,
            FileTypeFilter = [new FilePickerFileType("Audio") { Patterns = Scanner.AudioExtensions.Select(x => "*" + x).ToList() }],
        });
        var paths = files.Select(f => f.TryGetLocalPath()).OfType<string>().ToList();
        await _vm.ScanPathsAsync(paths);
    }

    private void OnDragOver(object? sender, DragEventArgs e)
    {
        e.DragEffects = e.DataTransfer.Contains(DataFormat.File) ? DragDropEffects.Copy : DragDropEffects.None;
    }

    private async void OnDrop(object? sender, DragEventArgs e)
    {
        if (_vm is null) return;
        var items = e.DataTransfer.TryGetFiles();
        if (items is null) return;
        var paths = items.Select(i => i.TryGetLocalPath()).OfType<string>().ToList();
        await _vm.ScanPathsAsync(paths);
    }

    // ---------- Right-click menus ----------

    private void ShowMenu(Control target, MenuVm menu)
    {
        var flyout = new Flyout
        {
            Placement = PlacementMode.Pointer,
            Content = new ContentControl { Content = menu, ContentTemplate = (IDataTemplate)Resources["MenuTemplate"]! },
        };
        flyout.FlyoutPresenterClasses.Add("retro");
        flyout.ShowAt(target, showAtPointer: true);
    }

    private void OnCardContextRequested(object? sender, ContextRequestedEventArgs e)
    {
        if (_vm is null || sender is not Control c) return;
        switch (c.DataContext)
        {
            case AlbumItem a: ShowMenu(c, _vm.AlbumMenu(a)); break;
            case TrackCard t: ShowMenu(c, _vm.TrackMenu(t.Track)); break;
            default: return;
        }
        e.Handled = true;
    }

    private void OnTrackContextRequested(object? sender, ContextRequestedEventArgs e)
    {
        if (_vm is null || sender is not Control { DataContext: TrackRow row } c) return;
        ShowMenu(c, _vm.TrackMenu(row.Track));
        e.Handled = true;
    }

    // ---------- Modals ----------

    private void OnCreateBackdropPressed(object? sender, PointerPressedEventArgs e)
    {
        if (_vm is not null) _vm.IsCreatePlaylistOpen = false;
    }

    private void OnConfirmBackdropPressed(object? sender, PointerPressedEventArgs e)
    {
        if (_vm is not null) _vm.ConfirmDialog = null;
    }

    private void OnNewPlaylistKeyDown(object? sender, KeyEventArgs e)
    {
        if (_vm is null) return;
        if (e.Key == Key.Enter) { _vm.ConfirmCreatePlaylistCommand.Execute(null); e.Handled = true; }
        else if (e.Key == Key.Escape) { _vm.IsCreatePlaylistOpen = false; e.Handled = true; }
    }

    private void OnModalBackdropPressed(object? sender, PointerPressedEventArgs e)
    {
        if (_vm is not null) _vm.IsManualOpen = false;
    }

    private void OnModalBoxPressed(object? sender, PointerPressedEventArgs e) => e.Handled = true;

    private void OnKeyDown(object? sender, KeyEventArgs e)
    {
        if (_vm is null || Player is null || e.Source is TextBox) return;
        if (e.KeyModifiers == KeyModifiers.Control)
        {
            switch (e.Key)
            {
                case Key.S: Player.IsShuffle = !Player.IsShuffle; e.Handled = true; break;
                case Key.R: Player.CycleRepeatCommand.Execute(null); e.Handled = true; break;
            }
            return;
        }
        if (e.KeyModifiers != KeyModifiers.None) return;
        switch (e.Key)
        {
            case Key.Space: Player.TogglePlayCommand.Execute(null); e.Handled = true; break;
            case Key.Right: Player.NextCommand.Execute(null); e.Handled = true; break;
            case Key.Left: Player.PrevCommand.Execute(null); e.Handled = true; break;
            case Key.M: Player.IsMuted = !Player.IsMuted; break;
            case Key.F: _vm.ToggleFullScreenCommand.Execute(null); break;
            case Key.K: Player.Volume = Math.Min(1, Math.Round(Player.Volume * 15 + 1) / 15); break;
            case Key.J: Player.Volume = Math.Max(0, Math.Round(Player.Volume * 15 - 1) / 15); break;
            case Key.Escape: _vm.Escape(); break;
        }
    }
}
