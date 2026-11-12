using System.Collections.Concurrent;
using System.Diagnostics;
using System.IO.Pipes;
using System.Text;
using System.Text.Json;

namespace Retrograde.Audio;

/// <summary>
/// Drives a background mpv.exe through its JSON IPC named pipe (same protocol as the Electron build).
/// mpv handles both shared and WASAPI-exclusive output, so there is a single playback engine.
/// Events are raised on a background thread; UI code must marshal them.
/// </summary>
public sealed class MpvEngine : IAsyncDisposable
{
    private readonly string _mpvPath;
    private readonly string _pipeBase = $"retrograde-native-{Environment.ProcessId}";
    private int _starts;
    private Process? _process;
    private NamedPipeClientStream? _pipe;
    private StreamWriter? _writer;
    private readonly SemaphoreSlim _writeLock = new(1, 1);
    private CancellationTokenSource? _readCts;
    private bool _exclusive;
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

    public event Action<double>? TimePosChanged;
    public event Action<double>? DurationChanged;
    public event Action<bool>? PauseChanged;
    public event Action? EndOfFile;
    /// <summary>The current file could not be opened or played: (path, mpv's reason).</summary>
    public event Action<string, string>? PlaybackFailed;
    /// <summary>mpv went away on its own (crashed or was killed). StartAsync starts a new one.</summary>
    public event Action? Exited;

    public bool IsRunning
    {
        get
        {
            try { return _pipe is { IsConnected: true } && _process is { HasExited: false }; }
            catch (InvalidOperationException) { return false; }
        }
    }

    public MpvEngine(string mpvPath) => _mpvPath = mpvPath;

    public async Task StartAsync(bool exclusive, string? audioDevice = null, CancellationToken ct = default)
    {
        if (IsRunning) return;
        if (!File.Exists(_mpvPath)) throw new FileNotFoundException("mpv.exe not found", _mpvPath);
        Teardown(); // an earlier mpv that died or never connected
        _closing = false;
        _exclusive = exclusive;
        _audioDevice = string.IsNullOrEmpty(audioDevice) ? "auto" : audioDevice;
        // A new pipe name per start: a dying mpv may still hold the previous one.
        var pipeName = $"{_pipeBase}-{++_starts}";

        var psi = new ProcessStartInfo(_mpvPath)
        {
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardOutput = false,
            RedirectStandardError = false,
        };
        foreach (var arg in new[]
                 {
                     // Only the options below apply: no user mpv.conf, no user scripts, no yt-dlp.
                     "--no-config",
                     "--load-scripts=no",
                     "--ytdl=no",
                     "--osc=no",
                     "--load-stats-overlay=no",
                     "--load-auto-profiles=no",
                     "--idle=yes",
                     $"--input-ipc-server=\\\\.\\pipe\\{pipeName}",
                     "--ao=wasapi",
                     $"--audio-exclusive={(exclusive ? "yes" : "no")}",
                     $"--audio-device={_audioDevice}",
                     "--audio-client-name=Retrograde",
                     "--input-media-keys=no",
                     "--media-controls=no",
                     "--no-video",
                     // --no-video alone still opens a window for embedded cover art in this mpv build
                     "--vid=no",
                     "--audio-display=no",
                     "--force-window=no",
                     "--keep-open=yes",
                     "--terminal=no",
                     "--msg-level=all=no",
                 })
            psi.ArgumentList.Add(arg);
        var process = Process.Start(psi) ?? throw new InvalidOperationException("Failed to start mpv");
        _process = process;
        ChildProcessJob.Attach(process); // mpv dies with us, even on a crash or force-kill

        // Poll for the pipe instead of sleeping a fixed second.
        var pipe = new NamedPipeClientStream(".", pipeName, PipeDirection.InOut, PipeOptions.Asynchronous);
        try
        {
            var deadline = DateTime.UtcNow.AddSeconds(5);
            while (true)
            {
                try
                {
                    await pipe.ConnectAsync(100, ct);
                    break;
                }
                catch (TimeoutException) when (DateTime.UtcNow < deadline && !process.HasExited)
                {
                }
            }
        }
        catch (Exception ex)
        {
            // Do not leave a half-started mpv behind: the next StartAsync begins from scratch.
            pipe.Dispose();
            Teardown();
            if (ex is TimeoutException) throw new InvalidOperationException("mpv did not open its control pipe", ex);
            throw;
        }

        _pipe = pipe;
        _writer = new StreamWriter(pipe, new UTF8Encoding(false)) { AutoFlush = true, NewLine = "\n" };
        var cts = new CancellationTokenSource();
        _readCts = cts;
        _ = Task.Run(() => ReadLoopAsync(pipe, cts.Token));
        _ = Task.Run(() => PollTimePosAsync(cts.Token));

        // time-pos is polled (see PollTimePosAsync) instead of observed: mpv pushes it ~16x/s,
        // and parsing that stream costs more CPU than the UI needs.
        await SendAsync("observe_property", 2, "pause");
        await SendAsync("observe_property", 3, "eof-reached");
        await SendAsync("observe_property", 4, "duration");
    }

    private async Task ReadLoopAsync(NamedPipeClientStream pipe, CancellationToken ct)
    {
        try
        {
            using var reader = new StreamReader(pipe, Encoding.UTF8, false, 4096, leaveOpen: true);
            while (!ct.IsCancellationRequested)
            {
                var line = await reader.ReadLineAsync(ct);
                if (line is null) break;
                if (line.Length == 0) continue;
                // One odd message or a throwing subscriber must not end the loop: the player would go deaf.
                try { HandleMessage(line); }
                catch (Exception ex) when (ex is not OperationCanceledException) { Trace.WriteLine($"mpv message: {ex.Message}"); }
            }
        }
        catch (OperationCanceledException) { }
        catch (IOException) { }
        catch (ObjectDisposedException) { }

        if (ct.IsCancellationRequested || _closing) return; // closed by us
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

    /// <summary>Releases the pipe and the process. Safe to call when nothing is running.</summary>
    private void Teardown()
    {
        _playing = false;
        _currentEntryId = -1;
        try { _readCts?.Cancel(); } catch { }
        _readCts = null;
        _writer = null;
        try { _pipe?.Dispose(); } catch { }
        _pipe = null;
        var process = _process;
        _process = null;
        if (process is not null)
        {
            try { if (!process.HasExited) process.Kill(); } catch { }
            try { process.Dispose(); } catch { }
        }
        FailPending();
    }

    private const int TimePosRequestId = 1000;
    private const string TimePosRequest = "{\"command\":[\"get_property\",\"time-pos\"],\"request_id\":1000}";

    /// <summary>Asks mpv for the position twice a second, only while something is playing.</summary>
    private async Task PollTimePosAsync(CancellationToken ct)
    {
        using var timer = new PeriodicTimer(TimeSpan.FromMilliseconds(500));
        try
        {
            while (await timer.WaitForNextTickAsync(ct))
            {
                if (_playing) await WriteLineAsync(TimePosRequest);
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
            case "duration": DurationChanged?.Invoke(data.GetDouble()); break;
            case "pause":
                _playing = !data.GetBoolean();
                PauseChanged?.Invoke(!_playing);
                break;
            case "eof-reached" when data.ValueKind == JsonValueKind.True: EndOfFile?.Invoke(); break;
        }
    }

    private Task SendAsync(string command, params object[] args) => SendAsync(null, command, args);

    private async Task SendAsync(int? requestId, string command, params object[] args)
    {
        if (_writer is null) return;
        using var ms = new MemoryStream();
        using (var w = new Utf8JsonWriter(ms))
        {
            w.WriteStartObject();
            if (requestId is { } id) w.WriteNumber("request_id", id);
            w.WriteStartArray("command");
            w.WriteStringValue(command);
            foreach (var a in args)
            {
                switch (a)
                {
                    case string s: w.WriteStringValue(s); break;
                    case bool b: w.WriteBooleanValue(b); break;
                    case int i: w.WriteNumberValue(i); break;
                    case double d: w.WriteNumberValue(d); break;
                    default: w.WriteStringValue(a.ToString()); break;
                }
            }
            w.WriteEndArray();
            w.WriteEndObject();
        }
        await WriteLineAsync(Encoding.UTF8.GetString(ms.ToArray()));
    }

    private async Task WriteLineAsync(string json)
    {
        var writer = _writer;
        if (writer is null) return;
        await _writeLock.WaitAsync();
        try { await writer.WriteLineAsync(json); }
        catch (IOException) { }
        catch (ObjectDisposedException) { }
        finally { _writeLock.Release(); }
    }

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
        _pendingSeek = startSeconds;
        var id = Interlocked.Increment(ref _nextRequestId);
        _loadRequestId = id;
        await SendAsync(id, "loadfile", filePath, "replace");
        await SendAsync("set_property", "pause", false);
    }

    public Task SetPausedAsync(bool paused) => SendAsync("set_property", "pause", paused);
    public Task TogglePauseAsync() => SendAsync("cycle", "pause");
    public async Task SeekAsync(double seconds)
    {
        await SendAsync("seek", seconds, "absolute");
        await WriteLineAsync(TimePosRequest); // refresh position right away
    }
    public Task SetVolumeAsync(double volume01) => SendAsync("set_property", "volume", Math.Clamp(volume01, 0, 1) * 100.0);
    public Task SetMuteAsync(bool muted) => SendAsync("set_property", "mute", muted);
    public Task StopAsync()
    {
        _currentEntryId = -1;
        return SendAsync("stop");
    }

    /// <summary>Switch between shared and WASAPI exclusive output without restarting mpv.</summary>
    public async Task SetExclusiveAsync(bool exclusive)
    {
        if (_exclusive == exclusive) return;
        _exclusive = exclusive;
        await SendAsync("set_property", "audio-exclusive", exclusive);
        await SendAsync("ao-reload");
    }

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

    /// <summary>Output devices as mpv sees them for --ao=wasapi ("auto" first).</summary>
    public async Task<IReadOnlyList<AudioDevice>> GetAudioDevicesAsync()
    {
        var list = new List<AudioDevice>();
        if (await RequestAsync("get_property", "audio-device-list") is not { ValueKind: JsonValueKind.Array } arr) return list;
        foreach (var d in arr.EnumerateArray())
        {
            var name = d.TryGetProperty("name", out var n) ? n.GetString() : null;
            // mpv lists every audio backend's devices; only wasapi ones work with --ao=wasapi.
            if (string.IsNullOrEmpty(name) || (name != "auto" && !name.StartsWith("wasapi/", StringComparison.Ordinal))) continue;
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
        await SendAsync("set_property", "audio-device", name);
        await SendAsync("ao-reload");
    }

    public async ValueTask DisposeAsync()
    {
        _closing = true; // the read loop ending is expected from here on
        try { await SendAsync("quit").ConfigureAwait(false); } catch { }
        try { _process?.WaitForExit(500); } catch { }
        Teardown();
    }
}

/// <summary>An mpv output device: Name is what --audio-device takes, Description is the friendly label.</summary>
public sealed record AudioDevice(string Name, string Description);
