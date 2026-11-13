using Avalonia.Media.Imaging;

namespace Retrograde.App.Services;

/// <summary>
/// Decodes cover images off the UI thread, straight to the display width (no full-size decode),
/// and keeps a bounded LRU so memory stays flat no matter how big the library is.
/// </summary>
public static class CoverLoader
{
    private const int MaxEntries = 240;
    private static readonly object Gate = new();
    private static readonly Dictionary<string, LinkedListNode<(string Key, Bitmap Bmp)>> Map = new();
    private static readonly LinkedList<(string Key, Bitmap Bmp)> Lru = new();
    private static readonly Dictionary<string, Task<Bitmap?>> InFlight = new();
    private static readonly SemaphoreSlim DecodeSlots = new(Math.Max(2, Environment.ProcessorCount / 2));

    public static Bitmap? TryGetCached(string path, int width)
    {
        var key = Key(path, width);
        lock (Gate)
        {
            if (!Map.TryGetValue(key, out var node)) return null;
            Lru.Remove(node);
            Lru.AddFirst(node);
            return node.Value.Bmp;
        }
    }

    public static Task<Bitmap?> LoadAsync(string path, int width)
    {
        var key = Key(path, width);
        lock (Gate)
        {
            if (Map.TryGetValue(key, out var node))
            {
                Lru.Remove(node);
                Lru.AddFirst(node);
                return Task.FromResult<Bitmap?>(node.Value.Bmp);
            }
            if (InFlight.TryGetValue(key, out var pending)) return pending;
            var task = DecodeAsync(key, path, width);
            InFlight[key] = task;
            return task;
        }
    }

    private static async Task<Bitmap?> DecodeAsync(string key, string path, int width)
    {
        await DecodeSlots.WaitAsync().ConfigureAwait(false);
        try
        {
            var bmp = await Task.Run(() =>
            {
                if (!File.Exists(path)) return null;
                using var fs = File.OpenRead(path);
                return Bitmap.DecodeToWidth(fs, width, BitmapInterpolationMode.MediumQuality);
            }).ConfigureAwait(false);
            if (bmp is null) return null;
            lock (Gate)
            {
                Map[key] = Lru.AddFirst((key, bmp));
                // Evicted bitmaps are left to the GC rather than disposed: one may still be on screen.
                while (Map.Count > MaxEntries)
                {
                    var last = Lru.Last!;
                    Lru.RemoveLast();
                    Map.Remove(last.Value.Key);
                }
            }
            return bmp;
        }
        catch
        {
            return null;
        }
        finally
        {
            DecodeSlots.Release();
            lock (Gate) InFlight.Remove(key);
        }
    }

    private static string Key(string path, int width) => $"{width}|{path}";
}
