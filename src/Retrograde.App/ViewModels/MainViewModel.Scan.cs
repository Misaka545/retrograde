using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Retrograde.Core;

namespace Retrograde.App.ViewModels;

/// <summary>ADD MUSIC / drag and drop: runs the scanner and shows the progress card (App.jsx scan overlay).</summary>
public partial class MainViewModel
{
    [ObservableProperty] private bool _isScanning;
    [ObservableProperty] private string _scanTitle = "";
    [ObservableProperty] private string _scanFile = "";
    [ObservableProperty] private double _scanFraction;
    [ObservableProperty] private bool _scanHasTotal;

    public async Task ScanPathsAsync(IReadOnlyList<string> paths)
    {
        if (IsScanning || paths.Count == 0) return;
        // Remember added folders: RESCAN_LIBRARY looks there for new albums later.
        var added = false;
        foreach (var p in paths)
        {
            if (!Directory.Exists(p)) continue;
            var dir = Path.GetFullPath(p);
            if (_settings.LibraryFolders.Contains(dir, StringComparer.OrdinalIgnoreCase)) continue;
            _settings.LibraryFolders.Add(dir);
            added = true;
        }
        if (added) _settings.Save();
        await RunScanAsync(paths);
    }

    /// <summary>
    /// F5 / tray menu: goes over every folder the library knows (the added folders and the folders of
    /// its albums), picks up new files, re-reads changed ones and drops tracks whose files are gone.
    /// </summary>
    [RelayCommand]
    private async Task RescanLibrary()
    {
        if (IsScanning) return;
        var dirs = new HashSet<string>(_settings.LibraryFolders, StringComparer.OrdinalIgnoreCase);
        foreach (var a in _allAlbums)
            foreach (var t in a.Album.Tracks)
                if (Path.GetDirectoryName(t.FilePath) is { Length: > 0 } d) dirs.Add(d);
        // A folder inside another listed folder is covered by that one.
        var roots = new List<string>();
        foreach (var d in dirs.OrderBy(d => d.Length))
        {
            var covered = roots.Any(r => d.Length > r.Length && d.StartsWith(r, StringComparison.OrdinalIgnoreCase)
                                         && (r.EndsWith('\\') || d[r.Length] == '\\'));
            if (!covered) roots.Add(d);
        }
        if (roots.Count == 0)
        {
            ShowToast("RESCAN // LIBRARY IS EMPTY", ToastKind.Info);
            return;
        }
        await RunScanAsync(roots);
    }

    private async Task RunScanAsync(IReadOnlyList<string> paths, bool pruneMissing = false)
    {
        IsScanning = true;
        ApplyProgress(new ScanProgress("discovering", 0, 0, "Discovering files..."));
        try
        {
            var progress = new Progress<ScanProgress>(ApplyProgress); // created on the UI thread, so reports land there
            var result = await Scanner.ScanAsync(paths, progress, pruneMissing: pruneMissing);
            if (result.Changed)
            {
                await LoadLibraryAsync();
                if (HasSearchQuery) RebuildSearch();
                AfterCollectionsChanged(); // playlists and favorites resolve against the new catalog
                var parts = new List<string>();
                if (result.AddedTracks > 0) parts.Add($"{result.AddedTracks} NEW");
                if (result.UpdatedTracks > 0) parts.Add($"{result.UpdatedTracks} UPDATED");
                if (result.RemovedTracks > 0) parts.Add($"{result.RemovedTracks} REMOVED");
                if (result.Covers > 0) parts.Add($"{result.Covers} COVERS");
                if (result.MergedAlbums > 0) parts.Add($"{result.MergedAlbums} SPLIT ALBUMS JOINED");
                ShowToast($"SCAN_COMPLETE // {string.Join(", ", parts)}", ToastKind.Success);
            }
            else if (result.MissingTracks == 0)
            {
                ShowToast("SCAN_COMPLETE // NO CHANGES", ToastKind.Info);
            }
            if (result.MissingTracks > 0)
            {
                // Files gone from disk are only dropped once the user agrees: the folder may just be moved
                // or on a drive that is offline. Playlists and favorites keep their entries either way.
                var n = result.MissingTracks;
                ConfirmDialog = new ConfirmDialogVm("WARNING: MISSING_FILES", $"{n} TRACK{(n == 1 ? "" : "S")}",
                    "NOT ON DISK. REMOVE FROM LIBRARY?", "REMOVE",
                    async () => await RunScanAsync(paths, pruneMissing: true))
                {
                    TargetLabel = "TARGETS: ",
                    TargetBrush = OrangeBrush,
                };
            }
        }
        catch (Exception ex)
        {
            ShowToast($"SCAN_FAILED // {ex.Message}", ToastKind.Error);
        }
        finally
        {
            IsScanning = false;
        }
    }

    private void ApplyProgress(ScanProgress p)
    {
        ScanTitle = p.Phase switch
        {
            "discovering" => "DISCOVERING_FILES...",
            "scanning" => $"SCANNING_METADATA [{p.Scanned}/{p.Total}]",
            "covers" => $"EXTRACTING_COVERS [{p.Scanned}/{p.Total}]",
            _ => "SYSTEM_SCANNING...",
        };
        ScanHasTotal = p.Total > 0;
        ScanFraction = p.Total > 0 ? (double)p.Scanned / p.Total : 0;
        ScanFile = p.CurrentFile;
    }
}
