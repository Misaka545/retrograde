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
            MediaControls.TryAttach(_window, _vm.Player);

            // A second launch hands over here: show the window and scan any paths it was given.
            SingleInstance.Listen(paths => Dispatcher.UIThread.Post(() =>
            {
                _window.Reveal();
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
        if (paths.Count > 0) await vm.ScanPathsAsync(paths);
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
        show.Click += (_, _) => _window.Reveal();
        var rescan = new NativeMenuItem("Rescan library");
        rescan.Click += (_, _) => _vm.RescanLibraryCommand.Execute(null);
        var import = new NativeMenuItem("Import favorites and playlists from the Electron build");
        import.Click += (_, _) => _vm.ImportFromElectronCommand.Execute(null);
        var exit = new NativeMenuItem("Exit");
        exit.Click += (_, _) => Quit();

        var tray = new TrayIcon
        {
            Icon = new WindowIcon(AssetLoader.Open(new Uri("avares://Retrograde/Assets/icon.ico"))),
            ToolTipText = "Retrograde",
            Menu = new NativeMenu { show, rescan, import, new NativeMenuItemSeparator(), minimize, new NativeMenuItemSeparator(), exit },
        };
        // Left click toggles the window, like tray.on('click').
        tray.Clicked += (_, _) =>
        {
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
        MediaControls.Detach();
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
