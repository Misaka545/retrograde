using System.Collections.ObjectModel;
using Avalonia.Threading;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Retrograde.Audio;
using Retrograde.Core;

namespace Retrograde.App.ViewModels;

public partial class PlayerViewModel : ObservableObject
{
    private readonly MpvEngine _engine;
    private readonly AppSettings _settings;
    private readonly Random _rng = new();
    private List<Track> _queue = [];
    private int _index = -1;
    private Task? _startTask;
    private long _lastTimeUiTick;
    /// <summary>False until mpv holds the current track: after a failed load, or when mpv had to be restarted.</summary>
    private bool _fileLoaded;
    private int _loadFailures;
    /// <summary>Play order while shuffle is on: a permutation of the queue indices; _orderPos is the current track's place in it.</summary>
    private List<int> _order = [];
    private int _orderPos = -1;
    /// <summary>Bumped by every explicit load, so events about an earlier file are recognised as stale.</summary>
    private int _playGen;
    /// <summary>True from the start of an explicit load until mpv has been told about it.</summary>
    private bool _loading;
    /// <summary>Queue index of the track handed to mpv as "next" (gapless), or -1.</summary>
    private int _queuedNextIndex = -1;

    [ObservableProperty] [NotifyPropertyChangedFor(nameof(HasTrack), nameof(TitleText), nameof(ArtistText), nameof(QualityText), nameof(CoverPath), nameof(AlbumHeader))]
    private Track? _currentTrack;

    [ObservableProperty]
    private bool _isPlaying;

    [ObservableProperty] [NotifyPropertyChangedFor(nameof(PositionText))]
    private double _position;

    [ObservableProperty] [NotifyPropertyChangedFor(nameof(DurationText), nameof(SeekMax))]
    private double _duration;

    [ObservableProperty] [NotifyPropertyChangedFor(nameof(IsVolumeOff), nameof(IsVolumeLow), nameof(IsVolumeHigh))]
    private double _volume;
    [ObservableProperty] [NotifyPropertyChangedFor(nameof(IsVolumeOff), nameof(IsVolumeLow), nameof(IsVolumeHigh))]
    private bool _isMuted;
    [ObservableProperty] private bool _isExclusive;
    [ObservableProperty] private bool _isShuffle;
    [ObservableProperty] [NotifyPropertyChangedFor(nameof(IsRepeatOne), nameof(IsRepeatOn))]
    private int _repeatMode; // 0 off, 1 all, 2 one

    /// <summary>Set by the view while the user drags the seek slider.</summary>
    public bool IsSeeking { get; private set; }

    private volatile bool _isUiActive = true;
    private double _lastKnownPosition;

    /// <summary>False while the window is hidden or minimized; position updates are then skipped.</summary>
    public bool IsUiActive
    {
        get => _isUiActive;
        set
        {
            if (_isUiActive == value) return;
            _isUiActive = value;
            if (value && !IsSeeking) Position = _lastKnownPosition; // catch up when shown again
        }
    }
    public void SetSeeking(bool value) => IsSeeking = value;

    public bool HasTrack => CurrentTrack is not null;
    public string TitleText => CurrentTrack is null ? "NO_DATA" : Fmt.Upper(CurrentTrack.Title);
    public string ArtistText => CurrentTrack?.Artist ?? "UNKNOWN";
    public string QualityText => CurrentTrack?.QualityLabel ?? "";
    public string? CoverPath => CurrentTrack?.CoverThumbPath;
    public string AlbumHeader => string.IsNullOrEmpty(CurrentTrack?.Album) ? "SINGLE TRACK" : Fmt.Upper(CurrentTrack.Album);
    public bool IsRepeatOn => RepeatMode != 0;
    public bool IsRepeatOne => RepeatMode == 2;
    public bool IsVolumeOff => IsMuted || Volume <= 0;
    public bool IsVolumeLow => !IsVolumeOff && Volume < 0.5;
    public bool IsVolumeHigh => !IsVolumeOff && Volume >= 0.5;

    public event Action<string, ToastKind>? Toast;
    /// <summary>Slider maximum; never 0 so an empty player shows the thumb at the start.</summary>
    public double SeekMax => Duration > 0 ? Duration : 1;
    public string PositionText => Format(Position);
    public string DurationText => Format(Duration);

    public event Action<Track?>? TrackChanged;

    public PlayerViewModel(MpvEngine engine, AppSettings settings)
    {
        _engine = engine;
        _settings = settings;
        _volume = settings.Volume;
        _isExclusive = settings.ExclusiveMode;
        _replayGain = settings.ReplayGain is "track" or "album" ? settings.ReplayGain : "no";

        _engine.TimePosChanged += t =>
        {
            // Coalesce to 2 UI updates per second: the time label shows whole seconds and the
            // seek bar moves about a pixel per second, so faster updates only cost redraws.
            _lastKnownPosition = t;
            if (!IsUiActive) return; // window hidden in the tray or minimized: nothing to redraw
            var now = Environment.TickCount64;
            if (now - Interlocked.Read(ref _lastTimeUiTick) < 500) return;
            Interlocked.Exchange(ref _lastTimeUiTick, now);
            Dispatcher.UIThread.Post(() => { if (!IsSeeking) Position = t; });
        };
        // mpv only reports a duration once a file has really opened.
        _engine.DurationChanged += d => Dispatcher.UIThread.Post(() => { Duration = d; _loadFailures = 0; });
        // An idle mpv (started only to list devices) reports pause=no; that is not playback.
        // The same goes for an mpv restarted after a crash: nothing is loaded in it yet.
        _engine.PauseChanged += p => Dispatcher.UIThread.Post(() => IsPlaying = !p && CurrentTrack is not null && (_fileLoaded || IsPlaying));
        _engine.EndOfFile += () => Dispatcher.UIThread.Post(OnEndOfFile);
        _engine.PlaybackFailed += (path, why) => Dispatcher.UIThread.Post(() => OnPlaybackFailed(path, why));
        _engine.Exited += () => Dispatcher.UIThread.Post(OnEngineExited);
        _engine.AdvancedToNext += path =>
        {
            var gen = Volatile.Read(ref _playGen);
            Dispatcher.UIThread.Post(() => OnAdvanced(path, gen));
        };
    }

    /// <summary>mpv crashed or was killed: forget it, so the next play starts a new one.</summary>
    private void OnEngineExited()
    {
        _startTask = null;
        _fileLoaded = false;
        _queuedNextIndex = -1;
        if (IsPlaying) Toast?.Invoke("AUDIO_ENGINE_STOPPED // PRESS PLAY TO RESUME", ToastKind.Error);
        IsPlaying = false;
    }

    /// <summary>mpv could not be started or reached (mpv.exe missing, pipe never opened).</summary>
    private void OnEngineFailure(Exception ex)
    {
        _startTask = null;
        _fileLoaded = false;
        IsPlaying = false;
        Toast?.Invoke($"AUDIO_ENGINE_FAILED // {ex.Message}", ToastKind.Error);
    }

    /// <summary>The file is gone or unreadable: say so and move on instead of sitting on a silent track.</summary>
    private async void OnPlaybackFailed(string path, string why)
    {
        if (CurrentTrack is not { } track || !string.Equals(track.FilePath, path, StringComparison.OrdinalIgnoreCase)) return;
        _fileLoaded = false;
        var missing = await Task.Run(() => !File.Exists(path));
        if (!ReferenceEquals(CurrentTrack, track)) return; // the user moved on meanwhile
        Toast?.Invoke(missing
            ? $"FILE_NOT_FOUND // {Fmt.Upper(track.Title)}"
            : $"PLAYBACK_FAILED // {Fmt.Upper(track.Title)} // {why.ToUpperInvariant()}", ToastKind.Error);
        // A few in a row means it is not about one file (unplugged drive, device busy): stop there.
        if (++_loadFailures >= 3 || _queue.Count <= 1)
        {
            IsPlaying = false;
            return;
        }
        await Next();
    }

    // mpv is started lazily on first playback so an idle app holds no audio device.
    private Task EnsureEngineAsync()
    {
        return _startTask ??= Task.Run(async () =>
        {
            await _engine.StartAsync(IsExclusive, _settings.AudioDevice, ReplayGain);
            await _engine.SetVolumeAsync(Volume);
            if (IsMuted) await _engine.SetMuteAsync(true);
        });
    }

    public async void PlayQueue(IReadOnlyList<Track> tracks, int startIndex)
    {
        if (tracks.Count == 0) return;
        _queue = tracks.ToList();
        var start = Math.Clamp(startIndex, 0, _queue.Count - 1);
        _index = start;
        if (IsShuffle) BuildOrder();
        QueueChanged?.Invoke();
        await PlayIndexAsync(start, orderSet: true);
    }

    /// <summary>Appends to the play queue without interrupting playback.</summary>
    public void AddToQueue(IEnumerable<Track> tracks)
    {
        var list = tracks.ToList();
        if (list.Count == 0) return;
        if (_queue.Count == 0)
        {
            // Nothing queued: start playing, as addToQueue does in the Electron build.
            PlayQueue(list, 0);
            return;
        }
        var first = _queue.Count;
        _queue.AddRange(list);
        if (IsShuffle)
        {
            // New tracks land at random places among those still to come; what already played stays put.
            for (int i = first; i < _queue.Count; i++)
                _order.Insert(_rng.Next(_orderPos + 1, _order.Count + 1), i);
        }
        AfterQueueEdit();
    }

    /// <summary>Raised when the list of queued tracks changed (not on a mere track change).</summary>
    public event Action? QueueChanged;

    private void AfterQueueEdit()
    {
        RefreshQueue();
        SyncNext();
        QueueChanged?.Invoke();
    }

    // ---------- Shuffle order ----------

    /// <summary>
    /// A fresh round: the current track first, every other track once in random order.
    /// Unlike picking at random each time, nothing repeats until all have played, and Prev can walk back.
    /// </summary>
    private void BuildOrder()
    {
        _order = new List<int>(_queue.Count);
        for (int i = 0; i < _queue.Count; i++)
            if (i != _index) _order.Add(i);
        for (int i = _order.Count - 1; i > 0; i--)
        {
            int j = _rng.Next(i + 1);
            (_order[i], _order[j]) = (_order[j], _order[i]);
        }
        if (_index >= 0 && _index < _queue.Count)
        {
            _order.Insert(0, _index);
            _orderPos = 0;
        }
        else
        {
            _orderPos = -1;
        }
    }

    /// <summary>The user picked a track out of turn: it becomes the next step of the round.</summary>
    private void MoveOrderTo(int index)
    {
        if (_order.Count != _queue.Count) { BuildOrder(); return; }
        var at = _order.IndexOf(index);
        if (at < 0) { BuildOrder(); return; }
        if (at == _orderPos) return;
        _order.RemoveAt(at);
        if (at < _orderPos) _orderPos--;
        _order.Insert(++_orderPos, index);
    }

    private void RemoveFromOrder(int queueIndex)
    {
        var at = _order.IndexOf(queueIndex);
        if (at >= 0)
        {
            _order.RemoveAt(at);
            if (at < _orderPos) _orderPos--;
        }
        for (int i = 0; i < _order.Count; i++)
            if (_order[i] > queueIndex) _order[i]--;
    }

    /// <summary>What the Next button plays, or -1 at the end.</summary>
    private int NextIndex()
    {
        if (IsShuffle) return _orderPos + 1 < _order.Count ? _order[_orderPos + 1] : -1;
        if (_index + 1 < _queue.Count) return _index + 1;
        return RepeatMode == 1 && _queue.Count > 0 ? 0 : -1;
    }

    /// <summary>What follows the current track when it ends on its own (repeat-one included), or -1.</summary>
    private int PeekAutoNext()
    {
        if (_index < 0 || _index >= _queue.Count) return -1;
        return RepeatMode == 2 ? _index : NextIndex();
    }

    /// <summary>
    /// Tells mpv which file comes next, so it can open it early and continue into it without a gap.
    /// Called whenever the answer may have changed: a new track, a queue edit, shuffle or repeat toggled.
    /// </summary>
    private void SyncNext()
    {
        if (!_fileLoaded || _startTask is not { IsCompletedSuccessfully: true })
        {
            _queuedNextIndex = -1;
            return;
        }
        var next = PeekAutoNext();
        _queuedNextIndex = next;
        _ = _engine.SetNextAsync(next >= 0 ? _queue[next].FilePath : null);
    }

    /// <summary>mpv ran from the current file straight into the one queued behind it.</summary>
    private async void OnAdvanced(string path, int gen)
    {
        if (gen != _playGen || _loading) return; // an explicit load has taken over since
        var expected = _queuedNextIndex;
        if (expected >= 0 && expected < _queue.Count && string.Equals(_queue[expected].FilePath, path, StringComparison.OrdinalIgnoreCase))
        {
            _index = expected;
            if (IsShuffle) MoveOrderTo(expected);
            var track = _queue[expected];
            CurrentTrack = track;
            _lastKnownPosition = 0;
            if (!IsSeeking) Position = 0;
            Duration = track.Duration;
            _fileLoaded = true;
            IsPlaying = true;
            TrackChanged?.Invoke(track);
            RefreshQueue();
            SyncNext();
            return;
        }
        // The queue was edited right at the boundary and mpv entered a file that is no longer next.
        var want = PeekAutoNext();
        if (want >= 0)
        {
            await PlayIndexAsync(want);
            return;
        }
        _fileLoaded = false;
        IsPlaying = false;
        await _engine.StopAsync();
    }

    // ---------- Queue views (QueuePopup / FullScreenPlayer) ----------

    /// <summary>Cover thumbnails of the next few tracks, so they can be decoded before a queue view opens.</summary>
    public IEnumerable<string> UpcomingCovers(int max)
    {
        var shuffled = IsShuffle && _order.Count == _queue.Count;
        var first = shuffled ? _orderPos + 1 : Math.Max(0, _index + 1);
        var seen = new HashSet<string>();
        for (int n = 0; n < max && first + n < _queue.Count; n++)
        {
            var path = _queue[shuffled ? _order[first + n] : first + n].CoverThumbPath;
            if (!string.IsNullOrEmpty(path) && seen.Add(path)) yield return path;
        }
    }

    /// <summary>Tracks after the current one, as shown in NEXT SEQUENCE / Next in Queue.</summary>
    [ObservableProperty] private IReadOnlyList<QueueRow> _upcoming = [];
    [ObservableProperty] private QueueRow? _currentQueueRow;
    [ObservableProperty] private string _upcomingText = "0 TRACKS";
    [ObservableProperty] private bool _isQueueEmpty = true;
    [ObservableProperty] private bool _isEndOfQueue;
    [ObservableProperty] private bool _hasUpcoming;

    private bool _queueViewsActive;

    /// <summary>
    /// True while the queue popup or the full-screen player is open. The row list is only built then:
    /// with a queue of thousands of tracks, rebuilding it on every track change is wasted work.
    /// </summary>
    public bool QueueViewsActive
    {
        get => _queueViewsActive;
        set
        {
            if (_queueViewsActive == value) return;
            _queueViewsActive = value;
            RefreshQueue();
        }
    }

    private void RefreshQueue()
    {
        // In shuffle the rows follow the play order, so the list shows what will really come next.
        var shuffled = IsShuffle && _order.Count == _queue.Count;
        var first = shuffled ? _orderPos + 1 : Math.Max(0, _index + 1);
        var count = Math.Max(0, _queue.Count - first);
        if (_queueViewsActive && count > 0)
        {
            var rows = new List<QueueRow>(count);
            for (int n = 0; n < count; n++)
            {
                var i = shuffled ? _order[first + n] : first + n;
                rows.Add(new QueueRow(_queue[i], i, n + 1, first + n + 1, this));
            }
            Upcoming = rows; // one swap: the view resets once instead of once per row
        }
        else if (Upcoming.Count > 0)
        {
            Upcoming = [];
        }
        CurrentQueueRow = _index >= 0 && _index < _queue.Count ? new QueueRow(_queue[_index], _index, 0, first, this) : null;
        UpcomingText = $"{count} TRACKS";
        IsQueueEmpty = _queue.Count == 0;
        HasUpcoming = count > 0;
        IsEndOfQueue = !IsQueueEmpty && count == 0;
    }

    public async void PlayAt(int queueIndex)
    {
        if (queueIndex >= 0 && queueIndex < _queue.Count) await PlayIndexAsync(queueIndex);
    }

    public void RemoveAt(int queueIndex)
    {
        if (queueIndex < 0 || queueIndex >= _queue.Count || queueIndex == _index) return;
        _queue.RemoveAt(queueIndex);
        if (queueIndex < _index) _index--;
        if (IsShuffle) RemoveFromOrder(queueIndex);
        AfterQueueEdit();
    }

    /// <summary>
    /// Drops tracks from the queue (their album was deleted). If the playing track goes too,
    /// playback stops and the player returns to its empty state, as in handleBatchDelete.
    /// </summary>
    public async Task RemoveTracksAsync(Func<Track, bool> match)
    {
        var current = _index >= 0 && _index < _queue.Count ? _queue[_index] : null;
        _queue = _queue.Where(t => !match(t)).ToList();
        var stop = CurrentTrack is not null && match(CurrentTrack);
        if (stop)
        {
            _playGen++;
            _index = -1;
            _fileLoaded = false;
            _queuedNextIndex = -1;
            CurrentTrack = null;
            IsPlaying = false;
            Position = 0;
            _lastKnownPosition = 0;
            Duration = 0;
            TrackChanged?.Invoke(null);
        }
        else
        {
            _index = current is null ? -1 : _queue.IndexOf(current);
        }
        if (IsShuffle) BuildOrder();
        if (stop && _startTask is { IsCompletedSuccessfully: true }) await _engine.StopAsync();
        AfterQueueEdit();
    }

    /// <summary>[CLEAR_ALL_DATA]: keeps only the playing track.</summary>
    [RelayCommand]
    private void ClearQueue()
    {
        if (_index >= 0 && _index < _queue.Count)
        {
            _queue = [_queue[_index]];
            _index = 0;
        }
        else
        {
            _queue = [];
            _index = -1;
        }
        if (IsShuffle) BuildOrder();
        AfterQueueEdit();
    }

    /// <param name="orderSet">The shuffle order already points at this track (a new round, or stepping back).</param>
    private async Task PlayIndexAsync(int index, bool orderSet = false)
    {
        var gen = ++_playGen;
        _loading = true;
        _queuedNextIndex = -1;
        _index = index;
        if (IsShuffle && !orderSet) MoveOrderTo(index);
        RefreshQueue();
        var track = _queue[index];
        CurrentTrack = track;
        Position = 0;
        _lastKnownPosition = 0;
        Duration = track.Duration;
        TrackChanged?.Invoke(track);
        try
        {
            await EnsureEngineAsync();
            if (gen != _playGen) return; // a newer request came in while mpv was starting
            await _engine.PlayAsync(track.FilePath);
            _fileLoaded = true;
            IsPlaying = true;
        }
        catch (Exception ex)
        {
            OnEngineFailure(ex);
        }
        finally
        {
            if (gen == _playGen) _loading = false;
        }
        if (gen == _playGen && _fileLoaded) SyncNext();
    }

    [RelayCommand]
    private async Task TogglePlay()
    {
        if (CurrentTrack is not { } track) return;
        try
        {
            await EnsureEngineAsync();
            if (!_fileLoaded)
            {
                // Nothing is loaded in mpv yet (restored session, restarted mpv, failed load):
                // load the track where the UI left off.
                _playGen++;
                await _engine.PlayAsync(track.FilePath, Position);
                _fileLoaded = true;
                IsPlaying = true;
                SyncNext();
                return;
            }
            await _engine.SetPausedAsync(IsPlaying);
            IsPlaying = !IsPlaying;
        }
        catch (Exception ex)
        {
            OnEngineFailure(ex);
        }
    }

    [RelayCommand]
    private async Task Next()
    {
        if (_queue.Count == 0) return;
        var next = NextIndex();
        if (next < 0 && IsShuffle && RepeatMode == 1)
        {
            // Every track has had its turn: deal a new round, not opening with the one that just played.
            var last = _index;
            _index = -1;
            BuildOrder();
            if (_order.Count > 1 && _order[0] == last) (_order[0], _order[^1]) = (_order[^1], _order[0]);
            _orderPos = 0;
            await PlayIndexAsync(_order[0], orderSet: true);
            return;
        }
        if (next < 0)
        {
            await _engine.SetPausedAsync(true);
            IsPlaying = false;
            return;
        }
        await PlayIndexAsync(next);
    }

    [RelayCommand]
    private async Task Prev()
    {
        if (_queue.Count == 0) return;
        if (Position > 3) { await SeekAsync(0); return; }
        if (IsShuffle)
        {
            // Back through what was actually played; at the start of the round there is nothing before.
            if (_orderPos <= 0) { await SeekAsync(0); return; }
            _orderPos--;
            await PlayIndexAsync(_order[_orderPos], orderSet: true);
            return;
        }
        await PlayIndexAsync(_index - 1 < 0 ? _queue.Count - 1 : _index - 1);
    }

    [RelayCommand] private void ToggleShuffle() => IsShuffle = !IsShuffle;
    [RelayCommand] private void CycleRepeat() => RepeatMode = (RepeatMode + 1) % 3;
    [RelayCommand] private void ToggleMute() => IsMuted = !IsMuted;
    [RelayCommand] private void ToggleExclusive() => IsExclusive = !IsExclusive;
    [RelayCommand] private void NotPorted(string feature) => Toast?.Invoke($"NOT_PORTED_YET // {feature}", ToastKind.Warning);

    partial void OnIsShuffleChanged(bool value)
    {
        if (value) BuildOrder();
        else
        {
            _order = [];
            _orderPos = -1;
        }
        RefreshQueue();
        SyncNext();
    }

    partial void OnRepeatModeChanged(int value) => SyncNext();

    // ---------- ReplayGain ----------

    /// <summary>"no", "track" or "album": level volume using the files' ReplayGain tags.</summary>
    [ObservableProperty] [NotifyPropertyChangedFor(nameof(IsReplayGainOff), nameof(IsReplayGainTrack), nameof(IsReplayGainAlbum), nameof(ReplayGainHint))]
    private string _replayGain;

    /// <summary>One line under the ReplayGain choices: what the selected mode does. Needs ReplayGain tags in the files.</summary>
    public string ReplayGainHint => ReplayGain switch
    {
        "track" => "TRACK: EVERY SONG AT THE SAME LOUDNESS. BEST FOR SHUFFLE.",
        "album" => "ALBUM: EACH ALBUM AT THE SAME LOUDNESS, ITS OWN DYNAMICS KEPT.",
        _ => "OFF: FILES PLAY AS MASTERED. TRACK / ALBUM EVEN OUT LOUDNESS USING REPLAYGAIN TAGS.",
    };

    public bool IsReplayGainOff => ReplayGain is not ("track" or "album");
    public bool IsReplayGainTrack => ReplayGain == "track";
    public bool IsReplayGainAlbum => ReplayGain == "album";

    [RelayCommand] private void SetReplayGain(string mode) => ReplayGain = mode is "track" or "album" ? mode : "no";

    partial void OnReplayGainChanged(string value)
    {
        _settings.ReplayGain = value;
        _settings.Save();
        if (_startTask is { IsCompletedSuccessfully: true }) _ = _engine.SetReplayGainAsync(value);
    }

    // ---------- Session (restored on the next start) ----------

    public IReadOnlyList<Track> Queue => _queue;
    public int QueueIndex => _index;
    /// <summary>Where playback really is; Position lags behind while the window is hidden.</summary>
    public double PlaybackPosition => _fileLoaded ? _lastKnownPosition : Position;

    /// <summary>
    /// Puts back the queue and the track of the last run, paused at its position. mpv is not started:
    /// the first Play loads the file there. Ignored when something was started meanwhile.
    /// </summary>
    public void RestoreSession(IReadOnlyList<Track> queue, int index, double position, bool shuffle, int repeatMode)
    {
        if (CurrentTrack is not null || _queue.Count > 0 || queue.Count == 0) return;
        _queue = queue.ToList();
        _index = index >= 0 && index < _queue.Count ? index : -1;
        RepeatMode = Math.Clamp(repeatMode, 0, 2);
        IsShuffle = shuffle; // builds the round around the restored track
        if (_index >= 0)
        {
            var track = _queue[_index];
            var pos = position > 0 && position < track.Duration - 2 ? position : 0;
            CurrentTrack = track;
            Duration = track.Duration;
            Position = pos;
            _lastKnownPosition = pos;
            TrackChanged?.Invoke(track);
        }
        RefreshQueue();
    }

    partial void OnIsMutedChanged(bool value)
    {
        // Unmuting at volume 0 would stay silent: come back at half volume (toggleMute in the Electron build).
        if (!value && Volume <= 0) Volume = 0.5;
        if (_startTask is { IsCompletedSuccessfully: true }) _ = _engine.SetMuteAsync(value);
    }

    public async Task SeekAsync(double seconds)
    {
        Position = seconds;
        _lastKnownPosition = seconds;
        if (_startTask is null) return;
        await _engine.SeekAsync(seconds);
    }

    private async void OnEndOfFile()
    {
        // Only reached when no next file was queued in mpv (see SyncNext); otherwise mpv moves on by itself.
        if (RepeatMode == 2 && CurrentTrack is not null && _index >= 0) { await PlayIndexAsync(_index, orderSet: true); return; }
        await Next();
    }

    partial void OnVolumeChanged(double value)
    {
        _settings.Volume = value;
        // Turning the volume up (K, bars, wheel) while muted unmutes, as in the Electron build.
        if (value > 0 && IsMuted) IsMuted = false;
        if (_startTask is { IsCompletedSuccessfully: true }) _ = _engine.SetVolumeAsync(value);
    }

    partial void OnIsExclusiveChanged(bool value)
    {
        _settings.ExclusiveMode = value;
        if (_startTask is { IsCompletedSuccessfully: true }) _ = _engine.SetExclusiveAsync(value);
    }

    // ---------- Output device (SELECT OUTPUT menu) ----------

    public ObservableCollection<AudioDeviceItem> AudioDevices { get; } = [];
    [ObservableProperty] private bool _hasNoDevices;

    /// <summary>Reloads the device list when the menu opens (starts mpv if it is not running yet).</summary>
    public async Task RefreshAudioDevicesAsync()
    {
        IReadOnlyList<AudioDevice> devices;
        try
        {
            await EnsureEngineAsync();
            devices = await _engine.GetAudioDevicesAsync();
        }
        catch
        {
            _startTask = null;
            devices = [];
        }
        AudioDevices.Clear();
        foreach (var d in devices)
            AudioDevices.Add(new AudioDeviceItem(d.Name, d.Name == "auto" ? "System Default" : d.Description, this)
            {
                IsSelected = d.Name == _settings.AudioDevice,
            });
        HasNoDevices = AudioDevices.Count == 0;
    }

    public async void SelectAudioDevice(AudioDeviceItem item)
    {
        foreach (var d in AudioDevices) d.IsSelected = ReferenceEquals(d, item);
        _settings.AudioDevice = item.Name;
        _settings.Save();
        if (_startTask is { IsCompletedSuccessfully: true }) await _engine.SetAudioDeviceAsync(item.Name);
    }

    private static string Format(double seconds)
    {
        if (double.IsNaN(seconds) || seconds < 0) seconds = 0;
        var t = TimeSpan.FromSeconds(seconds);
        return t.TotalHours >= 1 ? t.ToString(@"h\:mm\:ss") : t.ToString(@"m\:ss");
    }
}
