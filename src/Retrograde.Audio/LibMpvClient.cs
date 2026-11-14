using System.Globalization;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;

namespace Retrograde.Audio;

/// <summary>
/// mpv inside this process (libmpv-2.dll) instead of a background mpv.exe. It takes the same commands
/// as mpv's JSON IPC and reports back in the same JSON shape (replies with request_id, events with
/// "event"), so <see cref="MpvEngine"/> drives both the same way. Events arrive on one background thread.
/// </summary>
internal sealed unsafe class LibMpvClient : IDisposable
{
    private const string Lib = "libmpv-2";
    private static IntPtr _libHandle;
    private static readonly object LoadLock = new();

    private IntPtr _handle;
    private readonly Thread _thread;
    private readonly Action<string> _onMessage;
    private readonly Action _onShutdown;
    private volatile bool _disposing;
    private volatile bool _shutDown;

    public bool IsAlive => !_shutDown && _handle != IntPtr.Zero;

    /// <summary>Loads libmpv-2.dll from the given path once per process; false if it is missing or broken.</summary>
    public static bool TryLoad(string dllPath)
    {
        lock (LoadLock)
        {
            if (_libHandle != IntPtr.Zero) return true;
            if (!File.Exists(dllPath) || !NativeLibrary.TryLoad(dllPath, out var handle)) return false;
            _libHandle = handle;
            try
            {
                NativeLibrary.SetDllImportResolver(typeof(LibMpvClient).Assembly,
                    (name, _, _) => name == Lib ? _libHandle : IntPtr.Zero);
            }
            catch (InvalidOperationException) { /* a resolver is set already: ours */ }
            return true;
        }
    }

    public LibMpvClient(IEnumerable<KeyValuePair<string, string>> options, Action<string> onMessage, Action onShutdown)
    {
        _onMessage = onMessage;
        _onShutdown = onShutdown;
        _handle = mpv_create();
        if (_handle == IntPtr.Zero) throw new InvalidOperationException("libmpv: mpv_create failed");
        foreach (var (name, value) in options)
            mpv_set_option_string(_handle, name, value); // an option this build lacks is skipped, as on the command line
        var err = mpv_initialize(_handle);
        if (err < 0)
        {
            mpv_terminate_destroy(_handle);
            _handle = IntPtr.Zero;
            throw new InvalidOperationException("libmpv: " + ErrorString(err));
        }
        _thread = new Thread(EventLoop) { IsBackground = true, Name = "libmpv events" };
        _thread.Start();
    }

    // ---------- Requests (any thread) ----------

    /// <summary>An mpv command; its reply comes back as a message with request_id = userData (0: no reply).</summary>
    public void Command(ulong userData, IReadOnlyList<string> args)
    {
        if (!IsAlive) return;
        var ptrs = new IntPtr[args.Count + 1];
        try
        {
            for (int i = 0; i < args.Count; i++) ptrs[i] = Marshal.StringToCoTaskMemUTF8(args[i]);
            fixed (IntPtr* p = ptrs) mpv_command_async(_handle, userData, p);
        }
        finally
        {
            foreach (var ptr in ptrs) if (ptr != IntPtr.Zero) Marshal.FreeCoTaskMem(ptr);
        }
    }

    public void GetProperty(ulong userData, string name)
    {
        if (IsAlive) mpv_get_property_async(_handle, userData, name, MpvFormat.Node);
    }

    public void SetProperty(string name, string value)
    {
        if (!IsAlive) return;
        var str = Marshal.StringToCoTaskMemUTF8(value);
        try { mpv_set_property_async(_handle, 0, name, MpvFormat.String, &str); } // mpv copies the value
        finally { Marshal.FreeCoTaskMem(str); }
    }

    public void Observe(ulong userData, string name)
    {
        if (IsAlive) mpv_observe_property(_handle, userData, name, MpvFormat.Node);
    }

    // ---------- Events ----------

    private void EventLoop()
    {
        var handle = _handle;
        while (true)
        {
            var ev = mpv_wait_event(handle, -1);
            if (_disposing) break;
            if (ev->EventId == MpvEventId.Shutdown) break;
            if (ev->EventId == MpvEventId.None) continue;
            string? json = null;
            try { json = ToJson(ev); }
            catch (Exception) { /* an odd event is skipped, the loop goes on */ }
            if (json is null) continue;
            try { _onMessage(json); }
            catch (Exception) { /* a throwing subscriber must not end the loop */ }
        }
        var unexpected = !_disposing;
        _shutDown = true;
        _handle = IntPtr.Zero;
        mpv_terminate_destroy(handle);
        if (unexpected)
        {
            try { _onShutdown(); } catch { }
        }
    }

    private static string? ToJson(MpvEvent* ev)
    {
        var buffer = new System.Buffers.ArrayBufferWriter<byte>();
        using (var w = new Utf8JsonWriter(buffer))
        {
            w.WriteStartObject();
            switch (ev->EventId)
            {
                case MpvEventId.CommandReply:
                    if (ev->ReplyUserData == 0) return null;
                    w.WriteNumber("request_id", (long)ev->ReplyUserData);
                    w.WriteString("error", ev->Error < 0 ? ErrorString(ev->Error) : "success");
                    w.WritePropertyName("data");
                    if (ev->Error >= 0 && ev->Data != null) WriteNode(w, &((MpvEventCommand*)ev->Data)->Result);
                    else w.WriteNullValue();
                    break;
                case MpvEventId.GetPropertyReply:
                    if (ev->ReplyUserData == 0) return null;
                    w.WriteNumber("request_id", (long)ev->ReplyUserData);
                    w.WriteString("error", ev->Error < 0 ? ErrorString(ev->Error) : "success");
                    w.WritePropertyName("data");
                    WriteProperty(w, ev->Error >= 0 ? (MpvEventProperty*)ev->Data : null);
                    break;
                case MpvEventId.PropertyChange:
                {
                    var prop = (MpvEventProperty*)ev->Data;
                    w.WriteString("event", "property-change");
                    w.WriteNumber("id", (long)ev->ReplyUserData);
                    w.WriteString("name", Marshal.PtrToStringUTF8(prop->Name));
                    w.WritePropertyName("data");
                    WriteProperty(w, prop);
                    break;
                }
                case MpvEventId.StartFile:
                    w.WriteString("event", "start-file");
                    w.WriteNumber("playlist_entry_id", ((MpvEventStartFile*)ev->Data)->PlaylistEntryId);
                    break;
                case MpvEventId.EndFile:
                {
                    var end = (MpvEventEndFile*)ev->Data;
                    w.WriteString("event", "end-file");
                    w.WriteString("reason", end->Reason switch
                    {
                        0 => "eof",
                        2 => "stop",
                        3 => "quit",
                        4 => "error",
                        5 => "redirect",
                        _ => "unknown",
                    });
                    w.WriteNumber("playlist_entry_id", end->PlaylistEntryId);
                    if (end->Reason == 4) w.WriteString("file_error", ErrorString(end->Error));
                    break;
                }
                case MpvEventId.FileLoaded:
                    w.WriteString("event", "file-loaded");
                    break;
                default:
                    return null;
            }
            w.WriteEndObject();
        }
        return Encoding.UTF8.GetString(buffer.WrittenSpan);
    }

    private static void WriteProperty(Utf8JsonWriter w, MpvEventProperty* prop)
    {
        if (prop == null || prop->Data == null || prop->Format != MpvFormat.Node) { w.WriteNullValue(); return; }
        WriteNode(w, (MpvNode*)prop->Data);
    }

    private static void WriteNode(Utf8JsonWriter w, MpvNode* node)
    {
        switch (node->Format)
        {
            case MpvFormat.String:
            case MpvFormat.OsdString:
                w.WriteStringValue(Marshal.PtrToStringUTF8(node->U.String)); break;
            case MpvFormat.Flag: w.WriteBooleanValue(node->U.Flag != 0); break;
            case MpvFormat.Int64: w.WriteNumberValue(node->U.Int64); break;
            case MpvFormat.Double:
                if (double.IsFinite(node->U.Double)) w.WriteNumberValue(node->U.Double); else w.WriteNullValue();
                break;
            case MpvFormat.NodeArray:
            {
                var list = (MpvNodeList*)node->U.List;
                w.WriteStartArray();
                for (int i = 0; i < list->Num; i++) WriteNode(w, &list->Values[i]);
                w.WriteEndArray();
                break;
            }
            case MpvFormat.NodeMap:
            {
                var list = (MpvNodeList*)node->U.List;
                w.WriteStartObject();
                for (int i = 0; i < list->Num; i++)
                {
                    w.WritePropertyName(Marshal.PtrToStringUTF8(list->Keys[i]) ?? "");
                    WriteNode(w, &list->Values[i]);
                }
                w.WriteEndObject();
                break;
            }
            default: w.WriteNullValue(); break;
        }
    }

    private static string ErrorString(int error) => Marshal.PtrToStringUTF8(mpv_error_string(error)) ?? error.ToString(CultureInfo.InvariantCulture);

    public void Dispose()
    {
        if (_disposing) return;
        _disposing = true;
        var handle = _handle;
        if (handle == IntPtr.Zero) return;
        mpv_wakeup(handle);
        // The event thread destroys the handle when it leaves its loop
        if (!_thread.Join(2000)) { /* stuck in mpv: leave it, the process is going away */ }
    }

    // ---------- Interop ----------

    private static class MpvFormat
    {
        public const int String = 1, OsdString = 2, Flag = 3, Int64 = 4, Double = 5, Node = 6, NodeArray = 7, NodeMap = 8;
    }

    private static class MpvEventId
    {
        public const int None = 0, Shutdown = 1, GetPropertyReply = 3, CommandReply = 5, StartFile = 6, EndFile = 7,
            FileLoaded = 8, PropertyChange = 22;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MpvEvent
    {
        public int EventId;
        public int Error;
        public ulong ReplyUserData;
        public void* Data;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MpvEventProperty
    {
        public IntPtr Name;
        public int Format;
        public void* Data;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MpvEventEndFile
    {
        public int Reason;
        public int Error;
        public long PlaylistEntryId;
        public long PlaylistInsertId;
        public int PlaylistInsertNumEntries;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MpvEventStartFile
    {
        public long PlaylistEntryId;
    }

    [StructLayout(LayoutKind.Explicit)]
    private struct MpvNodeUnion
    {
        [FieldOffset(0)] public IntPtr String;
        [FieldOffset(0)] public int Flag;
        [FieldOffset(0)] public long Int64;
        [FieldOffset(0)] public double Double;
        [FieldOffset(0)] public IntPtr List;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MpvNode
    {
        public MpvNodeUnion U;
        public int Format;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MpvNodeList
    {
        public int Num;
        public MpvNode* Values;
        public IntPtr* Keys;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MpvEventCommand
    {
        public MpvNode Result;
    }

    [DllImport(Lib)] private static extern IntPtr mpv_create();
    [DllImport(Lib)] private static extern int mpv_initialize(IntPtr ctx);
    [DllImport(Lib)] private static extern void mpv_terminate_destroy(IntPtr ctx);
    [DllImport(Lib)] private static extern void mpv_wakeup(IntPtr ctx);
    [DllImport(Lib)] private static extern IntPtr mpv_error_string(int error);
    [DllImport(Lib)] private static extern MpvEvent* mpv_wait_event(IntPtr ctx, double timeout);
    [DllImport(Lib)] private static extern int mpv_command_async(IntPtr ctx, ulong replyUserData, IntPtr* args);

    [DllImport(Lib)]
    private static extern int mpv_set_option_string(IntPtr ctx,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string name, [MarshalAs(UnmanagedType.LPUTF8Str)] string data);

    [DllImport(Lib)]
    private static extern int mpv_get_property_async(IntPtr ctx, ulong replyUserData,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string name, int format);

    [DllImport(Lib)]
    private static extern int mpv_set_property_async(IntPtr ctx, ulong replyUserData,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string name, int format, void* data);

    [DllImport(Lib)]
    private static extern int mpv_observe_property(IntPtr ctx, ulong replyUserData,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string name, int format);
}
