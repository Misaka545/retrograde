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

    [ObservableProperty] [NotifyPropertyChangedFor(nameof(HasTrack), nameof(TitleText), nameof(ArtistText), nameof(QualityText), nameof(CoverPath), nameof(AlbumHeader))]
    private Track? _currentTrack;

    [ObservableProperty] [NotifyPropertyChangedFor(nameof(StatusText))]
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
    public string StatusText => IsPlaying ? "PLAYING" : "READY";
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
    }

    /// <summary>mpv crashed or was killed: forget it, so the next play starts a new one.</summary>
    private void OnEngineExited()
    {
        _startTask = null;
        _fileLoaded = false;
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
            await _engine.StartAsync(IsExclusive, _settings.AudioDevice);
            await _engine.SetVolumeAsync(Volume);
            if (IsMuted) await _engine.SetMuteAsync(true);
        });
    }

    public async void PlayQueue(IReadOnlyList<Track> tracks, int startIndex)
    {
        if (tracks.Count == 0) return;
        _queue = tracks.ToList();
        await PlayIndexAsync(Math.Clamp(startIndex, 0, _queue.Count - 1));
    }

    /// <summary>Appends to the play queue without interrupting playback.</summary>
    public void AddToQueue(IEnumerable<Track> tracks)
    {
        _queue.AddRange(tracks); // with nothing played yet, Next starts at index 0
        RefreshQueue();
    }

    // ---------- Queue views (QueuePopup / FullScreenPlayer) ----------

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
        var first = Math.Max(0, _index + 1);
        var count = Math.Max(0, _queue.Count - first);
        if (_queueViewsActive && count > 0)
        {
            var rows = new List<QueueRow>(count);
            for (int i = first; i < _queue.Count; i++)
                rows.Add(new QueueRow(_queue[i], i, i - _index, this));
            Upcoming = rows; // one swap: the view resets once instead of once per row
        }
        else if (Upcoming.Count > 0)
        {
            Upcoming = [];
        }
        CurrentQueueRow = _index >= 0 && _index < _queue.Count ? new QueueRow(_queue[_index], _index, 0, this) : null;
        UpcomingText = $"{count} TRACKS";
        IsQueueEmpty = _queue.Count == 0;
        HasUpcoming = count > 0;
        IsEndOfQueue = !IsQueueEmpty && _index >= _queue.Count - 1;
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
        RefreshQueue();
    }

    /// <summary>
    /// Drops tracks from the queue (their album was deleted). If the playing track goes too,
    /// playback stops and the player returns to its empty state, as in handleBatchDelete.
    /// </summary>
    public async Task RemoveTracksAsync(Func<Track, bool> match)
    {
        var current = _index >= 0 && _index < _queue.Count ? _queue[_index] : null;
        _queue = _queue.Where(t => !match(t)).ToList();
        if (CurrentTrack is not null && match(CurrentTrack))
        {
            _index = -1;
            _fileLoaded = false;
            CurrentTrack = null;
            IsPlaying = false;
            Position = 0;
            Duration = 0;
            TrackChanged?.Invoke(null);
            if (_startTask is { IsCompletedSuccessfully: true }) await _engine.StopAsync();
        }
        else
        {
            _index = current is null ? -1 : _queue.IndexOf(current);
        }
        RefreshQueue();
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
        RefreshQueue();
    }

    private async Task PlayIndexAsync(int index)
    {
        _index = index;
        RefreshQueue();
        var track = _queue[index];
        CurrentTrack = track;
        Position = 0;
        Duration = track.Duration;
        TrackChanged?.Invoke(track);
        try
        {
            await EnsureEngineAsync();
            await _engine.PlayAsync(track.FilePath);
            _fileLoaded = true;
            IsPlaying = true;
        }
        catch (Exception ex)
        {
            OnEngineFailure(ex);
        }
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
                // mpv was restarted, or the last load failed: load the track again where the UI left off.
                await _engine.PlayAsync(track.FilePath, Position);
                _fileLoaded = true;
                IsPlaying = true;
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
        int next;
        if (IsShuffle) next = _rng.Next(_queue.Count);
        else if (_index + 1 < _queue.Count) next = _index + 1;
        else if (RepeatMode == 1) next = 0;
        else { await _engine.SetPausedAsync(true); IsPlaying = false; return; }
        await PlayIndexAsync(next);
    }

    [RelayCommand]
    private async Task Prev()
    {
        if (_queue.Count == 0) return;
        if (Position > 3) { await SeekAsync(0); return; }
        await PlayIndexAsync(_index - 1 < 0 ? _queue.Count - 1 : _index - 1);
    }

    [RelayCommand] private void ToggleShuffle() => IsShuffle = !IsShuffle;
    [RelayCommand] private void CycleRepeat() => RepeatMode = (RepeatMode + 1) % 3;
    [RelayCommand] private void ToggleMute() => IsMuted = !IsMuted;
    [RelayCommand] private void ToggleExclusive() => IsExclusive = !IsExclusive;
    [RelayCommand] private void NotPorted(string feature) => Toast?.Invoke($"NOT_PORTED_YET // {feature}", ToastKind.Warning);

    partial void OnIsMutedChanged(bool value)
    {
        if (_startTask is { IsCompletedSuccessfully: true }) _ = _engine.SetMuteAsync(value);
    }

    public async Task SeekAsync(double seconds)
    {
        Position = seconds;
        if (_startTask is null) return;
        await _engine.SeekAsync(seconds);
    }

    private async void OnEndOfFile()
    {
        if (RepeatMode == 2 && CurrentTrack is not null) { await PlayIndexAsync(_index); return; }
        await Next();
    }

    partial void OnVolumeChanged(double value)
    {
        _settings.Volume = value;
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
