using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Retrograde.Core;

namespace Retrograde.App.ViewModels;

/// <summary>Library SELECT mode and album deletion (LibraryGrid.jsx / handleBatchDelete in App.jsx).</summary>
public partial class MainViewModel
{
    private LibraryHeaderRow? _listHeader;
    private LibraryHeaderRow? _pinnedHeader;

    private LibraryHeaderRow ListHeader => _listHeader ??= new LibraryHeaderRow(this, pinned: false);
    /// <summary>The header copy pinned above the grid while SELECT mode is on.</summary>
    public LibraryHeaderRow PinnedHeader => _pinnedHeader ??= new LibraryHeaderRow(this, pinned: true);

    [ObservableProperty] private bool _isSelectMode;

    [ObservableProperty] [NotifyPropertyChangedFor(nameof(SelectionText), nameof(HasSelection))]
    private int _selectedCount;

    public string SelectionText => $"{SelectedCount} / {_allAlbums.Count} SELECTED";
    public bool HasSelection => SelectedCount > 0;

    partial void OnIsSelectModeChanged(bool value)
    {
        foreach (var a in _allAlbums)
        {
            a.IsSelectable = value;
            a.IsSelected = false;
        }
        SelectedCount = 0;
        ListHeader.Refresh();
        PinnedHeader.Refresh();
    }

    // The grid component unmounts when the view changes, which drops its select state.
    partial void OnCurrentViewChanged(AppView value)
    {
        if (value != AppView.Library) IsSelectMode = false;
    }

    [RelayCommand] private void StartSelect() => IsSelectMode = true;
    [RelayCommand] private void CancelSelect() => IsSelectMode = false;

    [RelayCommand]
    private void SelectAll()
    {
        foreach (var a in _allAlbums) a.IsSelected = true;
        SelectedCount = _allAlbums.Count;
    }

    public void ToggleAlbumSelection(AlbumItem a)
    {
        a.IsSelected = !a.IsSelected;
        SelectedCount += a.IsSelected ? 1 : -1;
    }

    [RelayCommand]
    private void DeleteSelected()
    {
        var targets = _allAlbums.Where(a => a.IsSelected).ToList();
        if (targets.Count == 0) return;
        ConfirmDialog = new ConfirmDialogVm("WARNING: BATCH_DELETE", $"{targets.Count} ALBUM(S)", "DATA WILL BE PERMANENTLY ERASED.", "PURGE",
            async () =>
            {
                IsSelectMode = false;
                await DeleteAlbumsAsync(targets);
            })
        {
            TargetLabel = "TARGETS: ",
            TargetBrush = OrangeBrush,
        };
    }

    /// <summary>DELETE_ALBUM from the album card's right-click menu.</summary>
    private void ConfirmDeleteAlbumFromMenu(AlbumItem a)
    {
        ConfirmDialog = new ConfirmDialogVm("WARNING: BATCH_DELETE", "1 ALBUM", "DATA WILL BE PERMANENTLY ERASED.", "PURGE",
            async () => await DeleteAlbumsAsync([a]))
        {
            TargetLabel = "TARGETS: ",
            TargetBrush = OrangeBrush,
            Detail = a.Name,
        };
    }

    /// <summary>DELETE_ALBUM from the album page's ⋯ menu.</summary>
    [RelayCommand]
    private void DeleteCurrentAlbum()
    {
        if (CurrentHeader is not { Album: { } a }) return;
        ConfirmDialog = new ConfirmDialogVm("WARNING: PURGE_DATA", a.Name, "DATA WILL BE PERMANENTLY ERASED.", "PURGE",
            async () => await DeleteAlbumsAsync([a]));
    }

    /// <summary>DELETING_DATA... overlay over the content while library.json is rewritten.</summary>
    [ObservableProperty] private bool _isDeleting;

    private async Task DeleteAlbumsAsync(IReadOnlyList<AlbumItem> items)
    {
        if (IsScanning)
        {
            // The scan holds library.json until it has merged its results.
            ShowToast("SCAN_IN_PROGRESS // DELETE AGAIN WHEN IT FINISHES", ToastKind.Warning);
            return;
        }
        var names = items.Select(a => a.Name).ToHashSet(StringComparer.Ordinal);
        IsDeleting = true;
        try
        {
            await Task.Run(() => LibraryStore.DeleteAlbumsAsync(names));
        }
        catch (Exception ex)
        {
            ShowToast($"PURGE_FAILED // {ex.Message}", ToastKind.Error);
            return;
        }
        finally
        {
            IsDeleting = false;
        }

        var ids = items.SelectMany(a => a.Album.Tracks).Select(t => t.Id).ToHashSet(StringComparer.OrdinalIgnoreCase);
        await Player.RemoveTracksAsync(t => ids.Contains(t.Id));

        if (CurrentHeader is { Album: { } open } && names.Contains(open.Name)) CurrentView = AppView.Library;

        // Drop them from the in-memory catalog instead of re-reading library.json.
        _allAlbums.RemoveAll(a => names.Contains(a.Name));
        foreach (var name in names) _albumsByName.Remove(name);
        foreach (var id in ids) _tracksById.Remove(id);
        foreach (var a in SidebarAlbums.Where(a => names.Contains(a.Name)).ToList()) SidebarAlbums.Remove(a);
        OnPropertyChanged(nameof(IsLibraryEmpty));
        OnPropertyChanged(nameof(SelectionText));
        RebuildLibrary();
        if (HasSearchQuery) RebuildSearch();
        AfterCollectionsChanged(); // playlists / favorites no longer resolve the deleted tracks

        ShowToast(names.Count == 1 ? $"DATA_PURGED // {names.First()}" : $"DATA_PURGED // {names.Count} ALBUMS", ToastKind.Info);
    }
}
