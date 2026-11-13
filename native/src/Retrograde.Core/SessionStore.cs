using System.Text.Json;
using System.Text.Json.Serialization;

namespace Retrograde.Core;

/// <summary>Where playback stood: small, rewritten every few seconds while playing.</summary>
public sealed class SessionState
{
    /// <summary>Id (file path) of the current track; the index alone goes stale when tracks leave the library.</summary>
    public string? CurrentId { get; set; }
    public int Index { get; set; } = -1;
    public double Position { get; set; }
    public bool Shuffle { get; set; }
    public int RepeatMode { get; set; }
}

/// <summary>
/// The playback session kept across runs (playback_session in the Electron build's localStorage).
/// The queue can hold thousands of paths, so it lives in its own file and is only rewritten when
/// the queue itself changes; the position goes to the small state file.
/// </summary>
public static class SessionStore
{
    private static string StatePath => Path.Combine(CoverPaths.AppDir, "native-session.json");
    private static string QueuePath => Path.Combine(CoverPaths.AppDir, "native-queue.json");

    public static (SessionState State, List<string> Queue)? Load()
    {
        try
        {
            if (!File.Exists(StatePath) || !File.Exists(QueuePath)) return null;
            var state = JsonSerializer.Deserialize(File.ReadAllText(StatePath), SessionJsonContext.Default.SessionState);
            var queue = JsonSerializer.Deserialize(File.ReadAllText(QueuePath), SessionJsonContext.Default.ListString);
            return state is null || queue is null ? null : (state, queue);
        }
        catch { return null; } // a broken session is not worth an error: start with an empty player
    }

    public static void SaveState(SessionState state) =>
        Write(StatePath, JsonSerializer.Serialize(state, SessionJsonContext.Default.SessionState));

    public static void SaveQueue(IReadOnlyList<string> ids) =>
        Write(QueuePath, JsonSerializer.Serialize(ids.ToList(), Relaxed.ListString));

    private static readonly object Gate = new();

    private static void Write(string path, string json)
    {
        try
        {
            lock (Gate)
            {
                Directory.CreateDirectory(CoverPaths.AppDir);
                var tmp = path + ".tmp";
                File.WriteAllText(tmp, json);
                File.Move(tmp, path, overwrite: true);
            }
        }
        catch { /* best effort */ }
    }

    // Paths with CJK names stay readable instead of \uXXXX escapes.
    private static readonly SessionJsonContext Relaxed = new(new JsonSerializerOptions
    {
        Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping,
    });
}

[JsonSerializable(typeof(SessionState))]
[JsonSerializable(typeof(List<string>))]
internal partial class SessionJsonContext : JsonSerializerContext;
