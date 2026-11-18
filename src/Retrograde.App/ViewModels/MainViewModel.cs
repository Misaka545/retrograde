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
    private readonly PlayStats _stats = PlayStats.Load();
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
    public BulkObservableCollection<object> DetailRows { get; } = [];
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

    [ObservableProperty] [NotifyPropertyChangedFor(nameof(IsTabAll), nameof(IsTabTracks), nameof(IsTabAlbums), nameof(IsTabArtists), nameof(IsTabPlaylists))]
    private string _searchTab = "all";

    public bool IsTabAll => SearchTab == "all";
    public bool IsTabTracks => SearchTab == "tracks";
    public bool IsTabAlbums => SearchTab == "albums";
    public bool IsTabArtists => SearchTab == "artists";
    public bool IsTabPlaylists => SearchTab == "playlists";

    public bool IsLibraryView => CurrentView == AppView.Library;
    public bool IsSearchView => CurrentView == AppView.Search;
    public bool IsDetailView => CurrentView == AppView.Detail;
    public bool IsFavoritesView => CurrentView == AppView.Favorites;
    public bool IsPlaylistsMode => !IsAlbumsMode;
    public bool HasSearchQuery => SearchQuery.Trim().Length > 0;
    public bool IsLibraryEmpty => !IsLoading && _allAlbums.Count == 0;

    /// <summary>Title bar text, from the version the build was published with (-p:Version).</summary>
    public string VersionText { get; } = "RETROGRADE // VER " + AppVersion();

    private static string AppVersion()
    {
        var v = typeof(MainViewModel).Assembly.GetName().Version;
        return v is null ? "2.0" : $"{v.Major}.{v.Minor}";
    }

    /// <summary>Raised when a new detail page opens, so the view can scroll to the top.</summary>
    public event Action? DetailOpened;

    public MainViewModel()
    {
        // libmpv runs in this process; a second engine is made for crossfades
        Player = new PlayerViewModel(() => new MpvEngine(), _settings);
        Player.TrackChanged += OnTrackChanged;
        Player.TrackPlayed += OnTrackPlayed;
        // Shuffle plays the whole library; turning it off after a restart goes back to the track's album
        Player.LibraryTracks = () => _allAlbums.SelectMany(a => a.Album.Tracks).ToList();
        Player.AlbumOf = t => _tracksById.TryGetValue(t.Id, out var e) ? e.Album.Album.Tracks : null;
        Player.Toast += ShowToast;
        Action<SortOption> select = o => SelectedSort = o;
        SortOptions =
        [
            new("added_desc", "RECENTLY ADDED", select), new("added_asc", "OLDEST ADDED", select),
            new("name_asc", "NAME [A-Z]", select), new("name_desc", "NAME [Z-A]", select),
            new("artist_asc", "ARTIST [A-Z]", select), new("artist_desc", "ARTIST [Z-A]", select),
            new("year_desc", "NEWEST", select), new("year_asc", "OLDEST", select),
            new("played_desc", "RECENTLY PLAYED", select), new("plays_desc", "MOST PLAYED", select),
        ];
        _selectedSort = SortOptions.FirstOrDefault(o => o.Key == _settings.LibrarySort) ?? SortOptions[0];
        _selectedSort.IsSelected = true;
        _searchDebounce = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(150) };
        _searchDebounce.Tick += (_, _) => { _searchDebounce.Stop(); RebuildSearch(); };
        InitWindowSettings();
#pragma warning disable MVVMTK0034 // backing field: no save while constructing
        _isSidebarVisible = _settings.SidebarVisible;
        _isArtistsView = _settings.LibraryView == "artists";
#pragma warning restore MVVMTK0034
    }
    public async Task InitializeAsync()
    {
        try
        {
            await LoadLibraryAsync();
        }
        catch (Exception ex)
        {
            // An unreadable library.json must still end the startup load (the empty state shows instead).
            ErrorLog.Write("load library", ex);
            ShowToast(ex is InvalidDataException
                ? "LIBRARY_DAMAGED // NO USABLE BACKUP, SEE native-error.log"
                : $"LIBRARY_LOAD_FAILED // {ex.Message}", ToastKind.Error);
        }
        LoadCollections();
        try { RestoreSession(); }
        catch (Exception ex) { ErrorLog.Write("restore session", ex); }
        IsLoading = false;
        StartWatching();
        if (LibraryStore.RestoredFromBackup) ShowToast("LIBRARY_RESTORED // FROM BACKUP, DAMAGED FILE KEPT", ToastKind.Warning);
        if (CollectionsStore.RestoredFromBackup) ShowToast("PLAYLISTS_RESTORED // FROM BACKUP, DAMAGED FILE KEPT", ToastKind.Warning);
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
        BuildArtists();
        RebuildLibrary();
        RefreshSmartPlaylists();
    }

    /// <summary>The first of these ids that is in the library (a playlist's cover when it has none of its own).</summary>
    public Track? FirstTrack(IEnumerable<string> ids)
    {
        foreach (var id in ids)
            if (_tracksById.TryGetValue(id, out var e)) return e.Track;
        return null;
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
        _windowWidth = width;
        UpdateColumns();
    }

    private double _windowWidth = 1200;

    /// <summary>
    /// The Electron breakpoints (1280 / 1024 / 768 window px) restated as content width: window minus
    /// the 12px margins and the 256px sidebar column. With the sidebar hidden the same content width rules
    /// apply, and wide windows get a 6th and 7th column so covers keep about the same size.
    /// </summary>
    private void UpdateColumns()
    {
        var content = _windowWidth - 24 - SidebarWidth;
        var columns = content >= 1000 ? 5 : content >= 744 ? 4 : content >= 488 ? 3 : 2;
        if (!IsSidebarVisible) columns = content >= 1512 ? 7 : content >= 1256 ? 6 : columns;
        if (columns == _columns) return;
        _columns = columns;
        RebuildLibraryRows();
        if (HasSearchQuery) RebuildSearch();
        if (CurrentHeader is { Kind: DetailKind.Artist, Artist: { } artist }) OpenArtist(artist, keepScroll: true);
    }

    // ---------- Sidebar ----------

    /// <summary>The sidebar column, sidebar plus its 12px gap: 244px expanded, 60px collapsed to icons.</summary>
    private double SidebarWidth => IsSidebarVisible ? 256 : 72;

    [ObservableProperty] [NotifyPropertyChangedFor(nameof(SidebarToggleTip))]
    private bool _isSidebarVisible = true;

    /// <summary>True: full sidebar. False: collapsed to a column of icons and covers (Spotify style).</summary>
    public string SidebarToggleTip => IsSidebarVisible ? "Collapse sidebar (Ctrl+B)" : "Expand sidebar (Ctrl+B)";

    [RelayCommand] private void ToggleSidebar() => IsSidebarVisible = !IsSidebarVisible;

    partial void OnIsSidebarVisibleChanged(bool value)
    {
        _settings.SidebarVisible = value;
        _settings.Save();
        UpdateColumns();
    }

    private void AddCardRows(IList<object> target, IReadOnlyList<CardItem> cards)
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
            // Albums never played keep their added order at the end
            "played_desc" => _allAlbums.OrderByDescending(LastPlayed).ThenByDescending(a => a.Album.AddedAt),
            "plays_desc" => _allAlbums.OrderByDescending(PlayCount).ThenByDescending(LastPlayed).ThenByDescending(a => a.Album.AddedAt),
            _ => _allAlbums.OrderByDescending(a => a.Album.AddedAt),
        };
        _libraryCards = IsArtistsView ? SortedArtists().Cast<CardItem>().ToList() : q.Cast<CardItem>().ToList();
        RebuildLibraryRows();
    }

    private void RebuildLibraryRows()
    {
        LibraryRows.Clear();
        LibraryRows.Add(ListHeader);
        AddCardRows(LibraryRows, _libraryCards);
    }

    // ---------- Play counts ----------

    private long LastPlayed(AlbumItem a)
    {
        long last = 0;
        foreach (var t in a.Album.Tracks)
            if (_stats.Get(t.Id) is { } s && s.Last > last) last = s.Last;
        return last;
    }

    private int PlayCount(AlbumItem a)
    {
        var n = 0;
        foreach (var t in a.Album.Tracks) n += _stats.Get(t.Id)?.Count ?? 0;
        return n;
    }

    /// <summary>The current track counts as played: recorded at once, the file is small.</summary>
    private void OnTrackPlayed(Track track)
    {
        _stats.Record(track.Id, DateTimeOffset.Now);
        _ = Task.Run(_stats.Save);
        RefreshSmartPlaylists();
        // The grid is not reordered under the user's eyes; the new order shows on the next sort or start
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
        // Pages opened from a page (album -> artist -> album) go back one page at a time
        if (CurrentView == AppView.Detail && _backStack.Count > 0)
        {
            var reopen = _backStack.Pop();
            _navigatingBack = true;
            try { reopen(); }
            finally { _navigatingBack = false; }
            OnPropertyChanged(nameof(BackText));
            return;
        }
        CurrentView = _viewBeforeDetail is AppView.Detail or AppView.Favorites ? AppView.Library : _viewBeforeDetail;
    }

    // ---------- Page history ----------

    private readonly Stack<Action> _backStack = new();
    private Action? _reopenCurrent;
    private bool _navigatingBack;

    /// <summary>The back button: one page back, or out to where the pages were opened from.</summary>
    public string BackText => _backStack.Count > 0 ? "BACK" : _viewBeforeDetail switch
    {
        AppView.Search => "SEARCH",
        _ => "LIBRARY",
    };

    /// <summary>
    /// A detail page is about to show. Opening it from another detail page remembers that one for BACK;
    /// a refresh of the page on screen (keepScroll) does not.
    /// </summary>
    private void EnterDetail(Action reopen, bool keepScroll)
    {
        if (!keepScroll && !_navigatingBack && CurrentView == AppView.Detail && _reopenCurrent is not null)
        {
            _backStack.Push(_reopenCurrent);
            if (_backStack.Count > 30) // keep it short: drop the oldest
            {
                var keep = _backStack.Take(30).Reverse().ToList();
                _backStack.Clear();
                foreach (var a in keep) _backStack.Push(a);
            }
        }
        _reopenCurrent = reopen;
        OnPropertyChanged(nameof(BackText));
    }

    /// <summary>Escape: close dialogs first, then leave the detail page.</summary>
    public void Escape()
    {
        if (IsBackgroundPromptOpen) IsBackgroundPromptOpen = false;
        else if (TagEditor is { IsSaving: false }) TagEditor = null;
        else if (SmartEditor is not null) SmartEditor = null;
        else if (IsQueueOpen) IsQueueOpen = false;
        else if (IsFullScreen) IsFullScreen = false;
        else if (IsCreatePlaylistOpen) IsCreatePlaylistOpen = false;
        else if (ConfirmDialog is not null) ConfirmDialog = null;
        else if (IsEggOpen) IsEggOpen = false;
        else if (IsManualOpen) IsManualOpen = false;
        else if (IsSelectMode) IsSelectMode = false;
        else Back();
    }

    public void OpenAlbum(AlbumItem item) => OpenAlbum(item, false);

    private void OpenAlbum(AlbumItem item, bool keepScroll)
    {
        var a = item.Album;
        ShowTrackList(new DetailHeaderRow(this, DetailKind.Album, a.Name, a.Artist, a.CoverFullPath ?? a.CoverThumbPath, a.Tracks, a.Year) { Album = item },
            () => OpenAlbum(item, false), keepScroll);
    }

    /// <summary>Builds the detail page: header, column titles, then tracks grouped by disc (AlbumDetail.jsx).</summary>
    private void ShowTrackList(DetailHeaderRow header, Action reopen, bool keepScroll = false)
    {
        EnterDetail(reopen, keepScroll);
        if (CurrentView is not (AppView.Detail or AppView.Favorites)) _viewBeforeDetail = CurrentView;
        header.IsLiked = header.Tracks.Count > 0 && header.Tracks.All(t => IsLiked(t));
        var showCover = header.Kind != DetailKind.Album;
        var tracks = header.Tracks;
        // Only an album is split by disc: a playlist mixes albums, and their disc numbers mean nothing together
        var discs = header.Kind == DetailKind.Album
            ? tracks.Select(t => t.DiscNumber <= 0 ? 1 : t.DiscNumber).Distinct().OrderBy(d => d).ToList()
            : [0];
        var multiDisc = discs.Count > 1;
        var current = Player.CurrentTrack?.Id;
        var rows = new List<object>(tracks.Count + discs.Count + 2) { header, new TrackColumnsRow(showCover) };
        foreach (var disc in discs)
        {
            if (multiDisc) rows.Add(new DiscHeaderRow(disc, disc == discs[0]));
            int n = 0;
            for (int i = 0; i < tracks.Count; i++)
            {
                var t = tracks[i];
                if (disc != 0 && (t.DiscNumber <= 0 ? 1 : t.DiscNumber) != disc) continue;
                var index = i;
                rows.Add(new TrackRow(t, multiDisc ? ++n : index + 1, _ => Player.PlayQueue(tracks, index))
                {
                    IsCurrent = current is not null && t.Id == current,
                    ShowCover = showCover,
                });
            }
        }
        DetailRows.ReplaceAll(rows);
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
        var artists = _artists.Where(a => a.Name.Contains(q, cmp)).OrderBy(a => a.Name, StringComparer.CurrentCulture).Cast<CardItem>().ToList();
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
        Section("ARTISTS", artists, SearchTab is "all" or "artists");
        Section("PLAYLISTS", playlists, SearchTab is "all" or "playlists");
        if (tracks.Count == 0 && albums.Count == 0 && artists.Count == 0 && playlists.Count == 0)
            SearchRows.Add(new EmptyRow("NO RESULTS FOUND FOR QUERY"));
    }

    // ---------- Misc ----------

    private void OnTrackChanged(Track? track)
    {
        // Decode the full-screen artwork ahead of time, so opening full screen shows it at once
        // (and its reveal picture is not taken with the placeholder disc).
        if (track?.CoverFullPath is { } full) _ = Services.CoverLoader.LoadAsync(full, 1100);
        // Same for the first rows of Next in Queue (64 px, usually one cover for a whole album).
        foreach (var thumb in Player.UpcomingCovers(12)) _ = Services.CoverLoader.LoadAsync(thumb, 64);
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
        // Errors stay up longer: they are the ones worth reading.
        // Slide out (toast-exit, 0.4s) before leaving the list. Errors stay up longer: they are the ones worth reading.
        DispatcherTimer.RunOnce(() =>
        {
            toast.IsExiting = true;
            DispatcherTimer.RunOnce(() => Toasts.Remove(toast), TimeSpan.FromMilliseconds(400));
        }, TimeSpan.FromSeconds(kind == ToastKind.Error ? 6 : 3));
    }

    public void Shutdown()
    {
        _settings.Save();
        SaveSession(background: false);
        StopWatching();
        // Off the UI thread: the engine's awaits would otherwise queue behind this very wait and time out.
        Task.Run(Player.DisposeEnginesAsync).Wait(2000);
    }
}

/// <summary>An observable collection that can swap all its items with one Reset (a page of thousands of rows).</summary>
public sealed class BulkObservableCollection<T> : ObservableCollection<T>
{
    public void ReplaceAll(IEnumerable<T> items)
    {
        Items.Clear();
        foreach (var item in items) Items.Add(item);
        OnPropertyChanged(new System.ComponentModel.PropertyChangedEventArgs(nameof(Count)));
        OnPropertyChanged(new System.ComponentModel.PropertyChangedEventArgs("Item[]"));
        OnCollectionChanged(new System.Collections.Specialized.NotifyCollectionChangedEventArgs(System.Collections.Specialized.NotifyCollectionChangedAction.Reset));
    }
}
