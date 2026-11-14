namespace Retrograde.Core;

/// <summary>
/// Changes to write into a file's tags. A null field is left as it is; an empty string clears it.
/// </summary>
public sealed class TagEdit
{
    public string? Title { get; init; }
    public string? Artist { get; init; }
    public string? Album { get; init; }
    public string? AlbumArtist { get; init; }
    public uint? Year { get; init; }
    public uint? Track { get; init; }
    public uint? Disc { get; init; }
    /// <summary>Image file to embed as the front cover.</summary>
    public string? CoverImage { get; init; }
    /// <summary>Remove the embedded pictures (ignored when <see cref="CoverImage"/> is set).</summary>
    public bool RemoveCover { get; init; }

    public bool ChangesCover => CoverImage is not null || RemoveCover;
}

/// <summary>Writes tags with TagLib#, the library that reads them in the scanner.</summary>
public static class TagWriter
{
    /// <summary>Applies the edit to one file. Throws on failure (read-only file, file in use, unsupported format).</summary>
    public static void Write(string path, TagEdit edit)
    {
        using var file = TagLib.File.Create(path);
        var tag = file.Tag;
        if (edit.Title is { } title) tag.Title = Blank(title);
        if (edit.Artist is { } artist) tag.Performers = Split(artist);
        if (edit.Album is { } album) tag.Album = Blank(album);
        if (edit.AlbumArtist is { } albumArtist) tag.AlbumArtists = Split(albumArtist);
        if (edit.Year is { } year) tag.Year = year;
        if (edit.Track is { } track) tag.Track = track;
        if (edit.Disc is { } disc) tag.Disc = disc;
        if (edit.CoverImage is { } image)
        {
            var picture = new TagLib.Picture(image)
            {
                Type = TagLib.PictureType.FrontCover,
                Description = "Cover",
            };
            tag.Pictures = [picture];
        }
        else if (edit.RemoveCover)
        {
            tag.Pictures = [];
        }
        file.Save();
    }

    private static string? Blank(string s) => s.Trim().Length == 0 ? null : s.Trim();

    /// <summary>The artist as one value, as typed ("A, B" stays one entry): the scanner reads the first value only.</summary>
    private static string[] Split(string s) => s.Trim().Length == 0 ? [] : [s.Trim()];
}
