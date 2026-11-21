using System.Collections.ObjectModel;
using Avalonia.Threading;
using CommunityToolkit.Mvvm.ComponentModel;
using Retrograde.App.Services;
using Retrograde.Audio;
using Retrograde.Core;

namespace Retrograde.App.ViewModels;

/// <summary>
/// Direct output (bit-perfect): a sound card chosen under DIRECT OUTPUT is opened straight through ALSA (hw:),
/// after asking PipeWire to let go of it (DeviceReservation). Nothing sits between mpv and the driver: no
/// PipeWire mixer, no sample rate conversion, no format conversion. Everything that changes the samples is
/// bypassed while it is on: equalizer, ReplayGain, software volume and crossfade (a second stream cannot
/// open the same card). The status line shows what really reached the driver, read back from the kernel.
/// </summary>
public partial class PlayerViewModel
{
    private DeviceReservation? _reservation;
    private readonly SemaphoreSlim _deviceGate = new(1, 1);
    private int _statusGen;
    /// <summary>Playing straight to the card: the card is reserved and mpv runs on its hw: device. Read off the UI thread.</summary>
    private volatile bool _bitPerfect;
    private string _directProblem = "";
    private AudioPath? _lastPath;
    private int _pathGen;
    private bool _volumeToldAboutLock;

    // Hardware volume: in direct output the volume knob sets the card's own mixer control (PipeWire's volume is not in the path)
    private int _mixerCard = -1;
    private string? _mixerControl;
    private double _preDirectVolume;
    private bool _syncingVolume;
    private double _lastHwLevel = -1;
    private double _pendingHwLevel = -1;
    private int _hwWriter;
    private CancellationTokenSource? _hwPoll;

    /// <summary>Sound cards that can be opened directly (DIRECT OUTPUT list).</summary>
    public ObservableCollection<AudioDeviceItem> DirectDevices { get; } = [];
    [ObservableProperty] private bool _hasDirectDevices;

    /// <summary>The audio is going straight to the card: the equalizer, ReplayGain, volume and crossfade are bypassed.</summary>
    [ObservableProperty] private bool _isBitPerfectActive;

    /// <summary>A direct device is the chosen output (active or not): the status line shows.</summary>
    [ObservableProperty] private bool _showBitPerfectStatus;

    /// <summary>What the volume knob does while direct output is on: the card's own volume, or nothing when the card has none.</summary>
    [ObservableProperty] private string _directVolumeHint = "";

    /// <summary>One line: whether what reaches the card is the file's own samples.</summary>
    [ObservableProperty] private string _bitPerfectStatus = "";

    private void SetBitPerfect(bool on)
    {
        _bitPerfect = on;
        if (!on) _volumeToldAboutLock = false;
        Dispatcher.UIThread.Post(() =>
        {
            if (on) StartHardwareVolume(); else StopHardwareVolume();
            IsBitPerfectActive = on;
            ShowBitPerfectStatus = AlsaDirect.IsDirect(_settings.AudioDevice);
            RefreshBitPerfectStatus();
        });
    }

    private void Notify(string message, ToastKind kind) => Dispatcher.UIThread.Post(() => Toast?.Invoke(message, kind));

    private void ListDirectDevices()
    {
        DirectDevices.Clear();
        foreach (var d in AlsaDirect.List())
            DirectDevices.Add(new AudioDeviceItem(d.Name, d.Description, this) { IsSelected = d.Name == _settings.AudioDevice });
        HasDirectDevices = DirectDevices.Count > 0;
        ShowBitPerfectStatus = AlsaDirect.IsDirect(_settings.AudioDevice);
        RefreshBitPerfectStatus();
    }

    /// <summary>Takes the card behind a direct device from PipeWire. False (with a message) when someone keeps it.</summary>
    private async Task<bool> AcquireDirectAsync(string device)
    {
        var card = AlsaDirect.CardOf(device);
        if (card is null)
        {
            _directProblem = "DEVICE NOT FOUND";
            Notify("DIRECT_OUTPUT // DEVICE NOT FOUND", ToastKind.Warning);
            return false;
        }
        if (_reservation is { } held && held.Card == card) return true;
        var (reservation, holder) = await DeviceReservation.AcquireAsync(card.Value);
        if (reservation is null)
        {
            // Without a session bus nobody can be asked: try the card anyway, it may be free.
            if (holder == "no session bus") return true;
            _directProblem = $"IN USE BY {(holder ?? "ANOTHER PROGRAM").ToUpperInvariant()}";
            Notify($"DIRECT_OUTPUT // CARD IN USE BY {(holder ?? "ANOTHER PROGRAM").ToUpperInvariant()}", ToastKind.Warning);
            return false;
        }
        var previous = _reservation; // another card was held: mpv is still on it until the output moves, so it goes last
        _reservation = reservation;
        previous?.Dispose();
        return true;
    }

    private void ReleaseDirect()
    {
        _reservation?.Dispose(); // PipeWire takes the card back by itself
        _reservation = null;
    }

    /// <summary>Called as an engine starts: reserves the chosen direct device, or says why the session falls back to PipeWire.</summary>
    private async Task<string> ChooseOutputAsync()
    {
        var device = _settings.AudioDevice;
        if (!AlsaDirect.IsDirect(device))
        {
            ReleaseDirect();
            SetBitPerfect(false);
            return device;
        }
        var ok = await AcquireDirectAsync(device);
        SetBitPerfect(ok);
        return ok ? device : "auto";
    }

    /// <summary>The device for the spare (crossfade) engine: never a direct one, that card has one stream.</summary>
    private string SpareDevice() => AlsaDirect.IsDirect(_settings.AudioDevice) ? "auto" : _settings.AudioDevice;

    /// <summary>The output menu picked a direct device.</summary>
    private async Task SelectDirectAsync(AudioDeviceItem item)
    {
        if (_startTask is null)
        {
            // mpv is not running: the card is taken when it starts, at the first Play
            ApplyDeviceSelection(item);
            ReleaseDirect();
            SetBitPerfect(false);
            return;
        }
        var wasPlaying = IsPlaying;
        if (wasPlaying) await _engine.SetPausedAsync(true); // the card is taken away from a stream that is still on it
        if (!await AcquireDirectAsync(item.Name))
        {
            if (wasPlaying) await _engine.SetPausedAsync(false);
            return; // the selection stays where it was
        }
        ApplyDeviceSelection(item);
        await _engine.SetAudioFiltersAsync("");
        await _engine.SetReplayGainAsync("no");
        await _engine.SetVolumeAsync(1.0);
        await _engine.SetAudioDeviceAsync(item.Name);
        SetBitPerfect(true);
        if (wasPlaying) await _engine.SetPausedAsync(false);
    }

    /// <summary>The output menu picked a PipeWire device while the card was playing directly: give the card back first.</summary>
    private async Task LeaveDirectAsync(AudioDeviceItem item)
    {
        if (_startTask is null)
        {
            // no mpv to move: just give the card back
            ApplyDeviceSelection(item);
            ReleaseDirect();
            SetBitPerfect(false);
            return;
        }
        var wasPlaying = IsPlaying;
        if (wasPlaying) await _engine.SetPausedAsync(true);
        ApplyDeviceSelection(item);
        await _engine.SetAudioDeviceAsync(item.Name); // mpv lets go of the hw: device
        ReleaseDirect();
        SetBitPerfect(false);
        // PipeWire sets its node for the card up again a moment after the reservation ends: connect after that
        await Task.Delay(1000);
        await _engine.ReloadAudioAsync();
        await _engine.SetAudioFiltersAsync(EqualizerFilters());
        await _engine.SetReplayGainAsync(ReplayGain);
        await _engine.SetVolumeAsync(Volume);
        if (wasPlaying) await _engine.SetPausedAsync(false);
    }

    private void ApplyDeviceSelection(AudioDeviceItem item)
    {
        foreach (var d in AudioDevices) d.IsSelected = ReferenceEquals(d, item);
        foreach (var d in DirectDevices) d.IsSelected = ReferenceEquals(d, item);
        _settings.AudioDevice = item.Name;
        _settings.Save();
        ShowBitPerfectStatus = AlsaDirect.IsDirect(item.Name);
        _directProblem = "";
        _lastPath = null;
        RefreshBitPerfectStatus();
    }

    /// <summary>A card without a volume control of its own: the volume stays at 100%. Said once instead of letting the keys do nothing silently.</summary>
    private void TellVolumeIsLocked()
    {
        if (_volumeToldAboutLock) return;
        _volumeToldAboutLock = true;
        Toast?.Invoke("DIRECT_OUTPUT // THIS CARD HAS NO VOLUME CONTROL: VOLUME STAYS AT 100%, USE THE DAC'S OWN VOLUME", ToastKind.Info);
    }

    // ---------- Hardware volume ----------

    /// <summary>Direct output starts: the knob takes over the card's volume control, starting from where the card is.</summary>
    private void StartHardwareVolume()
    {
        if (AlsaDirect.CardOf(_settings.AudioDevice) is not { } card) return;
        if (_hwPoll is not null && _mixerCard == card) return; // already running for this card
        StopHardwareVolume();
        var control = AlsaMixer.FindControl(card);
        if (control is null || AlsaMixer.ReadLevel(card, control) is not { } level)
        {
            DirectVolumeHint = "THIS CARD HAS NO VOLUME CONTROL OF ITS OWN: THE VOLUME STAYS AT 100%. USE THE DAC'S OWN VOLUME.";
            return;
        }
        _mixerCard = card;
        _mixerControl = control;
        _preDirectVolume = Volume;
        _lastHwLevel = level;
        _syncingVolume = true;
        try { Volume = level; }
        finally { _syncingVolume = false; }
        DirectVolumeHint = $"VOLUME SETS THE CARD'S OWN {control.ToUpperInvariant()} CONTROL, THE FILE'S SAMPLES ARE NOT TOUCHED. THE SYSTEM VOLUME DOES NOT REACH THIS CARD.";
        var cts = _hwPoll = new CancellationTokenSource();
        _ = Task.Run(() => PollHardwareVolumeAsync(card, control, cts.Token));
    }

    /// <summary>Direct output ends: the knob goes back to the software volume it had before.</summary>
    private void StopHardwareVolume()
    {
        DirectVolumeHint = "";
        if (_hwPoll is null) return;
        _hwPoll.Cancel();
        _hwPoll = null;
        _mixerCard = -1;
        _mixerControl = null;
        _syncingVolume = true;
        try { Volume = _preDirectVolume; }
        finally { _syncingVolume = false; }
    }

    /// <summary>The knob moved in direct output: write it to the card. Quick changes (a drag) are merged into the last one.</summary>
    private void WriteHardwareVolume(double level)
    {
        if (_mixerControl is null) { TellVolumeIsLocked(); return; }
        Volatile.Write(ref _pendingHwLevel, level);
        if (Interlocked.Exchange(ref _hwWriter, 1) == 1) return; // a writer is running and will pick the newest value up
        var card = _mixerCard;
        var control = _mixerControl;
        _ = Task.Run(() =>
        {
            try
            {
                while (true)
                {
                    var next = Interlocked.Exchange(ref _pendingHwLevel, -1);
                    if (next < 0) break;
                    _lastHwLevel = next;
                    AlsaMixer.WriteLevel(card, control, next);
                }
            }
            finally { Volatile.Write(ref _hwWriter, 0); }
            // a value that arrived between the last check and giving up the writer role
            if (Volatile.Read(ref _pendingHwLevel) >= 0 && _mixerControl is not null) WriteHardwareVolume(Volatile.Read(ref _pendingHwLevel));
        });
    }

    /// <summary>The card's volume changed from outside (the DAC's own buttons, alsamixer ...): the knob follows. Once a second, only while the window shows.</summary>
    private async Task PollHardwareVolumeAsync(int card, string control, CancellationToken ct)
    {
        try
        {
            using var timer = new PeriodicTimer(TimeSpan.FromSeconds(1));
            while (await timer.WaitForNextTickAsync(ct))
            {
                if (!IsUiActive || Volatile.Read(ref _hwWriter) == 1) continue;
                if (AlsaMixer.ReadLevel(card, control) is not { } level) continue;
                if (Math.Abs(level - _lastHwLevel) < 0.01) continue; // our own last write, or no change
                _lastHwLevel = level;
                Dispatcher.UIThread.Post(() =>
                {
                    if (ct.IsCancellationRequested || !_bitPerfect) return;
                    _syncingVolume = true;
                    try { Volume = level; }
                    finally { _syncingVolume = false; }
                });
            }
        }
        catch (OperationCanceledException) { }
    }

    // ---------- What reaches the card ----------

    private void OnAudioPath(AudioPath path)
    {
        _lastPath = path;
        var gen = Interlocked.Increment(ref _pathGen);
        // mpv reports the decoder side and the output side separately, and the driver opens a moment later:
        // the line is worked out once the reports have settled.
        _ = Task.Run(async () =>
        {
            await Task.Delay(500);
            if (gen != Volatile.Read(ref _pathGen)) return;
            Dispatcher.UIThread.Post(RefreshBitPerfectStatus);
        });
    }

    private async void RefreshBitPerfectStatus()
    {
        var gen = ++_statusGen;
        try
        {
            var text = ComputeBitPerfectStatus();
            // "Bit-perfect" is only said when mpv itself confirms that nothing in its chain touches the samples
            if (text.StartsWith("BIT-PERFECT", StringComparison.Ordinal) && _startTask is { IsCompletedSuccessfully: true })
            {
                var engine = _engine;
                var altered = await engine.GetSampleAlterationAsync();
                if (altered is not null) text = $"NOT BIT-PERFECT // {altered}";
            }
            if (gen == _statusGen) BitPerfectStatus = text; // a newer refresh (device change ...) has the last word
        }
        catch (Exception ex) { ErrorLog.Write("output-status", ex); }
    }

    private string ComputeBitPerfectStatus()
    {
        if (!ShowBitPerfectStatus) return "";
        if (!_bitPerfect)
        {
            return _startTask is null
                ? "SELECTED. THE CARD IS TAKEN WHEN THE AUDIO ENGINE STARTS (FIRST PLAY, OR OPENING THIS MENU)."
                : $"NOT ACTIVE{(_directProblem.Length > 0 ? " // " + _directProblem : "")}. PLAYING THROUGH PIPEWIRE.";
        }
        var path = _lastPath;
        // Only the kernel's own report of the open card counts as proof
        var hw = AlsaDirect.ReadHwParams(_settings.AudioDevice);
        if (path?.In is not { } src || path.Out is null || hw is not { } h || !CurrentTrackIsLoaded())
            return "ACTIVE. PLAY A TRACK TO CHECK WHAT REACHES THE CARD.";
        var rate = h.Rate;
        var channels = h.Channels;
        var outBits = HwBits(h.Format);
        var outName = h.Format;
        var lossy = src.IntegerBits == 0; // decoded to float: MP3, AAC, Opus ...
        // mpv hands a 24-bit file over as s32: the file's own depth (when known) decides what the card must keep
        var srcBits = CurrentTrack is { BitsPerSample: > 0 } t && src.IntegerBits > 0 ? Math.Min(t.BitsPerSample, src.IntegerBits) : src.IntegerBits;
        if (rate != src.Rate)
            return $"NOT BIT-PERFECT // RESAMPLED {Khz(src.Rate)} TO {Khz(rate)} KHZ: THE CARD DOES NOT TAKE THIS RATE";
        if (channels != src.Channels)
            return $"NOT BIT-PERFECT // {src.Channels} CHANNELS MIXED TO {channels}";
        if (!lossy && outBits < srcBits)
            return $"NOT BIT-PERFECT // {srcBits}-BIT SOURCE REDUCED TO {outName}";
        if (lossy)
            return $"DIRECT // {Khz(rate)} KHZ // DECODED STRAIGHT TO THE CARD ({outName})";
        return $"BIT-PERFECT // {Khz(rate)} KHZ // {srcBits}-BIT // {outName}";
    }

    private bool CurrentTrackIsLoaded() => CurrentTrack is not null && _fileLoaded;

    private static string Khz(int hz) => (hz / 1000.0).ToString("0.###", System.Globalization.CultureInfo.InvariantCulture);

    /// <summary>Bits of an ALSA sample format name (S16_LE, S24_3LE, S32_LE); 0 for float or unknown.</summary>
    private static int HwBits(string format)
    {
        if (format.StartsWith("S16", StringComparison.Ordinal)) return 16;
        if (format.StartsWith("S24", StringComparison.Ordinal)) return 24;
        if (format.StartsWith("S32", StringComparison.Ordinal)) return 32;
        return 0;
    }
}
