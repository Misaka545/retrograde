using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Retrograde.Core;

namespace Retrograde.App.ViewModels;

/// <summary>
/// Artists: the library grid can list artists instead of albums (an artist is an album artist, so
/// "Various Artists" is one too), and an artist page shows the artist's most played tracks and albums.
/// </summary>
public partial class MainViewModel
{
    private List<ArtistItem> _artists = [];
    private Dictionary<string, ArtistItem> _artistsByName = new(StringComparer.CurrentCultureIgnoreCase);

    [ObservableProperty] [NotifyPropertyChangedFor(nameof(IsAlbumsView))]
    private bool _isArtistsView;

    public bool IsAlbumsView => !IsArtistsView;

    partial void OnIsArtistsViewChanged(bool value)
    {
        _settings.LibraryView = value ? "artists" : "albums";
        _settings.Save();
        if (value) IsSelectMode = false; // SELECT picks albums
        RebuildLibrary();
    }

    [RelayCommand] private void ShowLibraryAlbums() => IsArtistsView = false;
    [RelayCommand] private void ShowLibraryArtists() => IsArtistsView = true;

    /// <summary>Groups the albums by album artist (called after every library load).</summary>
    private void BuildArtists()
    {
        _artistsByName = new Dictionary<string, ArtistItem>(StringComparer.CurrentCultureIgnoreCase);
        foreach (var a in _allAlbums)
        {
            var name = a.Artist.Trim().Length > 0 ? a.Artist.Trim() : "Unknown Artist";
            if (!_artistsByName.TryGetValue(name, out var artist))
                _artistsByName[name] = artist = new ArtistItem(name, this);
            artist.Albums.Add(a);
        }
        foreach (var artist in _artistsByName.Values) artist.Seal();
        _artists = _artistsByName.Values.ToList();
    }

    /// <summary>The artists in the library sort: names alphabetically, dates by the artist's newest album.</summary>
    private IEnumerable<ArtistItem> SortedArtists()
    {
        var cmp = StringComparer.CurrentCulture;
        return SelectedSort.Key switch
        {
            "name_asc" or "artist_asc" => _artists.OrderBy(a => a.Name, cmp),
            "name_desc" or "artist_desc" => _artists.OrderByDescending(a => a.Name, cmp),
            "added_asc" => _artists.OrderBy(a => a.Albums.Min(x => x.Album.AddedAt)),
            "year_desc" => _artists.OrderByDescending(a => a.Albums.Max(x => x.Album.Year)),
            "year_asc" => _artists.OrderBy(a => a.Albums.Where(x => x.Album.Year > 0).Select(x => x.Album.Year).DefaultIfEmpty(int.MaxValue).Min()),
            "played_desc" => _artists.OrderByDescending(a => a.Albums.Max(LastPlayed)).ThenBy(a => a.Name, cmp),
            "plays_desc" => _artists.OrderByDescending(a => a.Albums.Sum(PlayCount)).ThenBy(a => a.Name, cmp),
            _ => _artists.OrderByDescending(a => a.Albums.Max(x => x.Album.AddedAt)),
        };
    }

    public ArtistItem? FindArtist(string? name) =>
        name is not null && _artistsByName.TryGetValue(name.Trim(), out var a) ? a : null;

    /// <summary>The artist page: header, most played tracks (when any were played), then the albums, newest first.</summary>
    public void OpenArtist(ArtistItem artist) => OpenArtist(artist, keepScroll: false);

    private void OpenArtist(ArtistItem artist, bool keepScroll)
    {
        EnterDetail(() => OpenArtist(artist, false), keepScroll);
        if (CurrentView is not (AppView.Detail or AppView.Favorites)) _viewBeforeDetail = CurrentView;
        var albums = artist.Albums.OrderByDescending(a => a.Album.Year).ThenByDescending(a => a.Album.AddedAt).ToList();
        var newest = albums[0].Album;
        var tracks = artist.Tracks;
        var albumsText = albums.Count == 1 ? "1 ALBUM" : $"{albums.Count} ALBUMS";
        var rows = new List<object>
        {
            new DetailHeaderRow(this, DetailKind.Artist, artist.Name, albumsText, newest.CoverFullPath ?? newest.CoverThumbPath, tracks, 0) { Artist = artist },
        };

        var popular = tracks.Select(t => (Track: t, Plays: _stats.Get(t.Id)?.Count ?? 0))
            .Where(e => e.Plays > 0)
            .OrderByDescending(e => e.Plays).ThenByDescending(e => _stats.Get(e.Track.Id)?.Last ?? 0)
            .Take(5).Select(e => e.Track).ToList();
        var current = Player.CurrentTrack?.Id;
        if (popular.Count > 0)
        {
            rows.Add(new SectionHeaderRow("MOST PLAYED", true));
            for (int i = 0; i < popular.Count; i++)
            {
                var index = i;
                rows.Add(new TrackRow(popular[i], i + 1, _ => Player.PlayQueue(popular, index))
                {
                    IsCurrent = current is not null && popular[i].Id == current,
                    ShowCover = true,
                });
            }
        }
        rows.Add(new SectionHeaderRow($"ALBUMS ({albums.Count})", popular.Count == 0));
        AddCardRows(rows, albums);
        DetailRows.ReplaceAll(rows);
        CurrentView = AppView.Detail;
        if (!keepScroll) DetailOpened?.Invoke();
    }

    [RelayCommand]
    private void OpenArtistNamed(string? name)
    {
        if (FindArtist(name) is { } a) OpenArtist(a);
    }

    /// <summary>Player bar: the artist of the playing track's album, else the track's own artist.</summary>
    [RelayCommand]
    private void OpenCurrentArtist()
    {
        if (Player.CurrentTrack is not { } t) return;
        var artist = _tracksById.TryGetValue(t.Id, out var e) ? FindArtist(e.Album.Artist) : null;
        artist ??= FindArtist(t.Artist);
        if (artist is not null) OpenArtist(artist);
        else ShowToast($"ARTIST_NOT_IN_LIBRARY // {t.Artist}", ToastKind.Info);
    }

    public void PlayArtist(ArtistItem artist) => Player.PlayQueue(artist.Tracks, 0);
}

/// <summary>An album artist: a card in the ARTISTS grid and the subject of an artist page.</summary>
public sealed class ArtistItem(string name, MainViewModel owner) : CardItem
{
    public string Name { get; } = name;
    public List<AlbumItem> Albums { get; } = [];
    /// <summary>Every track, albums oldest first (the order PLAY uses).</summary>
    public IReadOnlyList<Track> Tracks { get; private set; } = [];
    private string? _cover;
    private string _subtitle = "";

    /// <summary>Called once the albums are in: works out the track list, cover and subtitle.</summary>
    public void Seal()
    {
        var ordered = Albums.OrderBy(a => a.Album.Year == 0 ? int.MaxValue : a.Album.Year).ThenBy(a => a.Album.AddedAt).ToList();
        Tracks = ordered.SelectMany(a => a.Album.Tracks).ToList();
        _cover = Albums.OrderByDescending(a => a.Album.Year).ThenByDescending(a => a.Album.AddedAt)
            .Select(a => a.Cover).FirstOrDefault(c => c is not null);
        _subtitle = Albums.Count == 1 ? $"1 ALBUM · {Tracks.Count} TRACKS" : $"{Albums.Count} ALBUMS · {Tracks.Count} TRACKS";
    }

    public override string Badge => "ART";
    public override string Title => Name;
    public override string Subtitle => _subtitle;
    public override string? Cover => _cover;
    protected override void OnOpen() => owner.OpenArtist(this);
    protected override void OnPlay() => owner.PlayArtist(this);
}
