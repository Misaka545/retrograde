using Avalonia.Threading;
using Retrograde.Core;

namespace Retrograde.App.ViewModels;

/// <summary>
/// Keeps the playback session across runs: the queue, the current track and its position come back
/// paused on the next start (playback_session in the Electron build).
/// </summary>
public partial class MainViewModel
{
    private DispatcherTimer? _sessionTimer;
    private bool _queueDirty;
    private (string? Id, int Index, long Second, bool Shuffle, int Repeat) _savedState = (null, -1, 0, false, 0);

    /// <summary>Called once the library is loaded, so the saved track ids can be resolved.</summary>
    private void RestoreSession()
    {
        if (SessionStore.Load() is { } saved)
        {
            // Tracks that left the library since are dropped; the current one is found again by id.
            var queue = new List<Track>(saved.Queue.Count);
            var index = -1;
            for (int i = 0; i < saved.Queue.Count; i++)
            {
                if (!_tracksById.TryGetValue(saved.Queue[i], out var e)) continue;
                if (i == saved.State.Index) index = queue.Count;
                queue.Add(e.Track);
            }
            // The saved index wins (a track can be queued twice); the id is the cross-check and the fallback.
            if (saved.State.CurrentId is { } id)
            {
                if (index >= 0 && !string.Equals(queue[index].Id, id, StringComparison.OrdinalIgnoreCase)) index = -1;
                if (index < 0) index = queue.FindIndex(t => string.Equals(t.Id, id, StringComparison.OrdinalIgnoreCase));
            }
            Player.RestoreSession(queue, index, index >= 0 ? saved.State.Position : 0, saved.State.Shuffle, saved.State.RepeatMode);
        }

        Player.QueueChanged += () => _queueDirty = true;
        Player.TrackChanged += _ => SaveSessionState();
        // Position, shuffle and repeat are picked up here; the files are only written when something moved.
        _sessionTimer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(5) };
        _sessionTimer.Tick += (_, _) => SaveSession(background: true);
        _sessionTimer.Start();
    }

    private void SaveSession(bool background)
    {
        if (_queueDirty)
        {
            _queueDirty = false;
            var ids = Player.Queue.Select(t => t.Id).ToList();
            if (background) Task.Run(() => SessionStore.SaveQueue(ids));
            else SessionStore.SaveQueue(ids);
        }
        SaveSessionState();
    }

    private void SaveSessionState()
    {
        var position = Player.PlaybackPosition;
        var now = (Id: Player.CurrentTrack?.Id, Index: Player.QueueIndex, Second: (long)position, Shuffle: Player.IsShuffle, Repeat: Player.RepeatMode);
        if (now == _savedState) return;
        _savedState = now;
        SessionStore.SaveState(new SessionState
        {
            CurrentId = now.Id,
            Index = now.Index,
            Position = position,
            Shuffle = now.Shuffle,
            RepeatMode = now.Repeat,
        });
    }
}
