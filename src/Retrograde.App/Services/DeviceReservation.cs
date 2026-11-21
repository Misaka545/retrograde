using Tmds.DBus.Protocol;

namespace Retrograde.App.Services;

/// <summary>
/// The freedesktop device reservation protocol (org.freedesktop.ReserveDevice1): how programs share a sound
/// card. PipeWire (through WirePlumber) holds the card's name on the session bus while it uses the card; a
/// program that wants the card to itself, like a direct bit-perfect output, asks the holder to let go and
/// then owns the name for as long as it plays. The holder gives the card back when the name is released
/// (or this process ends), so other programs get that card back by themselves afterwards.
/// </summary>
public sealed class DeviceReservation : IDisposable
{
    private const string Iface = "org.freedesktop.ReserveDevice1";
    // Higher than PipeWire's (-20), so it hands the card over; a program that outranks this one can still ask for it back.
    private const int Priority = 100;

    private DBusConnection? _conn;
    private readonly string _name;
    public int Card { get; }

    private DeviceReservation(int card, DBusConnection conn)
    {
        Card = card;
        _conn = conn;
        _name = NameOf(card);
    }

    private static string NameOf(int card) => $"org.freedesktop.ReserveDevice1.Audio{card}";
    private static string PathOf(int card) => $"/org/freedesktop/ReserveDevice1/Audio{card}";

    /// <summary>Takes sound card <paramref name="card"/> from whoever holds it. Null Reservation: nobody could be asked, or it said no; Holder names who is in the way.</summary>
    public static async Task<(DeviceReservation? Reservation, string? Holder)> AcquireAsync(int card)
    {
        var name = NameOf(card);
        DBusConnection? conn = null;
        try
        {
            conn = await OpenAsync(card);
            if (conn is null) return (null, "no session bus");
            if (await conn.TryRequestNameAsync(name, RequestNameOptions.None))
            {
                var first = new DeviceReservation(card, conn);
                conn = null;
                return (first, null);
            }
            // Held by someone else: ask them to let go.
            bool released;
            try { released = await RequestReleaseAsync(conn, name, PathOf(card)); }
            catch { released = false; }
            if (!released) return (null, await HolderAsync(conn, name, PathOf(card)) ?? "another program");
            // The holder has let go of the card and gave the name up while it answered, but PipeWire asks for the
            // name again within a millisecond to be first in line for the card. It allows replacement, so take it over.
            // A connection asks for a name once: every attempt gets its own.
            conn.Dispose();
            conn = null;
            for (var i = 0; i < 20; i++)
            {
                var next = await OpenAsync(card);
                if (next is null) break;
                try
                {
                    if (await next.TryRequestNameAsync(name, RequestNameOptions.ReplaceExisting)) { var won = new DeviceReservation(card, next); next = null; return (won, null); }
                }
                finally { next?.Dispose(); }
                await Task.Delay(100);
            }
            return (null, "another program");
        }
        catch (Exception ex)
        {
            Retrograde.Core.ErrorLog.Write("reserve-device", ex);
            return (null, null);
        }
        finally
        {
            conn?.Dispose();
        }
    }

    private static async Task<DBusConnection?> OpenAsync(int card)
    {
        var addr = DBusAddress.Session;
        if (addr is null) return null; // no session bus: the caller decides whether to try the card anyway
        var conn = new DBusConnection(new DBusConnectionOptions(addr)
        {
            OnException = ctx => Retrograde.Core.ErrorLog.Write("reserve-device", ctx.Exception ?? new Exception("D-Bus error")),
        });
        try
        {
            await conn.ConnectAsync();
            conn.AddMethodHandler(new Handler(PathOf(card)));
            return conn;
        }
        catch
        {
            conn.Dispose();
            throw;
        }
    }

    private static Task<bool> RequestReleaseAsync(DBusConnection conn, string name, string path)
    {
        using var w = conn.GetMessageWriter();
        w.WriteMethodCallHeader(name, path, Iface, "RequestRelease", "i");
        w.WriteInt32(Priority);
        return conn.CallMethodAsync(w.CreateMessage(), (Message m, object? _) => m.GetBodyReader().ReadBool());
    }

    /// <summary>The name of the program holding the card, for the message to the user.</summary>
    private static async Task<string?> HolderAsync(DBusConnection conn, string name, string path)
    {
        try
        {
            var v = await conn.CallMethodAsync(GetNameMessage(conn, name, path), (Message m, object? _) => m.GetBodyReader().ReadVariantValue());
            return v.GetString();
        }
        catch { return null; }
    }

    private static MessageBuffer GetNameMessage(DBusConnection conn, string name, string path)
    {
        using var w = conn.GetMessageWriter();
        w.WriteMethodCallHeader(name, path, "org.freedesktop.DBus.Properties", "Get", "ss");
        w.WriteString(Iface);
        w.WriteString("ApplicationName");
        return w.CreateMessage();
    }

    /// <summary>Gives the card back: PipeWire takes it again by itself.</summary>
    public void Dispose()
    {
        var conn = _conn;
        _conn = null;
        try { conn?.Dispose(); } catch { }
    }

    /// <summary>Answers other programs that ask for the card: no. The card stays here until released.</summary>
    private sealed class Handler(string path) : IPathMethodHandler
    {
        public string Path { get; } = path;
        public bool HandlesChildPaths => false;

        private const string Props = "org.freedesktop.DBus.Properties";

        public ValueTask HandleMethodAsync(MethodContext context)
        {
            try
            {
                if (context.IsDBusIntrospectRequest)
                {
                    context.ReplyIntrospectXml([System.Text.Encoding.UTF8.GetBytes(
                        "<interface name=\"org.freedesktop.ReserveDevice1\">" +
                        "<method name=\"RequestRelease\"><arg type=\"i\" name=\"priority\" direction=\"in\"/><arg type=\"b\" name=\"result\" direction=\"out\"/></method>" +
                        "<property name=\"Priority\" type=\"i\" access=\"read\"/><property name=\"ApplicationName\" type=\"s\" access=\"read\"/>" +
                        "<property name=\"ApplicationDeviceName\" type=\"s\" access=\"read\"/></interface>")]);
                    return default;
                }
                var iface = context.Request.InterfaceAsString;
                var member = context.Request.MemberAsString;
                if (iface == Iface && member == "RequestRelease")
                {
                    using var w = context.CreateReplyWriter("b");
                    w.WriteBool(false);
                    context.Reply(w.CreateMessage());
                }
                else if (iface == Props && member == "Get")
                {
                    var r = context.Request.GetBodyReader();
                    var forIface = r.ReadString();
                    var prop = r.ReadString();
                    VariantValue? v = null;
                    if (forIface == Iface)
                    {
                        if (prop == "Priority") v = VariantValue.Int32(Priority);
                        else if (prop == "ApplicationName") v = VariantValue.String("Retrograde");
                        else if (prop == "ApplicationDeviceName") v = VariantValue.String("Direct output (bit-perfect)");
                    }
                    if (v is null) { context.ReplyError("org.freedesktop.DBus.Error.UnknownProperty", prop); return default; }
                    using var w = context.CreateReplyWriter("v");
                    w.WriteVariant(v.Value);
                    context.Reply(w.CreateMessage());
                }
                else context.ReplyUnknownMethodError();
            }
            catch (Exception ex)
            {
                Retrograde.Core.ErrorLog.Write("reserve-device", ex);
                context.ReplyError("org.freedesktop.DBus.Error.Failed", ex.Message);
            }
            return default;
        }
    }
}
