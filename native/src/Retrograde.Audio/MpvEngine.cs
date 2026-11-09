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
    private readonly string _pipeName = $"retrograde-native-{Environment.ProcessId}";
    private Process? _process;
    private NamedPipeClientStream? _pipe;
    private StreamWriter? _writer;
    private readonly SemaphoreSlim _writeLock = new(1, 1);
    private CancellationTokenSource? _readCts;
    private bool _exclusive;
    private double _pendingSeek;
    private volatile bool _playing;
    private string _audioDevice = "auto";
    private int _nextRequestId = 2000;
    private readonly System.Collections.Concurrent.ConcurrentDictionary<int, TaskCompletionSource<JsonElement?>> _pending = new();

    public event Action<double>? TimePosChanged;
    public event Action<double>? DurationChanged;
    public event Action<bool>? PauseChanged;
    public event Action? EndOfFile;

    public bool IsRunning => _pipe is { IsConnected: true };

    public MpvEngine(string mpvPath) => _mpvPath = mpvPath;

    public async Task StartAsync(bool exclusive, string? audioDevice = null, CancellationToken ct = default)
    {
        if (IsRunning) return;
        if (!File.Exists(_mpvPath)) throw new FileNotFoundException("mpv.exe not found", _mpvPath);
        _exclusive = exclusive;
        _audioDevice = string.IsNullOrEmpty(audioDevice) ? "auto" : audioDevice;

        var psi = new ProcessStartInfo(_mpvPath)
        {
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardOutput = false,
            RedirectStandardError = false,
        };
        foreach (var arg in new[]
                 {
                     "--idle=yes",
                     $"--input-ipc-server=\\\\.\\pipe\\{_pipeName}",
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
        _process = Process.Start(psi) ?? throw new InvalidOperationException("Failed to start mpv");
        ChildProcessJob.Attach(_process); // mpv dies with us, even on a crash or force-kill

        // Poll for the pipe instead of sleeping a fixed second.
        _pipe = new NamedPipeClientStream(".", _pipeName, PipeDirection.InOut, PipeOptions.Asynchronous);
        var deadline = DateTime.UtcNow.AddSeconds(5);
        while (true)
        {
            try
            {
                await _pipe.ConnectAsync(100, ct);
                break;
            }
            catch (TimeoutException) when (DateTime.UtcNow < deadline && !_process.HasExited)
            {
            }
        }

        _writer = new StreamWriter(_pipe, new UTF8Encoding(false)) { AutoFlush = true, NewLine = "\n" };
        _readCts = new CancellationTokenSource();
        _ = Task.Run(() => ReadLoopAsync(_pipe, _readCts.Token));
        _ = Task.Run(() => PollTimePosAsync(_readCts.Token));

        // time-pos is polled (see PollTimePosAsync) instead of observed: mpv pushes it ~16x/s,
        // and parsing that stream costs more CPU than the UI needs.
        await SendAsync("observe_property", 2, "pause");
        await SendAsync("observe_property", 3, "eof-reached");
        await SendAsync("observe_property", 4, "duration");
    }

    private async Task ReadLoopAsync(Stream pipe, CancellationToken ct)
    {
        using var reader = new StreamReader(pipe, Encoding.UTF8, false, 4096, leaveOpen: true);
        try
        {
            while (!ct.IsCancellationRequested)
            {
                var line = await reader.ReadLineAsync(ct);
                if (line is null) break;
                if (line.Length == 0) continue;
                HandleMessage(line);
            }
        }
        catch (OperationCanceledException) { }
        catch (IOException) { }
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
        try
        {
            using var doc = JsonDocument.Parse(line);
            var root = doc.RootElement;
            root.TryGetProperty("request_id", out var rid);
            if (rid.ValueKind == JsonValueKind.Number && rid.GetInt32() == TimePosRequestId)
            {
                if (root.TryGetProperty("data", out var pos) && pos.ValueKind == JsonValueKind.Number)
                    TimePosChanged?.Invoke(pos.GetDouble());
                return;
            }
            if (rid.ValueKind == JsonValueKind.Number && _pending.TryRemove(rid.GetInt32(), out var tcs))
            {
                tcs.TrySetResult(root.TryGetProperty("data", out var d) ? d.Clone() : null);
                return;
            }
            if (!root.TryGetProperty("event", out var ev)) return;
            var evName = ev.GetString();
            if (evName == "file-loaded" && _pendingSeek > 0)
            {
                var s = _pendingSeek;
                _pendingSeek = 0;
                _ = SeekAsync(s);
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
        catch (JsonException) { }
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
        if (_writer is null) return;
        await _writeLock.WaitAsync();
        try { await _writer.WriteLineAsync(json); }
        catch (IOException) { }
        catch (ObjectDisposedException) { }
        finally { _writeLock.Release(); }
    }

    /// <summary>Loads a file and starts playing. mpv keeps the pause flag across loadfile, so clear it explicitly.</summary>
    public async Task PlayAsync(string filePath, double startSeconds = 0)
    {
        _pendingSeek = startSeconds;
        await SendAsync("loadfile", filePath, "replace");
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
    public Task StopAsync() => SendAsync("stop");

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
        try { await SendAsync("quit"); } catch { }
        _readCts?.Cancel();
        try { _pipe?.Dispose(); } catch { }
        try
        {
            if (_process is { HasExited: false })
            {
                if (!_process.WaitForExit(500)) _process.Kill();
            }
        }
        catch { }
        _process?.Dispose();
    }
}

/// <summary>An mpv output device: Name is what --audio-device takes, Description is the friendly label.</summary>
public sealed record AudioDevice(string Name, string Description);
