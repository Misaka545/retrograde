using System.Diagnostics;
using Avalonia.Threading;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Retrograde.Audio;
using Retrograde.Core;

namespace Retrograde.App.ViewModels;

/// <summary>
/// Crossfade: near the end of a track the next one starts on a second engine (the spare) and the two
/// overlap for CrossfadeSeconds, equal-power, then the old one stops and the engines trade places.
/// An album heard in order stays gapless (live albums, mixes, classical), and so does everything in
/// WASAPI exclusive mode, which allows a single stream.
/// </summary>
public partial class PlayerViewModel
{
    private MpvEngine? _spare;
    private Task? _spareStart;
    private CancellationTokenSource? _fadeCts;
    /// <summary>Playback position (seconds) at which the fade starts; +∞ when none is due. Read off the UI thread.</summary>
    private double _fadeArmAt = double.PositiveInfinity;

    public static IReadOnlyList<int> CrossfadeChoices { get; } = [0, 2, 4, 6, 8, 10, 12];

    /// <summary>The choices as buttons (OFF, 2S ... 12S) for the audio menu.</summary>
    public IReadOnlyList<CrossfadeOption> CrossfadeOptions { get; private set; } = [];

    [ObservableProperty] [NotifyPropertyChangedFor(nameof(CrossfadeHint))]
    private int _crossfadeSeconds;

    public string CrossfadeHint =>
        CrossfadeSeconds == 0 ? "OFF: TRACKS FOLLOW EACH OTHER WITHOUT A GAP."
        : IsExclusive ? "PAUSED WHILE WASAPI EXCLUSIVE IS ON: IT ALLOWS ONE STREAM ONLY."
        : $"TRACKS OVERLAP FOR {CrossfadeSeconds} S. AN ALBUM PLAYED IN ORDER STAYS GAPLESS.";

    private void InitCrossfade()
    {
#pragma warning disable MVVMTK0034 // backing field: no save while constructing
        _crossfadeSeconds = CrossfadeChoices.Contains(_settings.CrossfadeSeconds) ? _settings.CrossfadeSeconds : 0;
#pragma warning restore MVVMTK0034
        CrossfadeOptions = CrossfadeChoices.Select(s => new CrossfadeOption(s, this) { IsSelected = s == CrossfadeSeconds }).ToList();
    }

    [RelayCommand] private void SetCrossfade(int seconds) => CrossfadeSeconds = CrossfadeChoices.Contains(seconds) ? seconds : 0;

    partial void OnCrossfadeSecondsChanged(int value)
    {
        _settings.CrossfadeSeconds = value;
        _settings.Save();
        foreach (var o in CrossfadeOptions) o.IsSelected = o.Seconds == value;
        SyncNext(); // the next change of track may now fade, or go back to gapless
    }

    /// <summary>The engines that are running: settings changes go to both.</summary>
    private IEnumerable<MpvEngine> StartedEngines()
    {
        if (_startTask is { IsCompletedSuccessfully: true }) yield return _engine;
        if (_spare is { } spare && _spareStart is { IsCompletedSuccessfully: true }) yield return spare;
    }

    /// <summary>Whether the change from the current track to this queue index fades.</summary>
    private bool CrossfadeApplies(int next)
    {
        if (CrossfadeSeconds <= 0 || IsExclusive || RepeatMode == 2) return false;
        if (next < 0 || next >= _queue.Count || _index < 0 || _index >= _queue.Count || next == _index) return false;
        var current = _queue[_index];
        var following = _queue[next];
        // An album heard in order stays gapless
        if (!IsShuffle && current.Album.Length > 0 && current.Album == following.Album) return false;
        // Too short to overlap
        return current.Duration <= 0 || current.Duration >= CrossfadeSeconds * 2 + 2;
    }

    /// <summary>Sets the position where the fade into the next track starts, if it fades.</summary>
    private void ArmCrossfade()
    {
        if (_fadeCts is not null || !_fileLoaded || Duration <= 0 || !CrossfadeApplies(PeekAutoNext()))
        {
            DisarmCrossfade();
            return;
        }
        Volatile.Write(ref _fadeArmAt, Duration - CrossfadeSeconds);
    }

    private void DisarmCrossfade() => Volatile.Write(ref _fadeArmAt, double.PositiveInfinity);

    /// <summary>Position poll (background thread): starts the fade once the arm point is reached.</summary>
    private void CheckCrossfade(double t)
    {
        if (t < Volatile.Read(ref _fadeArmAt)) return;
        DisarmCrossfade();
        var gen = Volatile.Read(ref _playGen);
        Dispatcher.UIThread.Post(() => StartCrossfade(gen));
    }

    private async void StartCrossfade(int gen)
    {
        if (gen != _playGen || _loading || _fadeCts is not null || !IsPlaying || !_fileLoaded) return;
        var next = PeekAutoNext();
        if (!CrossfadeApplies(next)) return;
        var track = _queue[next];
        // After a seek close to the end there is less time left than the full fade
        var seconds = Math.Clamp(Duration - _lastKnownPosition, 0.5, CrossfadeSeconds);
        var cts = new CancellationTokenSource();
        _fadeCts = cts;
        var outgoing = _engine;
        var outgoingStart = _startTask;
        MpvEngine? incoming = null;
        try
        {
            if (_spare is null)
            {
                _spare = _engineFactory();
                Wire(_spare);
            }
            incoming = _spare;
            _spareStart ??= StartEngineAsync(incoming, 0);
            var incomingStart = _spareStart;
            await incomingStart;
            if (cts.IsCancellationRequested || gen != _playGen) return;
            await incoming.SetVolumeAsync(0);
            await incoming.PlayAsync(track.FilePath);
            if (cts.IsCancellationRequested || gen != _playGen)
            {
                await incoming.StopAsync();
                return;
            }

            // The engines trade places: the new track is the current one from here on
            _engine = incoming;
            _startTask = incomingStart;
            _spare = outgoing;
            _spareStart = outgoingStart;
            _playGen++;
            _index = next;
            if (IsShuffle) MoveOrderTo(next);
            CurrentTrack = track;
            _lastKnownPosition = 0;
            if (!IsSeeking) Position = 0;
            Duration = track.Duration;
            _fileLoaded = true;
            IsPlaying = true;
            TrackChanged?.Invoke(track);
            RefreshQueue();
            SyncNext();

            await RampAsync(outgoing, incoming, seconds, cts.Token);
        }
        catch (Exception ex)
        {
            ErrorLog.Write("crossfade", ex);
            if (incoming is not null && incoming != _engine) _spareStart = null; // start it afresh next time
        }
        finally
        {
            if (_fadeCts == cts) _fadeCts = null;
            cts.Dispose();
            ArmCrossfade();
        }
    }

    /// <summary>
    /// The overlap: equal-power curves (sin / cos of amplitude). mpv's volume is cubic (amplitude =
    /// (volume/100)³), so the amplitude curve is taken to the cube root to get the volume to set.
    /// </summary>
    private async Task RampAsync(MpvEngine outgoing, MpvEngine incoming, double seconds, CancellationToken ct)
    {
        var clock = Stopwatch.StartNew();
        try
        {
            while (true)
            {
                var p = Math.Min(1, clock.Elapsed.TotalSeconds / seconds);
                var volume = Volume;
                await incoming.SetVolumeAsync(volume * Math.Cbrt(Math.Sin(p * Math.PI / 2)));
                await outgoing.SetVolumeAsync(volume * Math.Cbrt(Math.Cos(p * Math.PI / 2)));
                if (p >= 1) break;
                await Task.Delay(40, ct);
            }
        }
        catch (OperationCanceledException) { /* cut short: pause, seek, skip */ }
        finally
        {
            await outgoing.StopAsync();
            await outgoing.SetVolumeAsync(Volume);
            if (incoming == _engine) await incoming.SetVolumeAsync(Volume);
        }
    }

    /// <summary>Stops a fade in progress at once: the outgoing track stops, the new one plays at full volume.</summary>
    private void CancelFade()
    {
        try { _fadeCts?.Cancel(); } catch (ObjectDisposedException) { }
        DisarmCrossfade();
    }

    /// <summary>Closes the spare engine (exclusive mode: it must not hold the device).</summary>
    private void DropSpare()
    {
        var spare = _spare;
        _spare = null;
        _spareStart = null;
        if (spare is not null) _ = Task.Run(() => spare.DisposeAsync().AsTask());
    }

    /// <summary>App exit: closes both engines.</summary>
    public async Task DisposeEnginesAsync()
    {
        CancelFade();
        var spare = _spare;
        _spare = null;
        if (spare is not null) await spare.DisposeAsync();
        await _engine.DisposeAsync();
    }
}

public sealed partial class CrossfadeOption(int seconds, PlayerViewModel player) : ObservableObject
{
    public int Seconds { get; } = seconds;
    public string Label { get; } = seconds == 0 ? "OFF" : $"{seconds}S";
    [ObservableProperty] private bool _isSelected;
    [RelayCommand] private void Select() => player.SetCrossfadeCommand.Execute(Seconds);
}
