namespace Retrograde.Core;

public sealed class Track
{
    // Track identity is the absolute file path (same as the Electron build),
    // so playlists and liked songs stay valid across both apps.
    public required string Id { get; init; }
    public required string Title { get; init; }
    public string Artist { get; init; } = "Unknown Artist";
    public required string Album { get; init; }
    public double Duration { get; init; }
    public required string FilePath { get; init; }
    public int TrackNumber { get; init; }
    public int DiscNumber { get; init; } = 1;
    public string Codec { get; init; } = "";
    public int SampleRate { get; init; }
    public int BitsPerSample { get; init; }
    public int Channels { get; init; }

    /// <summary>Local path of the album thumbnail, if any.</summary>
    public string? CoverThumbPath { get; set; }
    public string? CoverFullPath { get; set; }

    public string QualityLabel
    {
        get
        {
            var codec = string.IsNullOrEmpty(Codec) ? Path.GetExtension(FilePath).TrimStart('.').ToUpperInvariant() : Codec.ToUpperInvariant();
            if (SampleRate <= 0) return codec;
            var khz = SampleRate % 1000 == 0 ? (SampleRate / 1000).ToString() : (SampleRate / 1000.0).ToString("0.#");
            return BitsPerSample > 0 ? $"{codec} {BitsPerSample}/{khz}" : $"{codec} {khz}kHz";
        }
    }
}

public sealed class Album
{
    public required string Name { get; init; }
    public string Artist { get; init; } = "Unknown Artist";
    public string? CoverThumbPath { get; init; }
    public string? CoverFullPath { get; init; }
    public int Year { get; init; }
    public long AddedAt { get; init; }
    public required IReadOnlyList<Track> Tracks { get; init; }
}
