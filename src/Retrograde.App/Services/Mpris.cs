using System.ComponentModel;
using Avalonia.Threading;
using Retrograde.App.ViewModels;
using Tmds.DBus.Protocol;

namespace Retrograde.App.Services;

/// <summary>
/// MPRIS 2 over the session bus (the Linux counterpart of SMTC): media keys, playerctl and the shell's
/// media widget see title / artist / album / cover and can play, pause, skip and seek. Owns
/// org.mpris.MediaPlayer2.retrograde. Uses Tmds.DBus.Protocol (shipped with Avalonia), no extra dependency.
/// </summary>
public static class Mpris
{
    private const string BusName = "org.mpris.MediaPlayer2.retrograde";
    private const string ObjPath = "/org/mpris/MediaPlayer2";
    private const string IfRoot = "org.mpris.MediaPlayer2";
    private const string IfPlayer = "org.mpris.MediaPlayer2.Player";
    private const string IfProps = "org.freedesktop.DBus.Properties";

    private static DBusConnection? _conn;
    private static PlayerViewModel? _player;
    private static Action? _raise;
    private static Action? _quit;
    private static double _lastPos;

    public static void Start(PlayerViewModel player, Action raise, Action quit)
    {
        _player = player;
        _raise = raise;
        _quit = quit;
        _ = StartAsync();
    }

    private static async Task StartAsync()
    {
        try
        {
            var addr = DBusAddress.Session;
            if (addr is null) return; // no session bus: the app works without media keys
            var conn = new DBusConnection(new DBusConnectionOptions(addr)
            {
                OnException = ctx => Retrograde.Core.ErrorLog.Write("mpris", ctx.Exception ?? new Exception("D-Bus error")),
            });
            await conn.ConnectAsync();
            conn.AddMethodHandler(new Handler());
            if (!await conn.TryRequestNameAsync(BusName, RequestNameOptions.ReplaceExisting | RequestNameOptions.AllowReplacement))
            {
                conn.Dispose();
                return;
            }
            _conn = conn;
            Dispatcher.UIThread.Post(() => _player!.PropertyChanged += OnPlayerChanged);
        }
        catch (Exception ex)
        {
            Retrograde.Core.ErrorLog.Write("mpris", ex);
            _conn?.Dispose();
            _conn = null;
        }
    }

    public static void Stop()
    {
        if (_player is not null) _player.PropertyChanged -= OnPlayerChanged;
        try { _conn?.Dispose(); } catch { }
        _conn = null;
    }

    // ---------- Player -> bus ----------

    private static void OnPlayerChanged(object? sender, PropertyChangedEventArgs e)
    {
        var p = _player;
        if (_conn is null || p is null) return;
        switch (e.PropertyName)
        {
            case nameof(PlayerViewModel.CurrentTrack):
                var has = p.CurrentTrack is not null;
                Changed(new()
                {
                    ["Metadata"] = MetadataValue(),
                    ["PlaybackStatus"] = Status(),
                    ["CanPlay"] = has,
                    ["CanPause"] = has,
                    ["CanSeek"] = has,
                });
                break;
            case nameof(PlayerViewModel.IsPlaying):
                Changed(new() { ["PlaybackStatus"] = Status() });
                break;
            case nameof(PlayerViewModel.Duration):
                Changed(new() { ["Metadata"] = MetadataValue() });
                break;
            case nameof(PlayerViewModel.Position):
                // Clients read Position on demand; only a jump (seek, restart) is announced.
                if (Math.Abs(p.Position - _lastPos) > 2) Seeked(p.Position);
                _lastPos = p.Position;
                break;
        }
    }

    private static string Status() => _player?.CurrentTrack is null ? "Stopped" : _player.IsPlaying ? "Playing" : "Paused";

    private static VariantValue MetadataValue()
    {
        var d = new Dict<string, VariantValue>();
        var t = _player?.CurrentTrack;
        if (t is not null)
        {
            d.Add("mpris:trackid", VariantValue.ObjectPath(new ObjectPath("/org/retrograde/track/" + new string(t.Id.Where(char.IsAsciiLetterOrDigit).ToArray()))));
            d.Add("mpris:length", (long)(Math.Max(0, _player!.Duration) * 1_000_000));
            d.Add("xesam:title", t.Title ?? "");
            d.Add("xesam:artist", VariantValue.Array(new[] { string.IsNullOrEmpty(t.Artist) ? "Unknown Artist" : t.Artist }));
            d.Add("xesam:album", string.IsNullOrEmpty(t.Album) ? "Unknown Album" : t.Album);
            var cover = t.CoverFullPath ?? t.CoverThumbPath;
            if (cover is not null && File.Exists(cover)) d.Add("mpris:artUrl", new Uri(cover).AbsoluteUri);
        }
        return d;
    }

    private static void Changed(Dictionary<string, VariantValue> props)
    {
        var conn = _conn;
        if (conn is null) return;
        try
        {
            using var w = conn.GetMessageWriter();
            w.WriteSignalHeader(null!, ObjPath, IfProps, "PropertiesChanged", "sa{sv}as");
            w.WriteString(IfPlayer);
            w.WriteDictionary(props);
            w.WriteArray(Array.Empty<string>());
            conn.TrySendMessage(w.CreateMessage());
        }
        catch { }
    }

    private static void Seeked(double seconds)
    {
        var conn = _conn;
        if (conn is null) return;
        try
        {
            using var w = conn.GetMessageWriter();
            w.WriteSignalHeader(null!, ObjPath, IfPlayer, "Seeked", "x");
            w.WriteInt64((long)(seconds * 1_000_000));
            conn.TrySendMessage(w.CreateMessage());
        }
        catch { }
    }

    // ---------- Bus -> player ----------

    private sealed class Handler : IPathMethodHandler
    {
        public string Path => ObjPath;
        public bool HandlesChildPaths => false;

        public ValueTask HandleMethodAsync(MethodContext context)
        {
            try
            {
                if (context.IsDBusIntrospectRequest)
                {
                    context.ReplyIntrospectXml([System.Text.Encoding.UTF8.GetBytes(IntrospectXml)]);
                    return default;
                }
                var m = context.Request;
                var iface = m.InterfaceAsString ?? "";
                var member = m.MemberAsString ?? "";
                if (iface == IfProps) HandleProps(context, member);
                else if (iface == IfRoot) HandleRoot(context, member);
                else if (iface == IfPlayer) HandlePlayer(context, member);
                else context.ReplyUnknownMethodError();
            }
            catch (Exception ex)
            {
                Retrograde.Core.ErrorLog.Write("mpris", ex);
                context.ReplyError("org.freedesktop.DBus.Error.Failed", ex.Message);
            }
            return default;
        }
    }

    private static void ReplyEmpty(MethodContext c)
    {
        using var w = c.CreateReplyWriter(null!);
        c.Reply(w.CreateMessage());
    }

    private static void HandleRoot(MethodContext c, string member)
    {
        switch (member)
        {
            case "Raise": _raise?.Invoke(); ReplyEmpty(c); break;
            case "Quit": Dispatcher.UIThread.Post(() => _quit?.Invoke()); ReplyEmpty(c); break;
            default: c.ReplyUnknownMethodError(); break;
        }
    }

    private static void HandlePlayer(MethodContext c, string member)
    {
        var p = _player;
        if (p is null || member is not ("Play" or "Pause" or "PlayPause" or "Stop" or "Next" or "Previous" or "Seek" or "SetPosition"))
        {
            c.ReplyUnknownMethodError();
            return;
        }
        long arg = 0;
        if (member == "Seek") arg = c.Request.GetBodyReader().ReadInt64();
        else if (member == "SetPosition")
        {
            var r = c.Request.GetBodyReader();
            r.ReadObjectPathAsString(); // track id: not checked
            arg = r.ReadInt64();
        }
        Dispatcher.UIThread.Post(async () =>
        {
            switch (member)
            {
                case "Play": if (!p.IsPlaying) p.TogglePlayCommand.Execute(null); break;
                case "Pause": case "Stop": if (p.IsPlaying) p.TogglePlayCommand.Execute(null); break;
                case "PlayPause": p.TogglePlayCommand.Execute(null); break;
                case "Next": p.NextCommand.Execute(null); break;
                case "Previous": p.PrevCommand.Execute(null); break;
                case "Seek": await p.SeekAsync(Math.Clamp(p.Position + arg / 1_000_000.0, 0, Math.Max(0, p.Duration))); break;
                case "SetPosition": await p.SeekAsync(Math.Clamp(arg / 1_000_000.0, 0, Math.Max(0, p.Duration))); break;
            }
        });
        ReplyEmpty(c);
    }

    private static void HandleProps(MethodContext c, string member)
    {
        var r = c.Request.GetBodyReader();
        var iface = r.ReadString();
        switch (member)
        {
            case "Get":
            {
                var name = r.ReadString();
                if (PropValue(iface, name) is not { } v) { c.ReplyError("org.freedesktop.DBus.Error.UnknownProperty", name); return; }
                using var w = c.CreateReplyWriter("v");
                w.WriteVariant(v);
                c.Reply(w.CreateMessage());
                break;
            }
            case "GetAll":
            {
                var all = new Dictionary<string, VariantValue>();
                foreach (var name in Names(iface))
                    if (PropValue(iface, name) is { } v) all[name] = v;
                using var w = c.CreateReplyWriter("a{sv}");
                w.WriteDictionary(all);
                c.Reply(w.CreateMessage());
                break;
            }
            default: c.ReplyUnknownMethodError(); break; // Set: every property is read-only
        }
    }

    private static string[] Names(string iface) => iface switch
    {
        IfRoot => ["CanQuit", "CanRaise", "HasTrackList", "Identity", "DesktopEntry", "SupportedUriSchemes", "SupportedMimeTypes"],
        IfPlayer => ["PlaybackStatus", "Rate", "MinimumRate", "MaximumRate", "Metadata", "Volume", "Position",
            "CanGoNext", "CanGoPrevious", "CanPlay", "CanPause", "CanSeek", "CanControl"],
        _ => [],
    };

    private static VariantValue? PropValue(string iface, string name)
    {
        var p = _player;
        var has = p?.CurrentTrack is not null;
        if (iface == IfRoot)
        {
            return name switch
            {
                "CanQuit" or "CanRaise" => true,
                "HasTrackList" => false,
                "Identity" => "Retrograde",
                "DesktopEntry" => "retrograde",
                "SupportedUriSchemes" or "SupportedMimeTypes" => VariantValue.Array(Array.Empty<string>()),
                _ => (VariantValue?)null,
            };
        }
        if (iface != IfPlayer) return null;
        return name switch
        {
            "PlaybackStatus" => Status(),
            "Rate" or "MinimumRate" or "MaximumRate" => 1.0,
            "Volume" => p?.Volume ?? 1.0,
            "Position" => (long)((p?.Position ?? 0) * 1_000_000),
            "CanGoNext" or "CanGoPrevious" or "CanControl" => true,
            "CanPlay" or "CanPause" or "CanSeek" => has,
            "Metadata" => MetadataValue(),
            _ => (VariantValue?)null,
        };
    }

    private const string IntrospectXml = """
        <interface name="org.mpris.MediaPlayer2"><method name="Raise"/><method name="Quit"/>
            <property name="CanQuit" type="b" access="read"/><property name="CanRaise" type="b" access="read"/>
            <property name="HasTrackList" type="b" access="read"/><property name="Identity" type="s" access="read"/>
            <property name="DesktopEntry" type="s" access="read"/>
            <property name="SupportedUriSchemes" type="as" access="read"/><property name="SupportedMimeTypes" type="as" access="read"/>
          </interface>
          <interface name="org.mpris.MediaPlayer2.Player">
            <method name="Next"/><method name="Previous"/><method name="Pause"/><method name="PlayPause"/><method name="Stop"/><method name="Play"/>
            <method name="Seek"><arg type="x" direction="in"/></method>
            <method name="SetPosition"><arg type="o" direction="in"/><arg type="x" direction="in"/></method>
            <signal name="Seeked"><arg type="x" direction="out"/></signal>
            <property name="PlaybackStatus" type="s" access="read"/><property name="Rate" type="d" access="read"/>
            <property name="MinimumRate" type="d" access="read"/><property name="MaximumRate" type="d" access="read"/>
            <property name="Metadata" type="a{sv}" access="read"/><property name="Volume" type="d" access="read"/>
            <property name="Position" type="x" access="read"/>
            <property name="CanGoNext" type="b" access="read"/><property name="CanGoPrevious" type="b" access="read"/>
            <property name="CanPlay" type="b" access="read"/><property name="CanPause" type="b" access="read"/>
            <property name="CanSeek" type="b" access="read"/><property name="CanControl" type="b" access="read"/>
          </interface>
        """;
}
