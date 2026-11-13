using System.ComponentModel;
using Avalonia.Controls;
using Avalonia.Threading;
using Retrograde.App.ViewModels;
using Windows.Media;
using Windows.Storage;
using Windows.Storage.Streams;

namespace Retrograde.App.Services;

/// <summary>
/// Windows System Media Transport Controls (the navigator.mediaSession part of the Electron build):
/// media keys, the volume-flyout / lock-screen card with title, artist, album and cover, and seeking
/// from the timeline. Bound to the main window handle, so no MediaPlayer is created.
/// </summary>
public static class MediaControls
{
    private static SystemMediaTransportControls? _smtc;
    private static PlayerViewModel? _player;
    private static string? _trackId;
    private static double _syncedPosition;
    private static long _syncedAt;

    public static void TryAttach(Window window, PlayerViewModel player)
    {
        if (!OperatingSystem.IsWindowsVersionAtLeast(10, 0, 10240)) return;
        _player = player;
        window.Opened += (_, _) =>
        {
            try
            {
                var hwnd = window.TryGetPlatformHandle()?.Handle ?? IntPtr.Zero;
                if (hwnd == IntPtr.Zero) return;
                _smtc = SystemMediaTransportControlsInterop.GetForWindow(hwnd);
                _smtc.IsEnabled = false; // shown once something plays
                _smtc.IsPlayEnabled = _smtc.IsPauseEnabled = _smtc.IsNextEnabled = _smtc.IsPreviousEnabled = true;
                _smtc.ButtonPressed += OnButtonPressed;
                _smtc.PlaybackPositionChangeRequested += OnSeekRequested;
                player.PropertyChanged += OnPlayerChanged;
            }
            catch (Exception ex)
            {
                System.Diagnostics.Trace.WriteLine($"SMTC unavailable: {ex.Message}");
                _smtc = null;
            }
        };
    }

    public static void Detach()
    {
        if (_smtc is null) return;
        try
        {
            _smtc.ButtonPressed -= OnButtonPressed;
            _smtc.PlaybackPositionChangeRequested -= OnSeekRequested;
            _smtc.DisplayUpdater.ClearAll();
            _smtc.IsEnabled = false;
        }
        catch { }
        if (_player is not null) _player.PropertyChanged -= OnPlayerChanged;
        _smtc = null;
    }

    private static void OnButtonPressed(SystemMediaTransportControls sender, SystemMediaTransportControlsButtonPressedEventArgs args)
    {
        Dispatcher.UIThread.Post(() =>
        {
            var p = _player;
            if (p is null) return;
            switch (args.Button)
            {
                case SystemMediaTransportControlsButton.Play when !p.IsPlaying:
                case SystemMediaTransportControlsButton.Pause when p.IsPlaying:
                    p.TogglePlayCommand.Execute(null);
                    break;
                case SystemMediaTransportControlsButton.Next: p.NextCommand.Execute(null); break;
                case SystemMediaTransportControlsButton.Previous: p.PrevCommand.Execute(null); break;
            }
        });
    }

    private static void OnSeekRequested(SystemMediaTransportControls sender, PlaybackPositionChangeRequestedEventArgs args)
    {
        var seconds = args.RequestedPlaybackPosition.TotalSeconds;
        Dispatcher.UIThread.Post(async () =>
        {
            if (_player is null) return;
            await _player.SeekAsync(seconds);
            UpdateTimeline();
        });
    }

    private static void OnPlayerChanged(object? sender, PropertyChangedEventArgs e)
    {
        if (_smtc is null || _player is null) return;
        switch (e.PropertyName)
        {
            case nameof(PlayerViewModel.CurrentTrack):
                _ = UpdateMetadataAsync();
                break;
            case nameof(PlayerViewModel.IsPlaying):
                _smtc.PlaybackStatus = _player.IsPlaying ? MediaPlaybackStatus.Playing : MediaPlaybackStatus.Paused;
                UpdateTimeline();
                break;
            case nameof(PlayerViewModel.Duration):
                UpdateTimeline();
                break;
            case nameof(PlayerViewModel.Position):
                // Windows extrapolates while playing; only resync after a jump (seek, track restart).
                var expected = _syncedPosition + (_player.IsPlaying ? (Environment.TickCount64 - _syncedAt) / 1000.0 : 0);
                if (Math.Abs(_player.Position - expected) > 2) UpdateTimeline();
                break;
        }
    }

    private static void UpdateTimeline()
    {
        if (_smtc is null || _player is null || _player.CurrentTrack is null) return;
        var duration = TimeSpan.FromSeconds(Math.Max(0, _player.Duration));
        var position = TimeSpan.FromSeconds(Math.Clamp(_player.Position, 0, Math.Max(0, _player.Duration)));
        _smtc.UpdateTimelineProperties(new SystemMediaTransportControlsTimelineProperties
        {
            StartTime = TimeSpan.Zero,
            EndTime = duration,
            MinSeekTime = TimeSpan.Zero,
            MaxSeekTime = duration,
            Position = position,
        });
        _syncedPosition = position.TotalSeconds;
        _syncedAt = Environment.TickCount64;
    }

    private static async Task UpdateMetadataAsync()
    {
        var smtc = _smtc;
        var track = _player?.CurrentTrack;
        if (smtc is null) return;
        if (track is null)
        {
            smtc.DisplayUpdater.ClearAll();
            smtc.IsEnabled = false;
            _trackId = null;
            return;
        }
        if (track.Id == _trackId) return;
        _trackId = track.Id;
        smtc.IsEnabled = true;
        var updater = smtc.DisplayUpdater;
        updater.Type = MediaPlaybackType.Music;
        updater.MusicProperties.Title = track.Title;
        updater.MusicProperties.Artist = string.IsNullOrEmpty(track.Artist) ? "Unknown Artist" : track.Artist;
        updater.MusicProperties.AlbumTitle = string.IsNullOrEmpty(track.Album) ? "Unknown Album" : track.Album;
        updater.Thumbnail = null;
        var cover = track.CoverFullPath ?? track.CoverThumbPath;
        if (cover is not null && File.Exists(cover))
        {
            try
            {
                var file = await StorageFile.GetFileFromPathAsync(cover);
                if (_trackId != track.Id) return; // a newer track won the race
                updater.Thumbnail = RandomAccessStreamReference.CreateFromFile(file);
            }
            catch { /* no artwork */ }
        }
        updater.Update();
        UpdateTimeline();
    }
}
