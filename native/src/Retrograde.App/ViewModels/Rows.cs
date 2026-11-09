using System.Globalization;
using Avalonia.Media;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Retrograde.Core;

namespace Retrograde.App.ViewModels;

public sealed partial class SortOption(string key, string label, Action<SortOption> select) : ObservableObject
{
    public string Key { get; } = key;
    public string Label { get; } = label;
    [ObservableProperty] private bool _isSelected;
    [RelayCommand] private void Select() => select(this);
    public override string ToString() => Label;
}

public static class Fmt
{
    /// <summary>Same as the Electron formatTime: m:ss, "0:00" for missing values.</summary>
    public static string Time(double seconds)
    {
        if (double.IsNaN(seconds) || seconds <= 0) return "0:00";
        var m = (int)(seconds / 60);
        var s = (int)(seconds % 60);
        return $"{m}:{s:00}";
    }

    public static string Upper(string? s) => (s ?? "").ToUpper(CultureInfo.InvariantCulture);

    private static string NormalizeCodec(string codec, string filePath)
    {
        if (string.IsNullOrEmpty(codec)) return "AUDIO";
        var c = codec.ToUpperInvariant();
        if (c.Contains("MPEG") || c == "MP3") return "MP3";
        if (c.Contains("VORBIS") || c == "OGG") return "OGG";
        if (c.Contains("PCM") || c == "WAV") return "WAV";
        if (c.Contains("WINDOWS MEDIA")) return "WMA";
        if (c.Contains("ALAC") || c.Contains("APPLE LOSSLESS")) return "ALAC";
        if (c.Contains("AAC")) return "AAC";
        if (c.Contains("OPUS")) return "OPUS";
        if (c.Contains("FLAC")) return "FLAC";
        var ext = Path.GetExtension(filePath).TrimStart('.');
        if (ext.Length > 0) return ext.ToUpperInvariant();
        return c.Length > 8 ? c[..8].Trim() : c;
    }

    /// <summary>Album quality line, ported from AlbumDetail.jsx: "FLAC / 24BIT / 48KHZ".</summary>
    public static string? AlbumQuality(IReadOnlyList<Track> tracks)
    {
        if (tracks.Count == 0) return null;
        var first = tracks[0];
        var codecs = tracks.Select(t => NormalizeCodec(t.Codec, t.FilePath)).Distinct().Count();
        var parts = new List<string> { codecs > 1 ? "MIXED" : NormalizeCodec(first.Codec, first.FilePath) };
        if (first.BitsPerSample > 0) parts.Add($"{first.BitsPerSample}BIT");
        if (first.SampleRate > 0)
        {
            var sr = first.SampleRate;
            parts.Add(sr >= 1000
                ? (sr % 1000 == 0 ? $"{sr / 1000}KHZ" : $"{(sr / 1000.0).ToString("0.0", CultureInfo.InvariantCulture)}KHZ")
                : $"{sr}HZ");
        }
        return string.Join(" / ", parts);
    }
}

/// <summary>A tile in the album/search grid (ALB or TRK card).</summary>
public abstract partial class CardItem : ObservableObject
{
    public abstract string Badge { get; }
    public abstract string Title { get; }
    public abstract string Subtitle { get; }
    public abstract string? Cover { get; }
    [ObservableProperty] private bool _isCurrent;
    /// <summary>Library SELECT mode: the card shows a checkbox and a click toggles it.</summary>
    [ObservableProperty] private bool _isSelectable;
    [ObservableProperty] private bool _isSelected;

    [RelayCommand] private void Open() => OnOpen();
    [RelayCommand] private void Play() => OnPlay();
    protected abstract void OnOpen();
    protected abstract void OnPlay();
}

public sealed class AlbumItem(Album album, MainViewModel owner) : CardItem
{
    public Album Album { get; } = album;
    public string Name => Album.Name;
    public string Artist => Album.Artist;
    public override string Badge => "ALB";
    public override string Title => Album.Name;
    public override string Subtitle { get; } = Fmt.Upper(album.Artist);
    public override string? Cover => Album.CoverThumbPath;
    protected override void OnOpen()
    {
        if (IsSelectable) owner.ToggleAlbumSelection(this);
        else owner.OpenAlbum(this);
    }
    protected override void OnPlay() => owner.Player.PlayQueue(Album.Tracks, 0);
}

public sealed class TrackCard(Track track, AlbumItem? album, MainViewModel owner) : CardItem
{
    public Track Track { get; } = track;
    public override string Badge => "TRK";
    public override string Title => Track.Title;
    public override string Subtitle { get; } = Fmt.Upper(track.Artist);
    public override string? Cover => Track.CoverThumbPath;
    protected override void OnOpen() => OnPlay();
    protected override void OnPlay() => owner.PlayTrackInAlbum(Track, album);
}

/// <summary>One virtualized row of cards (the grid is a list of rows).</summary>
public sealed class CardRow(IReadOnlyList<CardItem> items, int columns)
{
    public IReadOnlyList<CardItem> Items { get; } = items;
    public int Columns { get; } = columns;

    // Fixed slots instead of an ItemsControl: when a row container is recycled while scrolling,
    // each slot's ContentControl keeps its card visuals and only swaps the data.
    public CardItem? Slot0 => Items.Count > 0 ? Items[0] : null;
    public CardItem? Slot1 => Items.Count > 1 ? Items[1] : null;
    public CardItem? Slot2 => Items.Count > 2 ? Items[2] : null;
    public CardItem? Slot3 => Items.Count > 3 ? Items[3] : null;
    public CardItem? Slot4 => Items.Count > 4 ? Items[4] : null;
    public bool Has3 => Columns >= 3;
    public bool Has4 => Columns >= 4;
    public bool Has5 => Columns >= 5;
}

/// <summary>
/// LIBRARY_DATA header. The copy inside the list scrolls with the grid; in SELECT mode a second copy
/// pinned above the list takes over (the sticky header of LibraryGrid.jsx) and the list copy only keeps its space.
/// </summary>
public sealed partial class LibraryHeaderRow(MainViewModel vm, bool pinned) : ObservableObject
{
    public MainViewModel Vm { get; } = vm;
    public bool IsPinned { get; } = pinned;
    public bool ShowContent => IsPinned || !Vm.IsSelectMode;
    public void Refresh() => OnPropertyChanged(nameof(ShowContent));
}

public sealed class SectionHeaderRow(string text, bool first)
{
    public string Text { get; } = text;
    public bool IsFirst { get; } = first;
}

public sealed class EmptyRow(string text)
{
    public string Text { get; } = text;
}

public sealed class PlaylistCard(PlaylistItem playlist, MainViewModel owner) : CardItem
{
    public PlaylistItem Playlist { get; } = playlist;
    public override string Badge => "PL";
    public override string Title => Playlist.Name;
    public override string Subtitle => $"{Playlist.Data.TrackIds.Count} TRACKS";
    public override string? Cover => Playlist.Cover;
    protected override void OnOpen() => owner.OpenPlaylist(Playlist);
    protected override void OnPlay() => owner.Player.PlayQueue(owner.ResolveTracks(Playlist.Data.TrackIds), 0);
}

/// <summary>Sidebar entry for a user playlist.</summary>
public sealed partial class PlaylistItem(PlaylistData data, MainViewModel owner) : ObservableObject
{
    public PlaylistData Data { get; } = data;
    public string Name => Data.Name;
    /// <summary>Full-size custom cover (detail page).</summary>
    public string? CoverFull => CoverPaths.ToLocalPath(Data.CoverArt);
    /// <summary>Small cover for the sidebar and cards: the _thumb sibling when there is one.</summary>
    public string? Cover => CoverFull is { } full && PlaylistCovers.ThumbFor(full) is var thumb && File.Exists(thumb) ? thumb : CoverFull;
    public string UnitsText => $"{Data.TrackIds.Count} UNITS";
    public void Refresh()
    {
        OnPropertyChanged(nameof(UnitsText));
        OnPropertyChanged(nameof(Cover));
        OnPropertyChanged(nameof(CoverFull));
    }
    [RelayCommand] private void Open() => owner.OpenPlaylist(this);
}

public enum DetailKind { Album, Playlist, Liked }

/// <summary>Header of the album / playlist / liked-songs page (AlbumDetail.jsx).</summary>
public sealed partial class DetailHeaderRow : ObservableObject
{
    public DetailHeaderRow(MainViewModel vm, DetailKind kind, string name, string? artist, string? cover, IReadOnlyList<Track> tracks, int year)
    {
        Vm = vm;
        Kind = kind;
        Tracks = tracks;
        Name = Fmt.Upper(name.Length > 0 ? name : "UNTITLED");
        FullArtist = string.IsNullOrEmpty(artist) ? "UNKNOWN_ARTIST" : artist;
        Artist = Fmt.Upper(FullArtist);
        CoverFull = cover;
        Year = year;
        Quality = Fmt.AlbumQuality(tracks);
        RawCodec = tracks.Count > 0 && tracks[0].Codec.Length > 0 ? tracks[0].Codec : "Unknown";
    }

    public MainViewModel Vm { get; }
    public DetailKind Kind { get; }
    public IReadOnlyList<Track> Tracks { get; }
    public AlbumItem? Album { get; init; }
    public PlaylistItem? Playlist { get; init; }
    public string ChipText => Kind == DetailKind.Playlist ? "PLAYLIST" : "ALBUM_DATA";
    public string Name { get; }
    public string Artist { get; }
    public string FullArtist { get; }
    public string? CoverFull { get; }
    public string TrackCountText => $"{Tracks.Count} TRACKS";
    public int Year { get; }
    public bool HasYear => Year > 0;
    public string YearText => Year.ToString(CultureInfo.InvariantCulture);
    public string? Quality { get; }
    public bool HasQuality => Quality is not null;
    public string RawCodec { get; }
    public bool ShowLike => Kind != DetailKind.Liked;
    public bool ShowMenu => Kind != DetailKind.Liked;
    public bool IsPlaylist => Kind == DetailKind.Playlist;
    public bool IsAlbum => Kind == DetailKind.Album;
    public bool HasCustomCover => Playlist?.Data.CoverArt is not null;
    [ObservableProperty] private bool _isLiked;
}

public sealed class DiscHeaderRow(int disc, bool first)
{
    public string Text { get; } = $"Disc {disc}";
    public bool IsFirst { get; } = first;
}

public sealed class TrackColumnsRow(bool showCover)
{
    public bool ShowCover { get; } = showCover;
}

public sealed partial class TrackRow(Track track, int number, Action<TrackRow> play) : ObservableObject
{
    public Track Track { get; } = track;
    public string Number { get; } = "0" + number.ToString(CultureInfo.InvariantCulture); // same "0N" label as the Electron build
    public string Title => Track.Title;
    public string Artist { get; } = Fmt.Upper(track.Artist);
    public string DurationText => Fmt.Time(Track.Duration);
    public string? Cover => Track.CoverThumbPath;
    /// <summary>Playlists and liked songs show a small cover column, albums don't.</summary>
    public bool ShowCover { get; init; }

    [ObservableProperty] private bool _isCurrent;

    [RelayCommand] private void Play() => play(this);
}

/// <summary>Right-click menu content (COMMAND_MENU / ALBUM_OPERATIONS).</summary>
public sealed class MenuVm(string title, IBrush titleBrush, IReadOnlyList<MenuEntry> items)
{
    public string Title { get; } = title;
    public IBrush TitleBrush { get; } = titleBrush;
    public IReadOnlyList<MenuEntry> Items { get; } = items;
}

public sealed class MenuEntry
{
    public string Text { get; init; } = "";
    public string? Icon { get; init; }
    public IBrush? IconBrush { get; init; }
    public bool IconFilled { get; init; }
    public bool IsDanger { get; init; }
    /// <summary>Small grey caption (e.g. ADD_TO_PLAYLIST) instead of a clickable row.</summary>
    public bool IsCaption { get; init; }
    public bool IsSeparator { get; init; }
    /// <summary>Playlist rows: a 4px square bullet, orange with a check mark when the track is already in.</summary>
    public bool IsPlaylistRow { get; init; }
    public bool Checked { get; init; }
    public bool IsAction => !IsCaption && !IsSeparator;
    public bool HasIcon => Icon is not null;
    public Action? Execute { get; init; }
    public IRelayCommand Command => new RelayCommand(() => Execute?.Invoke());
}

/// <summary>A queue entry in the queue popup / full-screen "Next in Queue" list.</summary>
public sealed partial class QueueRow(Track track, int queueIndex, int offset, PlayerViewModel player)
{
    public Track Track { get; } = track;
    public int QueueIndex { get; } = queueIndex;
    /// <summary>Position in the whole queue (popup numbering).</summary>
    public string AbsNumber { get; } = (queueIndex + 1).ToString(CultureInfo.InvariantCulture);
    /// <summary>Position after the current track (full-screen numbering).</summary>
    public string RelNumber { get; } = offset.ToString(CultureInfo.InvariantCulture);
    public string Title => Track.Title;
    public string Artist => Track.Artist;
    public string ArtistUpper { get; } = Fmt.Upper(track.Artist);
    public string? Cover => Track.CoverThumbPath;

    [RelayCommand] private void Play() => player.PlayAt(QueueIndex);
    [RelayCommand] private void Remove() => player.RemoveAt(QueueIndex);
}

/// <summary>A row in the SELECT OUTPUT menu.</summary>
public sealed partial class AudioDeviceItem(string name, string label, PlayerViewModel player) : ObservableObject
{
    public string Name { get; } = name;
    public string Label { get; } = label;
    [ObservableProperty] private bool _isSelected;
    [RelayCommand] private void Select() => player.SelectAudioDevice(this);
}

public enum ToastKind { Success, Warning, Error, Info }

public sealed class ToastItem
{
    public ToastItem(string message, ToastKind kind)
    {
        Message = message;
        Kind = kind;
        var hex = kind switch
        {
            ToastKind.Warning => "#E8C060",
            ToastKind.Error => "#FF6B35",
            ToastKind.Info => "#888888",
            _ => "#4FD6BE",
        };
        var color = Color.Parse(hex);
        Accent = new SolidColorBrush(color);
        AccentBorder = new SolidColorBrush(Color.FromArgb(0x33, color.R, color.G, color.B));
        IconBorder = new SolidColorBrush(Color.FromArgb(0x44, color.R, color.G, color.B));
        IconFill = new SolidColorBrush(Color.FromArgb(0x11, color.R, color.G, color.B));
        AccentLine = new LinearGradientBrush
        {
            StartPoint = new Avalonia.RelativePoint(0, 0, Avalonia.RelativeUnit.Relative),
            EndPoint = new Avalonia.RelativePoint(1, 0, Avalonia.RelativeUnit.Relative),
            GradientStops = { new GradientStop(color, 0), new GradientStop(Colors.Transparent, 1) },
        };
        Icon = kind switch { ToastKind.Warning => "⚠", ToastKind.Error => "✗", ToastKind.Info => "−", _ => "✓" };
        TypeText = $"[{kind.ToString().ToUpperInvariant()}]";
        Time = DateTime.Now.ToString("HH:mm:ss", CultureInfo.InvariantCulture);
    }

    public string Message { get; }
    public ToastKind Kind { get; }
    public IBrush Accent { get; }
    public IBrush AccentBorder { get; }
    public IBrush IconBorder { get; }
    public IBrush IconFill { get; }
    public IBrush AccentLine { get; }
    public string Icon { get; }
    public string TypeText { get; }
    public string Time { get; }
}
