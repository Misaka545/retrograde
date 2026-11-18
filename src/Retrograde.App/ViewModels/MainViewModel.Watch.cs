using Avalonia.Threading;
using Retrograde.Core;

namespace Retrograde.App.ViewModels;

/// <summary>
/// Folder watching: new and changed audio files under the library folders are picked up a few seconds
/// after they land, quietly (no scan card, a toast when something changed). Removed files are left to
/// F5, which asks before dropping tracks: a drive going offline must not empty the library.
/// </summary>
public partial class MainViewModel
{
    private readonly Dictionary<string, FileSystemWatcher> _watchers = new(StringComparer.OrdinalIgnoreCase);
    private readonly HashSet<string> _pendingPaths = new(StringComparer.OrdinalIgnoreCase);
    private readonly object _pendingLock = new();
    private DispatcherTimer? _watchDebounce;
    private Task? _watchScan;

    /// <summary>Quiet time after the last file event before the pick-up runs (copies come in bursts).</summary>
    private static readonly TimeSpan WatchDelay = TimeSpan.FromSeconds(4);

    public bool IsWatchingFolders => _settings.WatchFolders;

    /// <summary>Tray menu: turn folder watching on or off (saved).</summary>
    public void SetWatchFolders(bool on)
    {
        if (_settings.WatchFolders == on) return;
        _settings.WatchFolders = on;
        _settings.Save();
        if (on) RefreshWatchers();
        else StopWatching();
    }

    private void StartWatching()
    {
        _watchDebounce ??= new DispatcherTimer { Interval = WatchDelay };
        _watchDebounce.Tick -= OnWatchDebounce;
        _watchDebounce.Tick += OnWatchDebounce;
        RefreshWatchers();
    }

    /// <summary>Watches exactly the current library roots (after a scan the set may have changed).</summary>
    private void RefreshWatchers()
    {
        if (!_settings.WatchFolders || _watchDebounce is null) return;
        var roots = TopLevelRoots(LibraryRoots().Where(Directory.Exists));
        foreach (var gone in _watchers.Keys.Where(k => !roots.Contains(k)).ToList())
        {
            _watchers[gone].Dispose();
            _watchers.Remove(gone);
        }
        foreach (var root in roots)
        {
            if (_watchers.ContainsKey(root)) continue;
            try
            {
                var w = new FileSystemWatcher(root)
                {
                    IncludeSubdirectories = true,
                    NotifyFilter = NotifyFilters.FileName | NotifyFilters.DirectoryName | NotifyFilters.LastWrite | NotifyFilters.Size,
                    InternalBufferSize = 64 * 1024,
                };
                w.Created += (_, e) => OnFileEvent(e.FullPath);
                w.Changed += (_, e) => OnFileEvent(e.FullPath);
                w.Renamed += (_, e) => OnFileEvent(e.FullPath);
                // Too many events at once (a big copy): look at the whole folder instead
                w.Error += (_, _) => OnFileEvent(root);
                w.EnableRaisingEvents = true;
                _watchers[root] = w;
            }
            catch (Exception ex)
            {
                ErrorLog.Write($"watch {root}", ex); // e.g. a network share that refuses notifications
            }
        }
    }

    /// <summary>
    /// Only the outermost folders: the library roots include every album folder, and a watcher already covers its
    /// subfolders (IncludeSubdirectories). One watcher per album folder meant ~130 watchers on a 130-album library,
    /// each with its own thread, and every event reported several times.
    /// </summary>
    private static HashSet<string> TopLevelRoots(IEnumerable<string> roots)
    {
        var kept = new List<string>();
        foreach (var r in roots.Select(r => Path.TrimEndingDirectorySeparator(r)).OrderBy(r => r.Length))
            if (!kept.Any(k => r.Equals(k, StringComparison.OrdinalIgnoreCase)
                               || r.StartsWith(k + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase)))
                kept.Add(r);
        return new HashSet<string>(kept, StringComparer.OrdinalIgnoreCase);
    }

    private void StopWatching()
    {
        foreach (var w in _watchers.Values) w.Dispose();
        _watchers.Clear();
        _watchDebounce?.Stop();
        lock (_pendingLock) _pendingPaths.Clear();
    }

    /// <summary>A file system event (thread pool): audio files and folders are noted, the rest ignored.</summary>
    private void OnFileEvent(string path)
    {
        var isAudio = Scanner.AudioExtensions.Contains(Path.GetExtension(path));
        // A folder moved in raises one event for the folder only; plain files other than audio are not ours
        if (!isAudio && Path.HasExtension(path) && !Directory.Exists(path)) return;
        lock (_pendingLock) _pendingPaths.Add(path);
        Dispatcher.UIThread.Post(() =>
        {
            if (_watchDebounce is null) return;
            _watchDebounce.Stop(); // restart: wait for the burst to end
            _watchDebounce.Start();
        });
    }

    private void OnWatchDebounce(object? sender, EventArgs e)
    {
        _watchDebounce?.Stop();
        if (IsScanning || _watchScan is not null)
        {
            _watchDebounce?.Start(); // a scan is running: try again after it
            return;
        }
        List<string> paths;
        lock (_pendingLock)
        {
            paths = _pendingPaths.ToList();
            _pendingPaths.Clear();
        }
        if (paths.Count > 0) _watchScan = PickUpAsync(paths);
    }

    private async Task PickUpAsync(List<string> paths)
    {
        try
        {
            // Files still being written (a copy in progress) wait for the next round
            var ready = await Task.Run(() => paths.Where(p =>
            {
                if (Directory.Exists(p)) return true;
                if (!File.Exists(p)) return false;
                if (IsReadable(p)) return true;
                lock (_pendingLock) _pendingPaths.Add(p);
                return false;
            }).ToList());
            if (_pendingPaths.Count > 0) _watchDebounce?.Start();
            if (ready.Count == 0) return;

            var result = await Scanner.ScanAsync(ready, null);
            if (!result.Changed) return;
            await LoadLibraryAsync();
            if (HasSearchQuery) RebuildSearch();
            AfterCollectionsChanged();
            var parts = new List<string>();
            if (result.AddedTracks > 0) parts.Add($"{result.AddedTracks} NEW");
            if (result.UpdatedTracks > 0) parts.Add($"{result.UpdatedTracks} UPDATED");
            if (result.Covers > 0) parts.Add($"{result.Covers} COVERS");
            if (parts.Count > 0) ShowToast($"LIBRARY_UPDATED // {string.Join(", ", parts)}", ToastKind.Success);
        }
        catch (Exception ex)
        {
            ErrorLog.Write("watch pick-up", ex);
        }
        finally
        {
            _watchScan = null;
            RefreshWatchers();
        }
    }

    private static bool IsReadable(string path)
    {
        try
        {
            using var _ = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read);
            return true;
        }
        catch (IOException) { return false; }
        catch (UnauthorizedAccessException) { return false; }
    }
}
