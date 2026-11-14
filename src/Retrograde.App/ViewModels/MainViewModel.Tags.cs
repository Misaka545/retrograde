using System.Globalization;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Retrograde.Core;

namespace Retrograde.App.ViewModels;

/// <summary>
/// EDIT_TAGS: one track (title, artist, album, album artist, year, track and disc number, cover) or a whole
/// album (album, album artist, year, cover). Written into the files with TagLib#, then the files are read
/// again so the library, the album grouping and the cover follow.
/// </summary>
public partial class MainViewModel
{
    [ObservableProperty] private TagEditorVm? _tagEditor;

    public void StartTagEdit(Track track)
    {
        _tracksById.TryGetValue(track.Id, out var e);
        TagEditor = TagEditorVm.ForTrack(this, track, e.Album?.Album);
    }

    [RelayCommand]
    private void EditCurrentAlbumTags()
    {
        if (CurrentHeader is { Album: { } a }) TagEditor = TagEditorVm.ForAlbum(this, a.Album);
    }

    [RelayCommand] private void CancelTagEditor() => TagEditor = null;

    public async Task SaveTagsAsync(TagEditorVm editor)
    {
        if (editor.Validate() is { } problem)
        {
            ShowToast($"TAGS_INVALID // {problem}", ToastKind.Warning);
            return;
        }
        var edit = editor.ToEdit();
        var tracks = editor.Tracks;
        if (!editor.HasChanges)
        {
            TagEditor = null;
            return;
        }
        editor.IsSaving = true;
        var failed = new List<string>();
        await Task.Run(() =>
        {
            foreach (var t in tracks)
            {
                try { TagWriter.Write(t.FilePath, edit); }
                catch (Exception ex)
                {
                    ErrorLog.Write($"tag write {t.FilePath}", ex);
                    failed.Add($"{Path.GetFileName(t.FilePath)}: {ex.Message}");
                }
            }
        });
        var written = tracks.Count - failed.Count;
        if (written > 0)
        {
            try
            {
                // A new or removed picture: the album's cover is taken again on the scan below
                if (edit.ChangesCover && editor.AlbumKey is { } key)
                {
                    foreach (var path in new[] { CoverPaths.ThumbPath(key), CoverPaths.FullPath(key) })
                        Services.CoverLoader.Invalidate(path);
                    await LibraryStore.ClearCoversAsync([key]);
                }
                if (_watchScan is { } running) await running;
                await Scanner.ScanAsync(tracks.Select(t => t.FilePath).ToList(), null);
                await LoadLibraryAsync();
                if (HasSearchQuery) RebuildSearch();
                Player.RefreshTracks(id => _tracksById.TryGetValue(id, out var e) ? e.Track : null);
                ReopenAfterTagEdit(tracks[0].Id);
                AfterCollectionsChanged();
            }
            catch (Exception ex)
            {
                ErrorLog.Write("tag rescan", ex);
                ShowToast($"RESCAN_FAILED // {ex.Message}", ToastKind.Error);
            }
        }
        editor.IsSaving = false;
        if (failed.Count == 0)
        {
            TagEditor = null;
            ShowToast(written == 1 ? "TAGS_SAVED // 1 FILE" : $"TAGS_SAVED // {written} FILES", ToastKind.Success);
        }
        else
        {
            ShowToast($"TAG_WRITE_FAILED // {failed.Count} FILE(S): {failed[0]}", ToastKind.Error);
            if (written > 0) TagEditor = null;
        }
    }

    /// <summary>The page on screen shows the new tags: the album page follows its tracks, even under a new name.</summary>
    private void ReopenAfterTagEdit(string trackId)
    {
        switch (CurrentHeader)
        {
            case { Kind: DetailKind.Album }:
                if (_tracksById.TryGetValue(trackId, out var e)) OpenAlbum(e.Album, keepScroll: true);
                break;
            case { Kind: DetailKind.Artist, Artist: { } artist }:
                if (FindArtist(artist.Name) is { } again) OpenArtist(again, keepScroll: true);
                else CurrentView = AppView.Library;
                break;
        }
    }
}

/// <summary>The EDIT_TAGS dialog.</summary>
public sealed partial class TagEditorVm : ObservableObject
{
    private readonly MainViewModel _owner;
    private string _title0 = "", _artist0 = "", _album0 = "", _albumArtist0 = "", _year0 = "", _track0 = "", _disc0 = "";

    private TagEditorVm(MainViewModel owner, IReadOnlyList<Track> tracks, bool isAlbum, string? albumKey)
    {
        _owner = owner;
        Tracks = tracks;
        IsAlbum = isAlbum;
        AlbumKey = albumKey;
    }

    public static TagEditorVm ForTrack(MainViewModel owner, Track t, Album? album)
    {
        var vm = new TagEditorVm(owner, [t], false, album?.Name);
        var tags = ReadTags(t.FilePath);
        vm.Init(tags?.Title ?? t.Title, tags?.Artist ?? t.Artist, tags?.Album ?? album?.Name ?? t.Album,
            tags?.AlbumArtist ?? "", tags?.Year ?? (album?.Year ?? 0), tags?.Track ?? t.TrackNumber, tags?.Disc ?? t.DiscNumber);
        vm.Target = Path.GetFileName(t.FilePath);
        vm.CurrentCover = t.CoverFullPath ?? t.CoverThumbPath;
        return vm;
    }

    public static TagEditorVm ForAlbum(MainViewModel owner, Album album)
    {
        var vm = new TagEditorVm(owner, album.Tracks, true, album.Name);
        var first = album.Tracks.Count > 0 ? ReadTags(album.Tracks[0].FilePath) : null;
        // The album name as tagged: the library may show it with " (Artist)" added to tell two apart
        vm.Init("", "", first?.Album ?? album.Name, first?.AlbumArtist ?? "", first?.Year ?? album.Year, 0, 0);
        vm.Target = album.Tracks.Count == 1 ? "1 FILE" : $"{album.Tracks.Count} FILES";
        vm.CurrentCover = album.CoverFullPath ?? album.CoverThumbPath;
        return vm;
    }

    private sealed record Tags(string Title, string Artist, string Album, string AlbumArtist, int Year, int Track, int Disc);

    /// <summary>The values as tagged (the library keeps only what it shows).</summary>
    private static Tags? ReadTags(string path)
    {
        try
        {
            using var f = TagLib.File.Create(path, TagLib.ReadStyle.Average | TagLib.ReadStyle.PictureLazy);
            var t = f.Tag;
            return new Tags(t.Title ?? "", t.JoinedPerformers ?? "", t.Album ?? "", t.JoinedAlbumArtists ?? "", (int)t.Year, (int)t.Track, (int)t.Disc);
        }
        catch { return null; }
    }

    private void Init(string title, string artist, string album, string albumArtist, int year, int track, int disc)
    {
        static string N(int v) => v > 0 ? v.ToString(CultureInfo.InvariantCulture) : "";
        _title0 = Title = title;
        _artist0 = Artist = artist;
        _album0 = Album = album;
        _albumArtist0 = AlbumArtist = albumArtist;
        _year0 = Year = N(year);
        _track0 = TrackNo = N(track);
        _disc0 = DiscNo = N(disc);
    }

    public IReadOnlyList<Track> Tracks { get; }
    public bool IsAlbum { get; }
    public bool IsTrack => !IsAlbum;
    /// <summary>The library album the tracks are in (its cover is taken again after a picture change).</summary>
    public string? AlbumKey { get; }
    public string Heading => IsAlbum ? "EDIT_ALBUM_TAGS" : "EDIT_TAGS";
    public string Target { get; private set; } = "";
    public string? CurrentCover { get; private set; }

    [ObservableProperty] private string _title = "";
    [ObservableProperty] private string _artist = "";
    [ObservableProperty] private string _album = "";
    [ObservableProperty] private string _albumArtist = "";
    [ObservableProperty] private string _year = "";
    [ObservableProperty] private string _trackNo = "";
    [ObservableProperty] private string _discNo = "";
    [ObservableProperty] private bool _isSaving;

    /// <summary>Image picked to embed; null = keep the pictures in the files.</summary>
    [ObservableProperty] [NotifyPropertyChangedFor(nameof(CoverPreview), nameof(CoverNote))]
    private string? _newCover;

    [ObservableProperty] [NotifyPropertyChangedFor(nameof(CoverPreview), nameof(CoverNote))]
    private bool _removeCover;

    public string? CoverPreview => NewCover ?? (RemoveCover ? null : CurrentCover);
    public string CoverNote => NewCover is not null ? "NEW COVER, EMBEDDED ON SAVE"
        : RemoveCover ? "EMBEDDED COVER REMOVED ON SAVE (A COVER IMAGE IN THE FOLDER STILL SHOWS)"
        : IsAlbum ? "EMBEDDED IN EVERY FILE OF THE ALBUM" : "EMBEDDED IN THIS FILE";

    public void PickCover(string path)
    {
        NewCover = path;
        RemoveCover = false;
    }

    [RelayCommand]
    private void ClearCover()
    {
        NewCover = null;
        RemoveCover = true;
    }

    [RelayCommand] private async Task Save() => await _owner.SaveTagsAsync(this);

    public bool HasChanges => ToEditFields().Any || NewCover is not null || RemoveCover;

    /// <summary>A number field that is not a number, or null when everything can be written.</summary>
    public string? Validate()
    {
        foreach (var (label, text) in new[] { ("YEAR", Year), ("TRACK", TrackNo), ("DISC", DiscNo) })
            if (text.Trim().Length > 0 && !uint.TryParse(text.Trim(), NumberStyles.Integer, CultureInfo.InvariantCulture, out _))
                return $"{label} MUST BE A NUMBER";
        if (IsTrack && Title.Trim().Length == 0) return "TITLE IS EMPTY";
        if (Album.Trim().Length == 0) return "ALBUM IS EMPTY";
        return null;
    }

    private (bool Any, string? Title, string? Artist, string? Album, string? AlbumArtist, uint? Year, uint? Track, uint? Disc) ToEditFields()
    {
        static uint? Num(string s) => uint.TryParse(s.Trim(), NumberStyles.Integer, CultureInfo.InvariantCulture, out var v) ? v : s.Trim().Length == 0 ? 0 : null;
        // Only what was changed is written: an album edit must not overwrite each track's own title
        var title = IsTrack && Title != _title0 ? Title : null;
        var artist = IsTrack && Artist != _artist0 ? Artist : null;
        var album = Album != _album0 ? Album : null;
        var albumArtist = AlbumArtist != _albumArtist0 ? AlbumArtist : null;
        var year = Year != _year0 ? Num(Year) : null;
        var track = IsTrack && TrackNo != _track0 ? Num(TrackNo) : null;
        var disc = IsTrack && DiscNo != _disc0 ? Num(DiscNo) : null;
        var any = title is not null || artist is not null || album is not null || albumArtist is not null
                  || year is not null || track is not null || disc is not null;
        return (any, title, artist, album, albumArtist, year, track, disc);
    }

    public TagEdit ToEdit()
    {
        var f = ToEditFields();
        return new TagEdit
        {
            Title = f.Title, Artist = f.Artist, Album = f.Album, AlbumArtist = f.AlbumArtist,
            Year = f.Year, Track = f.Track, Disc = f.Disc,
            CoverImage = NewCover, RemoveCover = RemoveCover && NewCover is null,
        };
    }
}
