using System.IO.Pipes;
using System.Text;

namespace Retrograde.App.Services;

/// <summary>
/// One running instance per user (app.requestSingleInstanceLock in the Electron build).
/// A second launch forwards its command-line paths over a named pipe and exits;
/// the first instance shows its window and scans those paths.
/// </summary>
public static class SingleInstance
{
    private static readonly string Id = "Retrograde.Native." + Environment.UserName;
    private static Mutex? _mutex;

    /// <summary>True when this process is the first instance. Otherwise the args were handed over.</summary>
    public static bool Acquire(string[] args)
    {
        _mutex = new Mutex(initiallyOwned: true, Id, out var createdNew);
        if (createdNew) return true;
        try
        {
            using var client = new NamedPipeClientStream(".", Id, PipeDirection.Out);
            client.Connect(2000);
            var payload = Encoding.UTF8.GetBytes(string.Join('\n', args.Select(Path.GetFullPath)));
            client.Write(payload);
        }
        catch { /* first instance is starting or busy: just exit */ }
        return false;
    }

    /// <summary>Listens for later launches; <paramref name="onActivate"/> gets their paths (possibly empty).</summary>
    public static void Listen(Action<string[]> onActivate)
    {
        _ = Task.Run(async () =>
        {
            while (true)
            {
                try
                {
                    await using var server = new NamedPipeServerStream(Id, PipeDirection.In, 1, PipeTransmissionMode.Byte, PipeOptions.Asynchronous | PipeOptions.CurrentUserOnly);
                    await server.WaitForConnectionAsync();
                    using var ms = new MemoryStream();
                    await server.CopyToAsync(ms);
                    var text = Encoding.UTF8.GetString(ms.ToArray());
                    onActivate(text.Split('\n', StringSplitOptions.RemoveEmptyEntries));
                }
                catch (IOException) { }
                catch (ObjectDisposedException) { return; }
            }
        });
    }
}
