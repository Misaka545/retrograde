using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.ApplicationLifetimes;
using Avalonia.Markup.Xaml;
using Avalonia.Platform;
using Avalonia.Threading;
using Retrograde.App.Services;
using Retrograde.App.ViewModels;
using Retrograde.App.Views;

namespace Retrograde.App;

public partial class App : Application
{
    private MainWindow? _window;
    private MiniPlayerWindow? _mini;
    private MainViewModel? _vm;
    private IClassicDesktopStyleApplicationLifetime? _desktop;
    private bool _quitting;
    private long _uiErrorWindowStart;
    private int _uiErrorsInWindow;

    public override void Initialize() => AvaloniaXamlLoader.Load(this);

    public override void OnFrameworkInitializationCompleted()
    {
        if (ApplicationLifetime is IClassicDesktopStyleApplicationLifetime desktop)
        {
            _desktop = desktop;
            Dispatcher.UIThread.UnhandledException += OnUiException;
            _vm = new MainViewModel();
            _window = new MainWindow { DataContext = _vm };
            desktop.MainWindow = _window;
            desktop.ShutdownRequested += (_, _) => Quit(); // OS logoff / shutdown
            _vm.HideRequested += () => _window.Hide();
            _vm.QuitRequested += Quit;
            CreateTray();
            Mpris.Start(_vm.Player, () => Dispatcher.UIThread.Post(() => _window.Reveal()), Quit);
            AccentTheme.Init(_vm.AccentFromCover);
            _vm.Player.TrackChanged += t => AccentTheme.Follow(t?.CoverFullPath ?? t?.CoverThumbPath);
            _vm.PropertyChanged += (_, e) =>
            {
                if (e.PropertyName == nameof(MainViewModel.IsMiniPlayer)) ShowMini(_vm.IsMiniPlayer);
                else if (e.PropertyName == nameof(MainViewModel.AccentFromCover)) AccentTheme.SetEnabled(_vm.AccentFromCover);
            };

            // A second launch hands over here: show the window and scan any paths it was given.
            SingleInstance.Listen(paths => Dispatcher.UIThread.Post(() =>
            {
                RevealMain();
                var valid = paths.Where(p => File.Exists(p) || Directory.Exists(p)).ToList();
                if (valid.Count > 0) _ = _vm.ScanPathsAsync(valid);
            }));

            // Files / folders passed on the command line (e.g. "Open with") are scanned into the library.
            var startPaths = (desktop.Args ?? []).Where(a => File.Exists(a) || Directory.Exists(a)).ToList();
            _ = StartAsync(_vm, startPaths);
        }
        base.OnFrameworkInitializationCompleted();
    }

    /// <summary>
    /// An exception escaping a click or command handler is logged and shown instead of ending the app
    /// mid-song. A burst of them means something is broken for good, so those are left to crash.
    /// </summary>
    private void OnUiException(object? sender, DispatcherUnhandledExceptionEventArgs e)
    {
        Retrograde.Core.ErrorLog.Write("ui", e.Exception);
        var now = Environment.TickCount64;
        if (now - _uiErrorWindowStart > 10_000)
        {
            _uiErrorWindowStart = now;
            _uiErrorsInWindow = 0;
        }
        if (++_uiErrorsInWindow > 5) return;
        e.Handled = true;
        try { _vm?.ShowToast($"INTERNAL_ERROR // {e.Exception.Message}", ToastKind.Error); } catch { }
    }

    private static async Task StartAsync(MainViewModel vm, IReadOnlyList<string> paths)
    {
        await vm.InitializeAsync();
        // The restored track's colour (TrackChanged fired before the theme was listening, or not at all)
        var t = vm.Player.CurrentTrack;
        AccentTheme.Follow(t?.CoverFullPath ?? t?.CoverThumbPath);
        if (paths.Count > 0) await vm.ScanPathsAsync(paths);
    }

    /// <summary>Mini player on: the small window shows and the main one hides; off: the other way round.</summary>
    private void ShowMini(bool on)
    {
        if (_window is null || _vm is null) return;
        if (on)
        {
            if (_mini is null)
            {
                _mini = new MiniPlayerWindow { DataContext = _vm };
                _mini.Closed += (_, _) =>
                {
                    _mini = null;
                    if (_vm.IsMiniPlayer && !_quitting) _vm.IsMiniPlayer = false;
                };
            }
            _vm.Player.IsUiActive = true;
            _mini.Show();
            // No tooltip of the hidden window may open (or stay) on screen: the pointer is still over the button
            ToolTip.SetServiceEnabled(_window, false);
            _window.CloseToolTips();
            _window.Hide();
        }
        else
        {
            var mini = _mini;
            _mini = null;
            mini?.Close();
            ToolTip.SetServiceEnabled(_window, true);
            if (!_quitting) _window.Reveal();
        }
    }

    /// <summary>Tray "Show Player" and a second launch: back to the full window, from the mini player too.</summary>
    private void RevealMain()
    {
        if (_vm is { IsMiniPlayer: true }) _vm.IsMiniPlayer = false;
        else _window?.Reveal();
    }

    /// <summary>Tray icon and menu, same entries as the Electron build.</summary>
    private void CreateTray()
    {
        if (_vm is null || _window is null) return;
        var minimize = new NativeMenuItem("Minimize to tray on close")
        {
            ToggleType = MenuItemToggleType.CheckBox,
            IsChecked = _vm.MinimizeToTrayChecked,
        };
        minimize.Click += (_, _) =>
        {
            _vm.MinimizeToTrayChecked = !_vm.MinimizeToTrayChecked;
            minimize.IsChecked = _vm.MinimizeToTrayChecked;
        };
        _vm.PropertyChanged += (_, e) =>
        {
            if (e.PropertyName == nameof(MainViewModel.MinimizeToTrayChecked)) minimize.IsChecked = _vm.MinimizeToTrayChecked;
        };
        var show = new NativeMenuItem("Show Player");
        show.Click += (_, _) => RevealMain();
        var mini = new NativeMenuItem("Mini player (Ctrl+M)");
        mini.Click += (_, _) => _vm.ToggleMiniPlayerCommand.Execute(null);
        var accent = new NativeMenuItem("Accent colour from the cover")
        {
            ToggleType = MenuItemToggleType.CheckBox,
            IsChecked = _vm.AccentFromCover,
        };
        accent.Click += (_, _) =>
        {
            _vm.AccentFromCover = !_vm.AccentFromCover;
            accent.IsChecked = _vm.AccentFromCover;
        };
        var rescan = new NativeMenuItem("Rescan library");
        rescan.Click += (_, _) => _vm.RescanLibraryCommand.Execute(null);
        var watch = new NativeMenuItem("Watch library folders for new files")
        {
            ToggleType = MenuItemToggleType.CheckBox,
            IsChecked = _vm.IsWatchingFolders,
        };
        watch.Click += (_, _) =>
        {
            _vm.SetWatchFolders(!_vm.IsWatchingFolders);
            watch.IsChecked = _vm.IsWatchingFolders;
        };
        var exit = new NativeMenuItem("Exit");
        exit.Click += (_, _) => Quit();

        var tray = new TrayIcon
        {
            Icon = new WindowIcon(AssetLoader.Open(new Uri("avares://Retrograde/Assets/icon.ico"))),
            ToolTipText = "Retrograde",
            Menu = new NativeMenu { show, mini, rescan, new NativeMenuItemSeparator(), minimize, watch, accent, new NativeMenuItemSeparator(), exit },
        };
        // Left click toggles the window, like tray.on('click').
        tray.Clicked += (_, _) =>
        {
            if (_vm.IsMiniPlayer) { RevealMain(); return; }
            if (_window.IsVisible && _window.WindowState != WindowState.Minimized) _window.Hide();
            else _window.Reveal();
        };
        TrayIcon.SetIcons(this, [tray]);
    }

    private void Quit()
    {
        if (_quitting) return;
        _quitting = true;
        _vm?.Shutdown();               // saves settings, stops mpv
        Mpris.Stop();
        _mini?.Close();
        if (_window is not null)
        {
            _window.AllowClose = true;
            _window.Close();
        }
        if (TrayIcon.GetIcons(this) is { } icons)
            foreach (var t in icons) t.IsVisible = false; // remove the icon now instead of leaving a ghost
        _desktop?.Shutdown();
    }
}
