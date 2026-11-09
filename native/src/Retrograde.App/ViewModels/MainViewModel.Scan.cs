using CommunityToolkit.Mvvm.ComponentModel;
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
        IsScanning = true;
        ApplyProgress(new ScanProgress("discovering", 0, 0, "Discovering files..."));
        try
        {
            var progress = new Progress<ScanProgress>(ApplyProgress); // created on the UI thread, so reports land there
            var result = await Scanner.ScanAsync(paths, progress);
            if (result.AddedTracks > 0 || result.Covers > 0)
            {
                await LoadLibraryAsync();
                ShowToast($"SCAN_COMPLETE // {result.AddedTracks} TRACKS, {result.NewAlbums} NEW ALBUMS", ToastKind.Success);
            }
            else
            {
                ShowToast("SCAN_COMPLETE // NO NEW TRACKS", ToastKind.Info);
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
