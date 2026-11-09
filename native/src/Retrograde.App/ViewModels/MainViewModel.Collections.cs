using System.Collections.ObjectModel;
using Avalonia.Media;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Retrograde.Core;

namespace Retrograde.App.ViewModels;

/// <summary>Confirmation dialog (CustomModal with a warning title).</summary>
public sealed class ConfirmDialogVm(string title, string target, string note, string confirmText, Action confirm)
{
    public string Title { get; } = title;
    /// <summary>"TARGET: " for one named item, "TARGETS: " for a count (BATCH_DELETE).</summary>
    public string TargetLabel { get; init; } = "TARGET: ";
    public IBrush TargetBrush { get; init; } = Brushes.White;
    /// <summary>Optional name under a count, with an orange edge (single delete from the album menu).</summary>
    public string? Detail { get; init; }
    public bool HasDetail => Detail is not null;
    public string Target { get; } = target;
    public string Note { get; } = note;
    public string ConfirmText { get; } = confirmText;
    public Action Confirm { get; } = confirm;
}

/// <summary>Liked songs, playlists, queue additions and the right-click menus (PlayerContext.jsx equivalents).</summary>
public partial class MainViewModel
{
    private static readonly IBrush TealBrush = new SolidColorBrush(Color.Parse("#4FD6BE"));
    private static readonly IBrush OrangeBrush = new SolidColorBrush(Color.Parse("#FF6B35"));
    private static readonly IBrush MuteBrush = new SolidColorBrush(Color.Parse("#555555"));

    private CollectionsData _collections = new();
    private HashSet<string> _liked = new(StringComparer.OrdinalIgnoreCase);

    public ObservableCollection<PlaylistItem> Playlists { get; } = [];

    [ObservableProperty] [NotifyPropertyChangedFor(nameof(LikedUnitsText))]
    private int _likedCount;

    [ObservableProperty] private bool _isCreatePlaylistOpen;
    [ObservableProperty] private string _newPlaylistName = "";
    [ObservableProperty] private ConfirmDialogVm? _confirmDialog;

    public string LikedUnitsText => $"{LikedCount} UNITS";
    public bool IsCurrentLiked => Player.CurrentTrack is { } t && IsLiked(t);

    private void LoadCollections()
    {
        _collections = CollectionsStore.Load();
        _liked = new HashSet<string>(_collections.Liked, StringComparer.OrdinalIgnoreCase);
        LikedCount = _liked.Count;
        Playlists.Clear();
        foreach (var p in _collections.Playlists) Playlists.Add(new PlaylistItem(p, this));
        if (_collections.ImportedFrom is not null && (_liked.Count > 0 || Playlists.Count > 0))
        {
            ShowToast($"IMPORTED // {_liked.Count} FAVORITES, {Playlists.Count} PLAYLISTS", ToastKind.Success);
            _collections.ImportedFrom = null; // announce once
            CollectionsStore.Save(_collections);
        }
    }

    private void SaveCollections()
    {
        _collections.Liked = _liked.ToList();
        _collections.Playlists = Playlists.Select(p => p.Data).ToList();
        CollectionsStore.Save(_collections);
    }

    public bool IsLiked(Track t) => _liked.Contains(t.Id);

    // ---------- Likes ----------

    private void SetLiked(IEnumerable<Track> tracks, bool liked)
    {
        foreach (var t in tracks)
        {
            if (liked)
            {
                if (_liked.Add(t.Id)) _collections.Liked.Add(t.Id);
            }
            else if (_liked.Remove(t.Id)) _collections.Liked.RemoveAll(id => string.Equals(id, t.Id, StringComparison.OrdinalIgnoreCase));
        }
        // Keep insertion order of the stored list (it drives the FAVORITES page order).
        _liked = new HashSet<string>(_collections.Liked, StringComparer.OrdinalIgnoreCase);
        LikedCount = _liked.Count;
        CollectionsStore.Save(_collections);
        AfterCollectionsChanged();
    }

    public void ToggleLike(Track t)
    {
        var was = IsLiked(t);
        SetLiked([t], !was);
        ShowToast(was ? "TRACK_REMOVED // FAVORITES" : $"TRACK_ADDED // FAVORITES: {t.Title}", was ? ToastKind.Info : ToastKind.Success);
    }

    /// <summary>toggleLikeMultiple: unlike all when every track is liked, otherwise like all.</summary>
    public void ToggleLikeMany(IReadOnlyList<Track> tracks)
    {
        if (tracks.Count == 0) return;
        var all = tracks.All(IsLiked);
        SetLiked(tracks, !all);
        ShowToast(all ? $"TRACKS_REMOVED // FAVORITES ({tracks.Count})" : $"TRACKS_ADDED // FAVORITES ({tracks.Count})", all ? ToastKind.Info : ToastKind.Success);
    }

    [RelayCommand]
    private void ToggleLikeCurrent()
    {
        if (Player.CurrentTrack is { } t) ToggleLike(t);
    }

    [RelayCommand]
    private void ToggleLikeDetail()
    {
        if (CurrentHeader is { } h) ToggleLikeMany(h.Tracks);
    }

    [RelayCommand]
    private void GoFavorites()
    {
        SearchQuery = "";
        OpenLiked();
    }

    private void OpenLiked(bool keepScroll = false)
    {
        var tracks = ResolveTracks(_collections.Liked);
        if (tracks.Count == 0)
        {
            if (CurrentView is not (AppView.Detail or AppView.Favorites)) _viewBeforeDetail = CurrentView;
            CurrentView = AppView.Favorites;
            return;
        }
        ShowTrackList(new DetailHeaderRow(this, DetailKind.Liked, "Liked Songs", "User Data", null, tracks, 0), keepScroll);
    }

    /// <summary>Refreshes whatever shows likes / playlist contents after a change.</summary>
    private void AfterCollectionsChanged()
    {
        OnPropertyChanged(nameof(IsCurrentLiked));
        foreach (var p in Playlists) p.Refresh();
        switch (CurrentHeader)
        {
            case { Kind: DetailKind.Liked }: OpenLiked(keepScroll: true); break;
            case { Kind: DetailKind.Playlist, Playlist: { } pl }: OpenPlaylist(pl, keepScroll: true); break;
            case { } h: h.IsLiked = h.Tracks.Count > 0 && h.Tracks.All(IsLiked); break;
        }
    }

    // ---------- Playlists ----------

    public void OpenPlaylist(PlaylistItem p) => OpenPlaylist(p, false);

    private void OpenPlaylist(PlaylistItem p, bool keepScroll)
    {
        var tracks = ResolveTracks(p.Data.TrackIds);
        ShowTrackList(new DetailHeaderRow(this, DetailKind.Playlist, p.Name, null, p.CoverFull, tracks, 0) { Playlist = p }, keepScroll);
    }

    [RelayCommand]
    private void StartCreatePlaylist()
    {
        NewPlaylistName = "";
        IsCreatePlaylistOpen = true;
    }

    [RelayCommand] private void CancelCreatePlaylist() => IsCreatePlaylistOpen = false;

    [RelayCommand]
    private void ConfirmCreatePlaylist()
    {
        var name = NewPlaylistName.Trim();
        if (name.Length == 0) return;
        var data = new PlaylistData { Id = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(), Name = name };
        Playlists.Add(new PlaylistItem(data, this));
        SaveCollections();
        IsAlbumsMode = false;
        IsCreatePlaylistOpen = false;
        ShowToast($"PLAYLIST_CREATED // {name}", ToastKind.Success);
    }

    private void TogglePlaylistTrack(PlaylistItem p, Track t)
    {
        var ids = p.Data.TrackIds;
        var idx = ids.FindIndex(id => string.Equals(id, t.Id, StringComparison.OrdinalIgnoreCase));
        if (idx >= 0)
        {
            ids.RemoveAt(idx);
            ShowToast($"TRACK_REMOVED // PLAYLIST: {p.Name}", ToastKind.Info);
        }
        else
        {
            ids.Add(t.Id);
            ShowToast($"TRACK_ADDED // PLAYLIST: {p.Name}", ToastKind.Success);
        }
        SaveCollections();
        AfterCollectionsChanged();
    }

    /// <summary>Album menu: remove all when every track is already in, otherwise add the missing ones.</summary>
    private void TogglePlaylistTracks(PlaylistItem p, IReadOnlyList<Track> tracks)
    {
        var ids = p.Data.TrackIds;
        var set = new HashSet<string>(ids, StringComparer.OrdinalIgnoreCase);
        if (tracks.All(t => set.Contains(t.Id)))
        {
            ids.RemoveAll(id => tracks.Any(t => string.Equals(t.Id, id, StringComparison.OrdinalIgnoreCase)));
            ShowToast($"TRACKS_REMOVED // PLAYLIST: {p.Name}", ToastKind.Info);
        }
        else
        {
            foreach (var t in tracks) if (set.Add(t.Id)) ids.Add(t.Id);
            ShowToast($"TRACKS_ADDED // PLAYLIST: {p.Name}", ToastKind.Success);
        }
        SaveCollections();
        AfterCollectionsChanged();
    }

    [RelayCommand]
    private void DeleteCurrentPlaylist()
    {
        if (CurrentHeader is { Playlist: { } p }) ConfirmDeletePlaylist(p);
    }

    private void ConfirmDeletePlaylist(PlaylistItem p)
    {
        ConfirmDialog = new ConfirmDialogVm("WARNING: DELETE_PLAYLIST", p.Name, "ACTION CANNOT BE UNDONE.", "CONFIRM", () =>
        {
            var wasOpen = CurrentHeader?.Playlist == p;
            Playlists.Remove(p);
            DeletePlaylistCoverFiles(p.Data);
            SaveCollections();
            if (wasOpen) CurrentView = AppView.Library;
            if (HasSearchQuery) RebuildSearch();
            ShowToast($"PLAYLIST_DELETED // {p.Name}", ToastKind.Info);
        });
    }

    // ---------- Playlist cover (CHANGE_COVER / REMOVE_COVER) ----------

    /// <summary>Sets the open playlist's cover from an image the user picked.</summary>
    public async Task SetCurrentPlaylistCoverAsync(string imagePath)
    {
        if (CurrentHeader is not { Playlist: { } p }) return;
        try
        {
            var previous = p.Data.CoverArt;
            p.Data.CoverArt = await Task.Run(() => PlaylistCovers.Save(p.Data.Id, imagePath, previous));
        }
        catch (Exception ex)
        {
            ShowToast($"COVER_FAILED // {ex.Message}", ToastKind.Error);
            return;
        }
        AfterCoverChanged(p);
    }

    [RelayCommand]
    private void RemovePlaylistCover()
    {
        if (CurrentHeader is not { Playlist: { } p } || p.Data.CoverArt is null) return;
        PlaylistCovers.Delete(p.Data.CoverArt);
        p.Data.CoverArt = null;
        AfterCoverChanged(p);
    }

    private void AfterCoverChanged(PlaylistItem p)
    {
        SaveCollections();
        p.Refresh();
        if (CurrentHeader?.Playlist == p) OpenPlaylist(p, keepScroll: true);
        if (HasSearchQuery) RebuildSearch();
    }

    private static void DeletePlaylistCoverFiles(PlaylistData p) => PlaylistCovers.Delete(p.CoverArt);

    [RelayCommand] private void CancelConfirm() => ConfirmDialog = null;

    [RelayCommand]
    private void AcceptConfirm()
    {
        var d = ConfirmDialog;
        ConfirmDialog = null;
        d?.Confirm();
    }

    // ---------- Right-click menus ----------

    private IEnumerable<MenuEntry> PlaylistEntries(IReadOnlyList<Track> tracks, Action<PlaylistItem> toggle)
    {
        yield return new MenuEntry { IsSeparator = true };
        yield return new MenuEntry { IsCaption = true, Text = "ADD_TO_PLAYLIST" };
        if (Playlists.Count == 0)
        {
            yield return new MenuEntry { IsCaption = true, Text = "NO_DATA" };
            yield break;
        }
        foreach (var p in Playlists)
        {
            var set = new HashSet<string>(p.Data.TrackIds, StringComparer.OrdinalIgnoreCase);
            var all = tracks.Count > 0 && tracks.All(t => set.Contains(t.Id));
            var pl = p;
            yield return new MenuEntry { IsPlaylistRow = true, Text = p.Name, Checked = all, Execute = () => toggle(pl) };
        }
    }

    /// <summary>COMMAND_MENU for a track row (AlbumDetail.jsx).</summary>
    public MenuVm TrackMenu(Track t)
    {
        var liked = IsLiked(t);
        var items = new List<MenuEntry>
        {
            new() { Text = liked ? "REMOVE_FAVORITE" : "ADD_TO_FAVORITE", Icon = "IcHeart", IconBrush = liked ? OrangeBrush : MuteBrush, IconFilled = liked, Execute = () => ToggleLike(t) },
            new() { Text = "ADD_TO_QUEUE", Icon = "IcListMusic", IconBrush = MuteBrush, Execute = () => { Player.AddToQueue([t]); ShowToast($"TRACK_ADDED // QUEUE: {t.Title}", ToastKind.Success); } },
        };
        items.AddRange(PlaylistEntries([t], p => TogglePlaylistTrack(p, t)));
        return new MenuVm("COMMAND_MENU", TealBrush, items);
    }

    /// <summary>ALBUM_OPERATIONS for an album or playlist card (LibraryGrid.jsx).</summary>
    private MenuVm CollectionMenu(IReadOnlyList<Track> tracks, string deleteText, Action delete)
    {
        var allLiked = tracks.Count > 0 && tracks.All(IsLiked);
        var items = new List<MenuEntry>
        {
            new() { Text = "ADD_ALL_TO_QUEUE", Icon = "IcListMusic", IconBrush = MuteBrush, Execute = () => { Player.AddToQueue(tracks); ShowToast($"TRACKS_ADDED // QUEUE ({tracks.Count})", ToastKind.Success); } },
            new() { Text = allLiked ? "UNLIKE_ALL_TRACKS" : "LIKE_ALL_TRACKS", Icon = "IcHeart", IconBrush = allLiked ? OrangeBrush : MuteBrush, IconFilled = allLiked, Execute = () => ToggleLikeMany(tracks) },
            new() { Text = deleteText, Icon = "IcTrash", IconBrush = OrangeBrush, IsDanger = true, Execute = delete },
        };
        items.AddRange(PlaylistEntries(tracks, p => TogglePlaylistTracks(p, tracks)));
        return new MenuVm("ALBUM_OPERATIONS", OrangeBrush, items);
    }

    public MenuVm AlbumMenu(AlbumItem a) => CollectionMenu(a.Album.Tracks, "DELETE_ALBUM", () => ConfirmDeleteAlbumFromMenu(a));

    public MenuVm PlaylistMenu(PlaylistItem p) => CollectionMenu(ResolveTracks(p.Data.TrackIds), "DELETE_PLAYLIST", () => ConfirmDeletePlaylist(p));
}
