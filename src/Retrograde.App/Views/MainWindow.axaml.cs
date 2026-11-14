using System.ComponentModel;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.Primitives;
using Avalonia.Controls.Templates;
using Avalonia.Platform.Storage;
using Retrograde.Core;
using Avalonia.Input;
using Avalonia.Media;
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
        // Space is play/pause wherever the focus is, as in the Electron build. Without this a focused
        // button takes Space as a click: after opening full screen with the mouse and closing it,
        // Space opened it again instead of pausing.
        AddHandler(KeyDownEvent, (_, e) =>
        {
            if (e.Key != Key.Space || e.KeyModifiers != KeyModifiers.None || e.Source is TextBox || Player is null) return;
            Player.TogglePlayCommand.Execute(null);
            e.Handled = true;
        }, RoutingStrategies.Tunnel);
        InitFullScreen();
        // Window.idle pauses the looping animations while the window is not in front (app-idle in the Electron build).
        Activated += (_, _) => UpdateIdle();
        Deactivated += (_, _) => UpdateIdle();
        UpdateIdle();
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
            // Queue popup: above its button, at most 60% of the window and 520px, so it stays a popup next
            // to the player bar instead of a column as tall as the window.
            Dispatcher.UIThread.Post(PlaceQueuePopup, DispatcherPriority.Loaded);
        };
        InitQueueDrag();
        // Queue popup closes on any press outside it (the press still reaches its target).
        AddHandler(PointerPressedEvent, (_, e) =>
        {
            if (_vm is not { IsQueueOpen: true } || e.Source is not Visual v) return;
            if (QueuePopup.IsVisualAncestorOf(v) || QueueButton.IsVisualAncestorOf(v) || v == QueueButton) return;
            _vm.IsQueueOpen = false;
        }, RoutingStrategies.Tunnel, handledEventsToo: true);
        PlayerGrid.SizeChanged += (_, _) => BalancePlayerBar();
        Extras.SizeChanged += (_, _) => BalancePlayerBar();
    }

    /// <summary>
    /// Transport in the middle of the window while both sides have room for the extras; in a narrower window
    /// the extras keep their width and the track info gets what is left.
    /// </summary>
    private void BalancePlayerBar()
    {
        var cols = PlayerGrid.ColumnDefinitions;
        var side = (PlayerGrid.Bounds.Width - Transport.DesiredSize.Width - cols[1].ActualWidth - cols[3].ActualWidth) / 2;
        // DesiredSize is cut to the space given, so add up the children: a horizontal StackPanel measures them unconstrained
        var natural = Extras.Children.Sum(c => c.DesiredSize.Width) + Extras.Spacing * Math.Max(0, Extras.Children.Count - 1);
        var centred = side >= natural;
        if (centred != cols[4].Width.IsStar)
            cols[4].Width = centred ? new GridLength(1, GridUnitType.Star) : GridLength.Auto;
    }

    private PlayerViewModel? Player => _vm?.Player;

    // RETRO_ANIM=always keeps looping animations running in the background too (dev: measure their CPU cost).
    private static readonly bool AnimateAlways = Environment.GetEnvironmentVariable("RETRO_ANIM") == "always";

    private void UpdateIdle() => Classes.Set("idle", !AnimateAlways && (!IsActive || !IsVisible || WindowState == WindowState.Minimized));

    /// <summary>Set by the app when it is really shutting down, so Closing is not intercepted.</summary>
    public bool AllowClose { get; set; }

    protected override void OnPropertyChanged(AvaloniaPropertyChangedEventArgs change)
    {
        base.OnPropertyChanged(change);
        if (change.Property == IsVisibleProperty || change.Property == WindowStateProperty)
        {
            // The mini player shows the position while this window is hidden
            if (Player is not null) Player.IsUiActive = (IsVisible && WindowState != WindowState.Minimized) || _vm?.IsMiniPlayer == true;
            if (!IsVisible || WindowState == WindowState.Minimized) SkipFullScreenAnimation();
            UpdateIdle();
        }
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
        InitLyrics(_vm.Player);
    }

    private void OnVmPropertyChanged(object? sender, PropertyChangedEventArgs e)
    {
        if (e.PropertyName == nameof(MainViewModel.IsFullScreen) && _vm is not null)
        {
            AnimateFullScreen(_vm.IsFullScreen);
            return;
        }
        // The player bar's buttons move with the window size and the sidebar: place the popup on opening
        if (e.PropertyName == nameof(MainViewModel.IsQueueOpen) && _vm is { IsQueueOpen: true })
            PlaceQueuePopup();
        // A tooltip left open while the sidebar changes size can stay behind as an empty box over the
        // nav and swallow its clicks: close them all.
        if (e.PropertyName == nameof(MainViewModel.IsSidebarVisible)) CloseToolTips(SidebarHost);

        // SEARCH autofocuses its input, like the Electron view.
        if (e.PropertyName == nameof(MainViewModel.CurrentView) && _vm?.IsSearchView == true)
            Dispatcher.UIThread.Post(() => SearchBox.Focus(), DispatcherPriority.Loaded);
        if (e.PropertyName == nameof(MainViewModel.IsCreatePlaylistOpen) && _vm?.IsCreatePlaylistOpen == true)
            Dispatcher.UIThread.Post(() => NewPlaylistBox.Focus(), DispatcherPriority.Loaded);
    }

    /// <summary>
    /// Closes the tooltips open under this visual. One left open while its window hides (the mini player
    /// button) would otherwise stay on screen by itself.
    /// </summary>
    public void CloseToolTips(Visual? root = null)
    {
        foreach (var c in (root ?? this).GetVisualDescendants().OfType<Control>())
            if (ToolTip.GetIsOpen(c)) ToolTip.SetIsOpen(c, false);
    }

    private void OnDetailOpened()
    {
        Dispatcher.UIThread.Post(() =>
        {
            if (DetailList.GetVisualDescendants().OfType<ScrollViewer>().FirstOrDefault() is { } sv) sv.Offset = default;
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
        // Click fires before the button runs its Command. Closing right away detaches the popup content,
        // its Command binding loses the DataContext, and the entry does nothing; so close afterwards.
        if (sender is Control c && c.FindLogicalAncestorOfType<Popup>() is { } popup)
            Dispatcher.UIThread.Post(() => popup.Close(), DispatcherPriority.Background);
    }

    // ---------- Sound popup: equalizer / crossfade, output device ----------

    private async void OnAudioFlyoutOpening(object? sender, EventArgs e)
    {
        AudioButton.Classes.Add("open");
        AlignPopup(sender as Flyout, AudioButton);
        if (Player is not null) await Player.RefreshAudioDevicesAsync();
    }

    private void OnAudioFlyoutClosed(object? sender, EventArgs e) => AudioButton.Classes.Remove("open");

    // ---------- Playlist cover ----------

    private async void OnChangeCover(object? sender, RoutedEventArgs e)
    {
        CloseFlyout(sender, e);
        if (_vm is null) return;
        var files = await StorageProvider.OpenFilePickerAsync(new FilePickerOpenOptions
        {
            Title = "CHANGE_COVER",
            AllowMultiple = false,
            FileTypeFilter = [new FilePickerFileType("Images") { Patterns = PlaylistCovers.Extensions.Select(x => "*" + x).ToList() }],
        });
        if (files.Count > 0 && files[0].TryGetLocalPath() is { } path) await _vm.SetCurrentPlaylistCoverAsync(path);
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
        e.Handled = true;
        if (_vm.IsSelectMode) return; // no menu while picking albums, as in LibraryGrid.jsx
        switch (c.DataContext)
        {
            case AlbumItem a: ShowMenu(c, _vm.AlbumMenu(a)); break;
            case TrackCard t: ShowMenu(c, _vm.TrackMenu(t.Track)); break;
            case PlaylistCard p: ShowMenu(c, _vm.PlaylistMenu(p.Playlist)); break;
            default: e.Handled = false; break;
        }
    }

    private void OnArtistLinkClick(object? sender, RoutedEventArgs e)
    {
        if (_vm is not null && sender is Control { DataContext: DetailHeaderRow h }) _vm.OpenArtistNamedCommand.Execute(h.FullArtist);
    }

    // ---------- Tag editor ----------

    private async void OnTagCoverPick(object? sender, RoutedEventArgs e)
    {
        if (_vm?.TagEditor is not { } editor) return;
        var files = await StorageProvider.OpenFilePickerAsync(new FilePickerOpenOptions
        {
            Title = "CHANGE_COVER",
            AllowMultiple = false,
            FileTypeFilter = [new FilePickerFileType("Images") { Patterns = ["*.jpg", "*.jpeg", "*.png"] }],
        });
        if (files.Count > 0 && files[0].TryGetLocalPath() is { } path) editor.PickCover(path);
    }

    private void OnTagBackdropPressed(object? sender, PointerPressedEventArgs e)
    {
        if (_vm is { TagEditor.IsSaving: false }) _vm.TagEditor = null;
    }

    private void OnSmartBackdropPressed(object? sender, PointerPressedEventArgs e)
    {
        if (_vm is not null) _vm.SmartEditor = null;
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
        if (_vm is null || Player is null) return;
        // Escape closes the editors from inside their text boxes too
        if (e.Key == Key.Escape && e.KeyModifiers == KeyModifiers.None && (_vm.TagEditor is not null || _vm.SmartEditor is not null))
        {
            _vm.Escape();
            e.Handled = true;
            return;
        }
        if (e.Source is TextBox) return;
        // ? (Shift + / on most layouts): the keyboard shortcuts
        if (e.Key == Key.OemQuestion && (e.KeyModifiers & ~KeyModifiers.Shift) == KeyModifiers.None)
        {
            _vm.IsManualOpen = !_vm.IsManualOpen;
            e.Handled = true;
            return;
        }
        if (e.KeyModifiers == KeyModifiers.Control)
        {
            switch (e.Key)
            {
                case Key.M: _vm.ToggleMiniPlayerCommand.Execute(null); e.Handled = true; break;
                case Key.S: Player.IsShuffle = !Player.IsShuffle; e.Handled = true; break;
                case Key.R: Player.CycleRepeatCommand.Execute(null); e.Handled = true; break;
                case Key.B: _vm.ToggleSidebarCommand.Execute(null); e.Handled = true; break;
            }
            return;
        }
        if (e.KeyModifiers != KeyModifiers.None) return;
        switch (e.Key)
        {
            case Key.Right: Player.NextCommand.Execute(null); e.Handled = true; break;
            case Key.Left: Player.PrevCommand.Execute(null); e.Handled = true; break;
            case Key.M: Player.IsMuted = !Player.IsMuted; break;
            case Key.F: _vm.ToggleFullScreenCommand.Execute(null); break;
            case Key.K: Player.Volume = Math.Min(1, Math.Round(Player.Volume * 15 + 1) / 15); break;
            case Key.J: Player.Volume = Math.Max(0, Math.Round(Player.Volume * 15 - 1) / 15); break;
            case Key.Escape: _vm.Escape(); break;
            case Key.F5: _vm.RescanLibraryCommand.Execute(null); e.Handled = true; break;
        }
    }
}
