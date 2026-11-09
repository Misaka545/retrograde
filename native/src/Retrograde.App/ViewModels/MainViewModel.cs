using System.Collections.ObjectModel;
using Avalonia.Threading;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Retrograde.Audio;
using Retrograde.Core;

namespace Retrograde.App.ViewModels;

public enum AppView { Library, Search, Detail, Favorites }

public partial class MainViewModel : ObservableObject
{
    private readonly AppSettings _settings = AppSettings.Load();
    private readonly MpvEngine _engine;
    private readonly DispatcherTimer _searchDebounce;
    private List<AlbumItem> _allAlbums = [];
    private Dictionary<string, AlbumItem> _albumsByName = new(StringComparer.Ordinal);
    private Dictionary<string, (Track Track, AlbumItem Album)> _tracksById = new(StringComparer.OrdinalIgnoreCase);
    private List<CardItem> _libraryCards = [];
    private int _columns = 4;
    private AppView _viewBeforeDetail = AppView.Library;

    public PlayerViewModel Player { get; }
    public ObservableCollection<object> LibraryRows { get; } = [];
    public ObservableCollection<object> SearchRows { get; } = [];
    public ObservableCollection<object> DetailRows { get; } = [];
    public ObservableCollection<AlbumItem> SidebarAlbums { get; } = [];
    public ObservableCollection<ToastItem> Toasts { get; } = [];

    public IReadOnlyList<SortOption> SortOptions { get; }

    [ObservableProperty] private SortOption _selectedSort;
    [ObservableProperty] [NotifyPropertyChangedFor(nameof(IsLibraryEmpty))]
    private bool _isLoading = true;
    [ObservableProperty] private bool _isManualOpen;

    [ObservableProperty]
    [NotifyPropertyChangedFor(nameof(IsLibraryView), nameof(IsSearchView), nameof(IsDetailView), nameof(IsFavoritesView))]
    private AppView _currentView = AppView.Library;

    [ObservableProperty] [NotifyPropertyChangedFor(nameof(IsPlaylistsMode))]
    private bool _isAlbumsMode = true;

    [ObservableProperty] [NotifyPropertyChangedFor(nameof(HasSearchQuery))]
    private string _searchQuery = "";

    [ObservableProperty] [NotifyPropertyChangedFor(nameof(IsTabAll), nameof(IsTabTracks), nameof(IsTabAlbums), nameof(IsTabPlaylists))]
    private string _searchTab = "all";

    public bool IsTabAll => SearchTab == "all";
    public bool IsTabTracks => SearchTab == "tracks";
    public bool IsTabAlbums => SearchTab == "albums";
    public bool IsTabPlaylists => SearchTab == "playlists";

    public bool IsLibraryView => CurrentView == AppView.Library;
    public bool IsSearchView => CurrentView == AppView.Search;
    public bool IsDetailView => CurrentView == AppView.Detail;
    public bool IsFavoritesView => CurrentView == AppView.Favorites;
    public bool IsPlaylistsMode => !IsAlbumsMode;
    public bool HasSearchQuery => SearchQuery.Trim().Length > 0;
    public bool IsLibraryEmpty => !IsLoading && _allAlbums.Count == 0;

    /// <summary>Raised when a new detail page opens, so the view can scroll to the top.</summary>
    public event Action? DetailOpened;

    public MainViewModel()
    {
        _engine = new MpvEngine(Path.Combine(AppContext.BaseDirectory, "bin", "mpv.exe"));
        Player = new PlayerViewModel(_engine, _settings);
        Player.TrackChanged += OnTrackChanged;
        Player.Toast += ShowToast;
        Action<SortOption> select = o => SelectedSort = o;
        SortOptions =
        [
            new("added_desc", "RECENTLY ADDED", select), new("added_asc", "OLDEST ADDED", select),
            new("name_asc", "NAME [A-Z]", select), new("name_desc", "NAME [Z-A]", select),
            new("artist_asc", "ARTIST [A-Z]", select), new("artist_desc", "ARTIST [Z-A]", select),
            new("year_desc", "NEWEST", select), new("year_asc", "OLDEST", select),
        ];
        _selectedSort = SortOptions.FirstOrDefault(o => o.Key == _settings.LibrarySort) ?? SortOptions[0];
        _selectedSort.IsSelected = true;
        _searchDebounce = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(150) };
        _searchDebounce.Tick += (_, _) => { _searchDebounce.Stop(); RebuildSearch(); };
        InitWindowSettings();
    }

    public async Task InitializeAsync()
    {
        await LoadLibraryAsync();
        LoadCollections();
        IsLoading = false;
    }

    /// <summary>(Re)reads library.json, e.g. after a scan.</summary>
    private async Task LoadLibraryAsync()
    {
        var albums = await Task.Run(() => LibraryStore.LoadAsync());
        _allAlbums = albums.Select(a => new AlbumItem(a, this)).ToList();
        _albumsByName = new Dictionary<string, AlbumItem>(StringComparer.Ordinal);
        _tracksById = new Dictionary<string, (Track, AlbumItem)>(StringComparer.OrdinalIgnoreCase);
        foreach (var a in _allAlbums)
        {
            _albumsByName.TryAdd(a.Name, a);
            foreach (var t in a.Album.Tracks) _tracksById.TryAdd(t.Id, (t, a));
        }
        SidebarAlbums.Clear();
        foreach (var a in _allAlbums.OrderBy(a => a.Name, StringComparer.CurrentCulture))
            SidebarAlbums.Add(a);
        OnPropertyChanged(nameof(IsLibraryEmpty));
        RebuildLibrary();
    }

    public IReadOnlyList<Track> ResolveTracks(IEnumerable<string> ids)
    {
        var list = new List<Track>();
        foreach (var id in ids)
            if (_tracksById.TryGetValue(id, out var e)) list.Add(e.Track);
        return list;
    }

    // ---------- Grid layout ----------

    /// <summary>Same breakpoints as the Electron VirtualGrid (window width in DIPs).</summary>
    public void SetWindowWidth(double width)
    {
        var columns = width >= 1280 ? 5 : width >= 1024 ? 4 : width >= 768 ? 3 : 2;
        if (columns == _columns) return;
        _columns = columns;
        RebuildLibraryRows();
        if (HasSearchQuery) RebuildSearch();
    }

    private void AddCardRows(ObservableCollection<object> target, IReadOnlyList<CardItem> cards)
    {
        for (int i = 0; i < cards.Count; i += _columns)
        {
            var slice = new CardItem[Math.Min(_columns, cards.Count - i)];
            for (int j = 0; j < slice.Length; j++) slice[j] = cards[i + j];
            target.Add(new CardRow(slice, _columns));
        }
    }

    // ---------- Library ----------

    partial void OnSelectedSortChanged(SortOption value)
    {
        foreach (var o in SortOptions) o.IsSelected = ReferenceEquals(o, value);
        _settings.LibrarySort = value.Key;
        RebuildLibrary();
    }

    private void RebuildLibrary()
    {
        var cmp = StringComparer.CurrentCulture;
        IEnumerable<AlbumItem> q = SelectedSort.Key switch
        {
            "added_asc" => _allAlbums.OrderBy(a => a.Album.AddedAt),
            "name_asc" => _allAlbums.OrderBy(a => a.Name, cmp),
            "name_desc" => _allAlbums.OrderByDescending(a => a.Name, cmp),
            "artist_asc" => _allAlbums.OrderBy(a => a.Artist, cmp),
            "artist_desc" => _allAlbums.OrderByDescending(a => a.Artist, cmp),
            "year_desc" => _allAlbums.OrderByDescending(a => a.Album.Year),
            "year_asc" => _allAlbums.OrderBy(a => a.Album.Year),
            _ => _allAlbums.OrderByDescending(a => a.Album.AddedAt),
        };
        _libraryCards = q.Cast<CardItem>().ToList();
        RebuildLibraryRows();
    }

    private void RebuildLibraryRows()
    {
        LibraryRows.Clear();
        LibraryRows.Add(new LibraryHeaderRow(this));
        AddCardRows(LibraryRows, _libraryCards);
    }

    // ---------- Navigation ----------

    [RelayCommand]
    private void GoHome()
    {
        SearchQuery = "";
        CurrentView = AppView.Library;
    }

    [RelayCommand] private void GoSearch() => CurrentView = AppView.Search;
    [RelayCommand] private void ShowAlbums() => IsAlbumsMode = true;
    [RelayCommand] private void ShowPlaylists() => IsAlbumsMode = false;
    [RelayCommand] private void OpenManual() => IsManualOpen = true;
    [RelayCommand] private void CloseManual() => IsManualOpen = false;

    [RelayCommand]
    private void Back()
    {
        if (CurrentView is not (AppView.Detail or AppView.Favorites)) return;
        CurrentView = _viewBeforeDetail is AppView.Detail or AppView.Favorites ? AppView.Library : _viewBeforeDetail;
    }

    /// <summary>Escape: close dialogs first, then leave the detail page.</summary>
    public void Escape()
    {
        if (IsBackgroundPromptOpen) IsBackgroundPromptOpen = false;
        else if (IsQueueOpen) IsQueueOpen = false;
        else if (IsFullScreen) IsFullScreen = false;
        else if (IsCreatePlaylistOpen) IsCreatePlaylistOpen = false;
        else if (ConfirmDialog is not null) ConfirmDialog = null;
        else if (IsManualOpen) IsManualOpen = false;
        else Back();
    }

    public void OpenAlbum(AlbumItem item)
    {
        var a = item.Album;
        ShowTrackList(new DetailHeaderRow(this, DetailKind.Album, a.Name, a.Artist, a.CoverFullPath ?? a.CoverThumbPath, a.Tracks, a.Year) { Album = item });
    }

    /// <summary>Builds the detail page: header, column titles, then tracks grouped by disc (AlbumDetail.jsx).</summary>
    private void ShowTrackList(DetailHeaderRow header, bool keepScroll = false)
    {
        if (CurrentView is not (AppView.Detail or AppView.Favorites)) _viewBeforeDetail = CurrentView;
        header.IsLiked = header.Tracks.Count > 0 && header.Tracks.All(t => IsLiked(t));
        DetailRows.Clear();
        DetailRows.Add(header);
        var showCover = header.Kind != DetailKind.Album;
        DetailRows.Add(new TrackColumnsRow(showCover));
        var tracks = header.Tracks;
        var discs = tracks.Select(t => t.DiscNumber <= 0 ? 1 : t.DiscNumber).Distinct().OrderBy(d => d).ToList();
        var multiDisc = discs.Count > 1;
        var current = Player.CurrentTrack?.Id;
        foreach (var disc in discs)
        {
            if (multiDisc) DetailRows.Add(new DiscHeaderRow(disc, disc == discs[0]));
            int n = 0;
            for (int i = 0; i < tracks.Count; i++)
            {
                var t = tracks[i];
                if ((t.DiscNumber <= 0 ? 1 : t.DiscNumber) != disc) continue;
                var index = i;
                DetailRows.Add(new TrackRow(t, multiDisc ? ++n : index + 1, _ => Player.PlayQueue(tracks, index))
                {
                    IsCurrent = current is not null && t.Id == current,
                    ShowCover = showCover,
                });
            }
        }
        CurrentView = AppView.Detail;
        if (!keepScroll) DetailOpened?.Invoke();
    }

    private DetailHeaderRow? CurrentHeader => CurrentView == AppView.Detail ? DetailRows.FirstOrDefault() as DetailHeaderRow : null;

    [RelayCommand]
    private void PlaySelectedAlbum()
    {
        if (CurrentHeader is { } h) Player.PlayQueue(h.Tracks, 0);
    }

    [RelayCommand]
    private void OpenCurrentAlbum()
    {
        var t = Player.CurrentTrack;
        if (t is null) return;
        if (_tracksById.TryGetValue(t.Id, out var e)) { OpenAlbum(e.Album); return; }
        var pl = Playlists.FirstOrDefault(p => p.Data.TrackIds.Contains(t.Id, StringComparer.OrdinalIgnoreCase));
        if (pl is not null) { OpenPlaylist(pl); return; }
        if (_albumsByName.TryGetValue(t.Album, out var item)) OpenAlbum(item);
    }

    public void PlayTrackInAlbum(Track track, AlbumItem? album)
    {
        if (album is not null)
        {
            var idx = -1;
            for (int i = 0; i < album.Album.Tracks.Count; i++)
                if (album.Album.Tracks[i].Id == track.Id) { idx = i; break; }
            if (idx >= 0) { Player.PlayQueue(album.Album.Tracks, idx); return; }
        }
        Player.PlayQueue([track], 0);
    }

    // ---------- Search ----------

    partial void OnSearchQueryChanged(string value)
    {
        _searchDebounce.Stop();
        _searchDebounce.Start();
    }

    partial void OnSearchTabChanged(string value) => RebuildSearch();

    [RelayCommand] private void SetSearchTab(string tab) => SearchTab = tab;

    private void RebuildSearch()
    {
        SearchRows.Clear();
        var q = SearchQuery.Trim();
        if (q.Length == 0) return;
        const StringComparison cmp = StringComparison.CurrentCultureIgnoreCase;

        var current = Player.CurrentTrack?.Id;
        var tracks = new List<CardItem>();
        var albums = new List<CardItem>();
        foreach (var a in _allAlbums)
        {
            if (a.Name.Contains(q, cmp) || a.Artist.Contains(q, cmp)) albums.Add(a);
            foreach (var t in a.Album.Tracks)
                if (t.Title.Contains(q, cmp) || t.Artist.Contains(q, cmp))
                    tracks.Add(new TrackCard(t, a, this) { IsCurrent = current is not null && t.Id == current });
        }
        var playlists = Playlists.Where(p => p.Name.Contains(q, cmp)).Select(p => (CardItem)new PlaylistCard(p, this)).ToList();

        bool first = true;
        void Section(string label, List<CardItem> items, bool show)
        {
            if (!show || items.Count == 0) return;
            SearchRows.Add(new SectionHeaderRow($"{label} ({items.Count})", first));
            AddCardRows(SearchRows, items);
            first = false;
        }
        Section("TRACKS", tracks, SearchTab is "all" or "tracks");
        Section("ALBUMS", albums, SearchTab is "all" or "albums");
        Section("PLAYLISTS", playlists, SearchTab is "all" or "playlists");
        if (tracks.Count == 0 && albums.Count == 0 && playlists.Count == 0)
            SearchRows.Add(new EmptyRow("NO RESULTS FOUND FOR QUERY"));
    }

    // ---------- Misc ----------

    private void OnTrackChanged(Track? track)
    {
        var id = track?.Id;
        foreach (var r in DetailRows)
            if (r is TrackRow tr) tr.IsCurrent = id is not null && tr.Track.Id == id;
        foreach (var r in SearchRows)
            if (r is CardRow row)
                foreach (var c in row.Items)
                    if (c is TrackCard tc) tc.IsCurrent = id is not null && tc.Track.Id == id;
        OnPropertyChanged(nameof(IsCurrentLiked));
    }

    [RelayCommand] private void NotPorted(string feature) => ShowToast($"NOT_PORTED_YET // {feature}", ToastKind.Warning);

    public void ShowToast(string message, ToastKind kind)
    {
        var toast = new ToastItem(message, kind);
        Toasts.Add(toast);
        while (Toasts.Count > 5) Toasts.RemoveAt(0);
        DispatcherTimer.RunOnce(() => Toasts.Remove(toast), TimeSpan.FromSeconds(3));
    }

    public void Shutdown()
    {
        _settings.Save();
        _engine.DisposeAsync().AsTask().Wait(1500);
    }
}
