using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;

namespace Retrograde.App.ViewModels;

/// <summary>Close button / tray behaviour (handleWindowClose and the BACKGROUND_PLAY prompt in the Electron build).</summary>
public partial class MainViewModel
{
    [ObservableProperty] private bool _isBackgroundPromptOpen;
    [ObservableProperty] private bool _rememberCloseChoice;
    [ObservableProperty] private bool _minimizeToTrayChecked;

    /// <summary>The view hides the window to the tray.</summary>
    public event Action? HideRequested;
    /// <summary>The app shuts down (mpv is stopped by Shutdown()).</summary>
    public event Action? QuitRequested;

    #pragma warning disable MVVMTK0034 // set the backing field: no change notification or save during construction
    private void InitWindowSettings() => _minimizeToTrayChecked = _settings.MinimizeToTray != false;
#pragma warning restore MVVMTK0034

    /// <summary>Window close button / Alt+F4.</summary>
    public void RequestClose()
    {
        switch (_settings.MinimizeToTray)
        {
            case null:
                RememberCloseChoice = false;
                IsBackgroundPromptOpen = true;
                break;
            case true:
                HideRequested?.Invoke();
                break;
            default:
                QuitRequested?.Invoke();
                break;
        }
    }

    [RelayCommand] private void ChooseMinimize() => ResolveClose(true);
    [RelayCommand] private void ChooseQuit() => ResolveClose(false);

    private void ResolveClose(bool minimize)
    {
        IsBackgroundPromptOpen = false;
        if (RememberCloseChoice)
        {
            _settings.MinimizeToTray = minimize;
            MinimizeToTrayChecked = minimize;
            _settings.Save();
        }
        if (minimize) HideRequested?.Invoke();
        else QuitRequested?.Invoke();
    }

    /// <summary>Tray menu "Minimize to tray on close" checkbox.</summary>
    partial void OnMinimizeToTrayCheckedChanged(bool value)
    {
        if (_settings.MinimizeToTray == value) return;
        _settings.MinimizeToTray = value;
        _settings.Save();
    }

    [RelayCommand] private void Quit() => QuitRequested?.Invoke();

    // ---------- Full-screen player and queue popup ----------

    [ObservableProperty] private bool _isFullScreen;
    [ObservableProperty] private bool _isQueueOpen;

    // The queue rows are only kept up to date while one of the two views that show them is open.
    // Closing: the rows stay until the fold-back animation has finished (FullScreenHidden).
    partial void OnIsFullScreenChanged(bool value)
    {
        if (value) Player.QueueViewsActive = true;
    }

    /// <summary>The view calls this once the full-screen player is gone from the screen.</summary>
    public void FullScreenHidden() => Player.QueueViewsActive = IsFullScreen || IsQueueOpen;
    partial void OnIsQueueOpenChanged(bool value) => Player.QueueViewsActive = IsFullScreen || IsQueueOpen;

    [RelayCommand] private void ToggleQueue() => IsQueueOpen = !IsQueueOpen;
    [RelayCommand] private void CloseQueue() => IsQueueOpen = false;
    [RelayCommand] private void OpenFullScreen() { IsQueueOpen = false; IsFullScreen = true; }
    [RelayCommand] private void CloseFullScreen() => IsFullScreen = false;
    [RelayCommand] private void ToggleFullScreen() { if (IsFullScreen) IsFullScreen = false; else OpenFullScreen(); }

    /// <summary>"Go to album" from the full-screen header closes it first (handleOpenCurrentAlbum).</summary>
    [RelayCommand]
    private void OpenCurrentAlbumFromFullScreen()
    {
        IsFullScreen = false;
        OpenCurrentAlbumCommand.Execute(null);
    }
}
