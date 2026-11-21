using System.Collections.Concurrent;
using System.Diagnostics;
using System.Text;
using System.Text.Json;

namespace Retrograde.Audio;

/// <summary>
/// Drives mpv in this process through the system libmpv (libmpv.so.2). LibMpvClient takes mpv's JSON IPC
/// commands and reports back in the same JSON shape (replies with request_id, events with "event"), so the
/// message handling below is written against that shape. Events are raised on a background thread; UI code
/// must marshal them.
/// </summary>
public sealed class MpvEngine : IAsyncDisposable
{
    private const string LibName = "libmpv.so.2";
    private LibMpvClient? _lib;
    private PeriodicTimer? _pollTimer;
    private TimeSpan _pollInterval = TimeSpan.FromMilliseconds(500);
    private string _audioFilters = "";
    private CancellationTokenSource? _readCts;
    private double _pendingSeek;
    private volatile bool _playing;
    private volatile bool _closing;
    private string _audioDevice = "auto";
    private int _nextRequestId = 2000;
    private readonly ConcurrentDictionary<int, TaskCompletionSource<JsonElement?>> _pending = new();

    // The latest loadfile: its reply names the playlist entry that later end-file events refer to.
    private volatile int _loadRequestId = -1;
    private volatile int _currentEntryId = -1;
    private volatile string? _currentPath;

    // The track appended behind the current one, so mpv can run into it without a gap.
    // Request id -> path until mpv replies, then playlist entry id -> path until that entry starts.
    private readonly ConcurrentDictionary<int, string> _appendRequests = new();
    private readonly ConcurrentDictionary<int, string> _queued = new();
    private string _replayGain = "no";
    private AudioFormat? _pathIn, _pathOut;

    public event Action<double>? TimePosChanged;
    public event Action<double>? DurationChanged;
    public event Action<bool>? PauseChanged;
    public event Action? EndOfFile;
    /// <summary>The current file could not be opened or played: (path, mpv's reason).</summary>
    public event Action<string, string>? PlaybackFailed;
    /// <summary>mpv went away on its own (crashed or was killed). StartAsync starts a new one.</summary>
    public event Action? Exited;
    /// <summary>mpv ran from the current file into the one given to SetNextAsync: (its path).</summary>
    public event Action<string>? AdvancedToNext;
    /// <summary>The audio path changed: what the decoder produces and what goes to the output (rate, format, channels).</summary>
    public event Action<AudioPath>? AudioPathChanged;

    public bool IsRunning
    {
        get
        {
            return _lib is { IsAlive: true };
        }
    }

    /// <summary>How often the position is asked for while playing (500 ms; faster while lyrics show).</summary>
    public TimeSpan PollInterval
    {
        get => _pollInterval;
        set
        {
            _pollInterval = value;
            if (_pollTimer is { } timer) timer.Period = value;
        }
    }

    /// <summary>mpv's options for this app, without the leading "--" .</summary>
    private List<KeyValuePair<string, string>> Options() =>
    [
        // Only the options below apply: no user mpv.conf, no user scripts, no yt-dlp.
        new("config", "no"),
        new("load-scripts", "no"),
        new("ytdl", "no"),
        new("osc", "no"),
        new("load-stats-overlay", "no"),
        new("load-auto-profiles", "no"),
        new("idle", "yes"),
        new("ao", AoFor(_audioDevice)),
        new("audio-device", _audioDevice),
        new("audio-client-name", "Retrograde"),
        new("input-media-keys", "no"),
        new("media-controls", "no"),
        new("video", "no"),
        // video=no alone still opens a window for embedded cover art in this mpv build
        new("vid", "no"),
        new("audio-display", "no"),
        new("force-window", "no"),
        // keep-open only holds the last file; with a next file appended mpv runs straight into it.
        new("keep-open", "yes"),
        new("gapless-audio", "weak"),
        new("prefetch-playlist", "yes"),
        new("replaygain", _replayGain),
        new("af", _audioFilters),
        new("terminal", "no"),
        new("msg-level", "all=no"),
    ];

    /// <summary>
    /// The output driver for a device: a direct hw: device needs ao=alsa (with "pipewire,alsa" mpv would still
    /// pick PipeWire and ignore the device), everything else goes through PipeWire with ALSA as the fallback.
    /// </summary>
    private static string AoFor(string device) => AlsaDirect.IsDirect(device) ? "alsa" : "pipewire,alsa";

    public async Task StartAsync(string? audioDevice = null, string? replayGain = null, CancellationToken ct = default)
    {
        if (IsRunning) return;
        Teardown(); // an earlier mpv that died or never connected
        _closing = false;
        _audioDevice = string.IsNullOrEmpty(audioDevice) ? "auto" : audioDevice;
        _replayGain = NormalizeReplayGain(replayGain);

        if (!LibMpvClient.TryLoad(LibName))
            throw new FileNotFoundException("libmpv.so.2 not found: install the mpv package (sudo pacman -S mpv)", LibName);
        _lib = new LibMpvClient(Options(), OnLibMessage, OnLibShutdown);
        StartPolling();
        await ObserveAsync();
    }

    private void StartPolling()
    {
        var cts = _readCts ??= new CancellationTokenSource();
        _ = Task.Run(() => PollTimePosAsync(cts.Token));
    }

    // time-pos is polled (see PollTimePosAsync) instead of observed: mpv pushes it ~16x/s,
    // and handling that stream costs more CPU than the UI needs.
    private async Task ObserveAsync()
    {
        await SendAsync("observe_property", 2, "pause");
        await SendAsync("observe_property", 3, "eof-reached");
        await SendAsync("observe_property", 4, "duration");
        await SendAsync("observe_property", 5, "audio-params");
        await SendAsync("observe_property", 6, "audio-out-params");
    }

    private void OnLibMessage(string json)
    {
        try { HandleMessage(json); }
        catch (Exception ex) { Trace.WriteLine($"mpv message: {ex.Message}"); }
    }

    /// <summary>libmpv shut down without being asked to (it should not): the same as mpv exiting.</summary>
    private void OnLibShutdown()
    {
        if (_closing) return;
        _playing = false;
        _currentEntryId = -1;
        FailPending();
        try { Exited?.Invoke(); } catch { }
    }

    private void FailPending()
    {
        foreach (var id in _pending.Keys)
            if (_pending.TryRemove(id, out var tcs)) tcs.TrySetResult(null);
    }

    /// <summary>Releases libmpv. Safe to call when nothing is running.</summary>
    private void Teardown()
    {
        _playing = false;
        _currentEntryId = -1;
        _appendRequests.Clear();
        _queued.Clear();
        try { _readCts?.Cancel(); } catch { }
        _readCts = null;
        _pollTimer = null;
        var lib = _lib;
        _lib = null;
        lib?.Dispose();
        FailPending();
    }

    private const int TimePosRequestId = 1000;

    private Task RequestTimePosAsync() => SendAsync(TimePosRequestId, "get_property", "time-pos");

    /// <summary>Asks mpv for the position every PollInterval, only while something is playing.</summary>
    private async Task PollTimePosAsync(CancellationToken ct)
    {
        using var timer = new PeriodicTimer(_pollInterval);
        _pollTimer = timer;
        try
        {
            while (await timer.WaitForNextTickAsync(ct))
            {
                if (_playing) await RequestTimePosAsync();
            }
        }
        catch (OperationCanceledException) { }
    }

    private void HandleMessage(string line)
    {
        using var doc = JsonDocument.Parse(line);
        var root = doc.RootElement;
        if (root.TryGetProperty("request_id", out var rid) && rid.ValueKind == JsonValueKind.Number && rid.TryGetInt32(out var id) && id != 0)
        {
            if (id == TimePosRequestId)
            {
                if (root.TryGetProperty("data", out var pos) && pos.ValueKind == JsonValueKind.Number)
                    TimePosChanged?.Invoke(pos.GetDouble());
                return;
            }
            if (id == _loadRequestId)
            {
                if (root.TryGetProperty("data", out var ld) && ld.ValueKind == JsonValueKind.Object
                    && ld.TryGetProperty("playlist_entry_id", out var pe) && pe.TryGetInt32(out var entry))
                    _currentEntryId = entry;
                return;
            }
            if (_appendRequests.TryRemove(id, out var appended))
            {
                if (root.TryGetProperty("data", out var ad) && ad.ValueKind == JsonValueKind.Object
                    && ad.TryGetProperty("playlist_entry_id", out var ae) && ae.TryGetInt32(out var appendedEntry))
                    _queued[appendedEntry] = appended;
                return;
            }
            if (_pending.TryRemove(id, out var tcs))
                tcs.TrySetResult(root.TryGetProperty("data", out var d) ? d.Clone() : null);
            return;
        }
        if (!root.TryGetProperty("event", out var ev)) return;
        var evName = ev.GetString();
        if (evName == "file-loaded")
        {
            if (_pendingSeek > 0)
            {
                var s = _pendingSeek;
                _pendingSeek = 0;
                _ = SeekAsync(s);
            }
            return;
        }
        if (evName == "start-file")
        {
            // mpv moved on by itself to the file we appended: it is the current one now.
            if (root.TryGetProperty("playlist_entry_id", out var sid) && sid.TryGetInt32(out var started)
                && _queued.TryRemove(started, out var nextPath))
            {
                _currentEntryId = started;
                _currentPath = nextPath;
                AdvancedToNext?.Invoke(nextPath);
            }
            return;
        }
        if (evName == "end-file")
        {
            // Only an error on the file we asked for last: a missing file, an unreadable format, a lost device.
            if (root.TryGetProperty("reason", out var reason) && reason.GetString() == "error"
                && root.TryGetProperty("playlist_entry_id", out var eid) && eid.TryGetInt32(out var ended)
                && ended == _currentEntryId)
            {
                _currentEntryId = -1;
                var why = root.TryGetProperty("file_error", out var fe) ? fe.GetString() : null;
                PlaybackFailed?.Invoke(_currentPath ?? "", string.IsNullOrEmpty(why) ? "error" : why);
            }
            return;
        }
        if (evName != "property-change") return;
        var name = root.GetProperty("name").GetString();
        if (!root.TryGetProperty("data", out var data) || data.ValueKind == JsonValueKind.Null) return;
        switch (name)
        {
            case "audio-params": _pathIn = ParseFormat(data); AudioPathChanged?.Invoke(new AudioPath(_pathIn, _pathOut)); break;
            case "audio-out-params": _pathOut = ParseFormat(data); AudioPathChanged?.Invoke(new AudioPath(_pathIn, _pathOut)); break;
            case "duration": DurationChanged?.Invoke(data.GetDouble()); break;
            case "pause":
                _playing = !data.GetBoolean();
                PauseChanged?.Invoke(!_playing);
                break;
            case "eof-reached" when data.ValueKind == JsonValueKind.True: EndOfFile?.Invoke(); break;
        }
    }

    private static AudioFormat? ParseFormat(JsonElement e)
    {
        if (e.ValueKind != JsonValueKind.Object) return null;
        var format = e.TryGetProperty("format", out var f) ? f.GetString() : null;
        var rate = e.TryGetProperty("samplerate", out var r) && r.TryGetInt32(out var ri) ? ri : 0;
        var ch = e.TryGetProperty("channel-count", out var c) && c.TryGetInt32(out var ci) ? ci : 0;
        return format is null || rate <= 0 ? null : new AudioFormat(format, rate, ch);
    }

    private Task SendAsync(string command, params object[] args) => SendAsync(null, command, args);

    private Task SendAsync(int? requestId, string command, params object[] args)
    {
        if (_lib is { } lib) SendToLib(lib, requestId, command, args);
        return Task.CompletedTask;
    }

    /// <summary>A request as libmpv takes it: IPC-style verbs map to client API calls.</summary>
    private static void SendToLib(LibMpvClient lib, int? requestId, string command, object[] args)
    {
        var userData = (ulong)(requestId ?? 0);
        switch (command)
        {
            case "observe_property": lib.Observe((ulong)(int)args[0], (string)args[1]); break;
            case "get_property": lib.GetProperty(userData, (string)args[0]); break;
            case "set_property": lib.SetProperty((string)args[0], MpvString(args[1])); break;
            default:
            {
                var list = new string[args.Length + 1];
                list[0] = command;
                for (int i = 0; i < args.Length; i++) list[i + 1] = MpvString(args[i]);
                lib.Command(userData, list);
                break;
            }
        }
    }

    private static string MpvString(object value) => value switch
    {
        bool b => b ? "yes" : "no",
        double d => d.ToString("R", System.Globalization.CultureInfo.InvariantCulture),
        int i => i.ToString(System.Globalization.CultureInfo.InvariantCulture),
        _ => value.ToString() ?? "",
    };

    /// <summary>Loads a file and starts playing. mpv keeps the pause flag across loadfile, so clear it explicitly.</summary>
    public async Task PlayAsync(string filePath, double startSeconds = 0)
    {
        _currentEntryId = -1; // end-file events of the file being replaced are ignored from here on
        _currentPath = filePath;
        // loadfile also takes URLs and mpv's own protocols; the library only ever holds local files.
        if (!Path.IsPathFullyQualified(filePath))
        {
            PlaybackFailed?.Invoke(filePath, "not a local file");
            return;
        }
        _appendRequests.Clear();
        _queued.Clear(); // "replace" drops mpv's playlist, the appended file included
        _pendingSeek = startSeconds;
        var id = Interlocked.Increment(ref _nextRequestId);
        _loadRequestId = id;
        await SendAsync(id, "loadfile", filePath, "replace");
        await SendAsync("set_property", "pause", false);
    }

    /// <summary>
    /// Names the file to play after the current one (null: none), replacing an earlier choice.
    /// mpv then opens it ahead of time and continues into it without a gap; AdvancedToNext says when.
    /// Entries already handed to mpv stay known here: if mpv is entering one at this very moment,
    /// the caller still hears about it and can correct course.
    /// </summary>
    public async Task SetNextAsync(string? path)
    {
        _appendRequests.Clear();
        await SendAsync("playlist-clear"); // keeps the playing file, drops what was appended before
        if (path is null || !Path.IsPathFullyQualified(path)) return;
        if (_queued.Count > 64) _queued.Clear();
        var id = Interlocked.Increment(ref _nextRequestId);
        _appendRequests[id] = path;
        await SendAsync(id, "loadfile", path, "append");
    }

    private static string NormalizeReplayGain(string? mode) => mode is "track" or "album" ? mode : "no";

    /// <summary>"no", "track" or "album": volume levelling from the files' ReplayGain tags.</summary>
    public async Task SetReplayGainAsync(string mode)
    {
        _replayGain = NormalizeReplayGain(mode);
        await SendAsync("set_property", "replaygain", _replayGain);
    }

    public Task SetPausedAsync(bool paused) => SendAsync("set_property", "pause", paused);
    public Task TogglePauseAsync() => SendAsync("cycle", "pause");
    public async Task SeekAsync(double seconds)
    {
        await SendAsync("seek", seconds, "absolute");
        await RequestTimePosAsync(); // refresh position right away
    }
    public Task SetVolumeAsync(double volume01) => SendAsync("set_property", "volume", Math.Clamp(volume01, 0, 1) * 100.0);
    public Task SetMuteAsync(bool muted) => SendAsync("set_property", "mute", muted);
    public Task StopAsync()
    {
        _currentEntryId = -1;
        return SendAsync("stop");
    }

    /// <summary>
    /// mpv's audio filter chain (the af option, e.g. the equalizer's lavfi filters); "" for none.
    /// Kept for the next start too.
    /// </summary>
    public async Task SetAudioFiltersAsync(string filters)
    {
        if (_audioFilters == filters) return;
        _audioFilters = filters;
        await SendAsync("set_property", "af", filters);
    }

    /// <summary>
    /// A command to one filter inside a labelled lavfi entry of the chain (e.g. a new gain for one
    /// equalizer band). The target names the ffmpeg filter: sent to "all", the graph's auto-inserted
    /// format filters reject it and mpv reports an error.
    /// </summary>
    public Task FilterCommandAsync(string label, string target, string command, string argument) =>
        SendAsync("af-command", label, command, argument, target);

    /// <summary>Sends a command and waits for mpv's reply; null on error or after 2 s.</summary>
    private async Task<JsonElement?> RequestAsync(string command, params object[] args)
    {
        var id = Interlocked.Increment(ref _nextRequestId);
        var tcs = new TaskCompletionSource<JsonElement?>(TaskCreationOptions.RunContinuationsAsynchronously);
        _pending[id] = tcs;
        await SendAsync(id, command, args);
        var done = await Task.WhenAny(tcs.Task, Task.Delay(2000));
        _pending.TryRemove(id, out _);
        return done == tcs.Task ? tcs.Task.Result : null;
    }

    /// <summary>Output devices as mpv sees them for --ao=pipewire ("auto" first).</summary>
    public async Task<IReadOnlyList<AudioDevice>> GetAudioDevicesAsync()
    {
        var list = new List<AudioDevice>();
        if (await RequestAsync("get_property", "audio-device-list") is not { ValueKind: JsonValueKind.Array } arr) return list;
        foreach (var d in arr.EnumerateArray())
        {
            var name = d.TryGetProperty("name", out var n) ? n.GetString() : null;
            // mpv lists every audio backend's devices; only pipewire ones are offered (ALSA stays the fallback).
            if (string.IsNullOrEmpty(name) || (name != "auto" && !name.StartsWith("pipewire/", StringComparison.Ordinal))) continue;
            var desc = d.TryGetProperty("description", out var de) ? de.GetString() : null;
            list.Add(new AudioDevice(name, string.IsNullOrEmpty(desc) ? name : desc));
        }
        return list;
    }

    /// <summary>Moves output to another device; the current file keeps playing.</summary>
    public async Task SetAudioDeviceAsync(string name)
    {
        if (_audioDevice == name) return;
        _audioDevice = name;
        _pathOut = null; // the decoder side stays as it is
        await SendAsync("set_property", "ao", AoFor(name)); // the driver follows the device (hw: needs alsa)
        await SendAsync("set_property", "audio-device", name);
        await SendAsync("ao-reload");
    }

    /// <summary>
    /// What in mpv's own settings would change the samples before the output (a filter chain such as the
    /// equalizer, software volume, ReplayGain, playback speed), as a short phrase; null when nothing does.
    /// Asked of mpv itself. A question mpv does not answer is reported too: it cannot be confirmed.
    /// </summary>
    public async Task<string?> GetSampleAlterationAsync()
    {
        if (await RequestAsync("get_property", "af") is not { } af || af.ValueKind != JsonValueKind.Array) return "MPV DID NOT CONFIRM ITS SETTINGS";
        if (af.GetArrayLength() > 0) return "AN AUDIO FILTER IS ON";
        if (await RequestAsync("get_property", "volume") is not { ValueKind: JsonValueKind.Number } v) return "MPV DID NOT CONFIRM ITS SETTINGS";
        if (Math.Abs(v.GetDouble() - 100) > 0.01) return "SOFTWARE VOLUME IS ON";
        // The choice "no" comes back as the boolean false, "track" and "album" as strings
        if (await RequestAsync("get_property", "replaygain") is not { } rg || rg.ValueKind is not (JsonValueKind.False or JsonValueKind.String)) return "MPV DID NOT CONFIRM ITS SETTINGS";
        if (rg.ValueKind == JsonValueKind.String && rg.GetString() != "no") return "REPLAYGAIN IS ON";
        if (await RequestAsync("get_property", "speed") is not { ValueKind: JsonValueKind.Number } sp) return "MPV DID NOT CONFIRM ITS SETTINGS";
        if (Math.Abs(sp.GetDouble() - 1) > 0.0001) return "PLAYBACK SPEED IS CHANGED";
        return null;
    }

    /// <summary>Opens the audio output again on the current device (after the device came back or changed state).</summary>
    public Task ReloadAudioAsync()
    {
        _pathOut = null;
        return SendAsync("ao-reload");
    }

    public async ValueTask DisposeAsync()
    {
        _closing = true; // the read loop ending is expected from here on
        Teardown();
        await Task.CompletedTask;
    }
}

/// <summary>One end of the audio path: mpv's sample format name (s16, s32, floatp ...), sample rate and channel count.</summary>
public sealed record AudioFormat(string Format, int Rate, int Channels)
{
    /// <summary>Bits per sample of an integer format, 0 for float formats.</summary>
    public int IntegerBits => Format.TrimEnd('p') switch { "u8" => 8, "s16" => 16, "s24" => 24, "s32" => 32, "s64" => 64, _ => 0 };
}

/// <summary>What the decoder produces (In) and what goes to the output (Out); null until mpv has reported it.</summary>
public sealed record AudioPath(AudioFormat? In, AudioFormat? Out);

/// <summary>An mpv output device: Name is what --audio-device takes, Description is the friendly label.</summary>
public sealed record AudioDevice(string Name, string Description);
