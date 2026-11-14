using Retrograde.Core;

namespace Retrograde.App.ViewModels;

/// <summary>
/// Shuffle plays the whole library: turning it on keeps the current track and puts every other library
/// track behind it in random order; turning it off goes back to the queue that was playing before
/// (usually the album), at the current track. So crossfade, which leaves an album played in order
/// gapless, has tracks of different albums to fade between.
/// </summary>
public partial class PlayerViewModel
{
    /// <summary>Every track of the library (set by the main view model).</summary>
    public Func<IReadOnlyList<Track>>? LibraryTracks { get; set; }

    /// <summary>The album a track belongs to, for turning shuffle off after a restart (set by the main view model).</summary>
    public Func<Track, IReadOnlyList<Track>?>? AlbumOf { get; set; }

    /// <summary>The queue before shuffle spread it over the library; null while not shuffling (or after a restart).</summary>
    private List<Track>? _unshuffledQueue;

    /// <summary>Session restore turns shuffle on: the saved queue is the library-wide one already.</summary>
    private bool _restoringShuffle;

    private static bool SameTrack(Track a, Track b) =>
        ReferenceEquals(a, b) || string.Equals(a.Id, b.Id, StringComparison.OrdinalIgnoreCase);

    private Track? CurrentQueueTrack => _index >= 0 && _index < _queue.Count ? _queue[_index] : null;

    /// <summary>The current track first, then every other library track (and queued tracks the library lacks).</summary>
    private void SpreadOverLibrary()
    {
        if (CurrentQueueTrack is not { } current) return; // nothing playing: nothing to shuffle around
        if (LibraryTracks?.Invoke() is not { Count: > 0 } all) return;
        _unshuffledQueue = _queue.ToList();
        var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase) { current.Id };
        var list = new List<Track>(all.Count + 1) { current };
        foreach (var t in all)
            if (seen.Add(t.Id)) list.Add(t);
        foreach (var t in _queue)
            if (seen.Add(t.Id)) list.Add(t);
        _queue = list;
        _index = 0;
    }

    /// <summary>Back to the queue from before shuffle, at the track now playing.</summary>
    private void ReturnFromLibrary()
    {
        var back = _unshuffledQueue;
        _unshuffledQueue = null;
        if (CurrentQueueTrack is not { } current)
        {
            if (back is not null) { _queue = back; _index = -1; }
            return;
        }
        if (back is null)
        {
            // After a restart the queue from before is not known. A queue as big as the library is the
            // shuffle one: go to the current track's album. A smaller one was the user's own: keep it.
            var libraryCount = LibraryTracks?.Invoke().Count ?? int.MaxValue;
            if (_queue.Count < libraryCount) return;
        }
        var idx = back?.FindIndex(t => SameTrack(t, current)) ?? -1;
        if (idx < 0 && AlbumOf?.Invoke(current) is { Count: > 0 } album)
        {
            back = album.ToList();
            idx = back.FindIndex(t => SameTrack(t, current));
        }
        if (back is null || idx < 0)
        {
            back = [current];
            idx = 0;
        }
        _queue = back;
        _index = idx;
    }

    // ---------- Keeping the queue from before in step with queue edits made while shuffling ----------

    private void KeepUnshuffledAppend(IReadOnlyList<Track> tracks) => _unshuffledQueue?.AddRange(tracks);

    private void KeepUnshuffledPlayNext(IReadOnlyList<Track> tracks)
    {
        if (_unshuffledQueue is null || CurrentQueueTrack is not { } current) return;
        var at = _unshuffledQueue.FindIndex(t => SameTrack(t, current));
        _unshuffledQueue.InsertRange(at < 0 ? _unshuffledQueue.Count : at + 1, tracks);
    }

    private void KeepUnshuffledRemove(Track track)
    {
        var at = _unshuffledQueue?.FindIndex(t => ReferenceEquals(t, track)) ?? -1;
        if (at >= 0) _unshuffledQueue!.RemoveAt(at);
    }
}
