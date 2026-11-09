using SkiaSharp;

namespace Retrograde.Core;

/// <summary>
/// Custom playlist covers: a full JPEG (quality 95) and a 300px-wide JPEG thumb (quality 80), as the
/// Electron saveCoverArt does. Files get their own native_pl_{id}_{ms} names rather than Electron's
/// hash of `playlist_${id}`: imported playlists keep their Electron ids, and sharing those files would
/// change the Electron build's covers too. A new name per change also defeats the image cache.
/// </summary>
public static class PlaylistCovers
{
    public static readonly string[] Extensions = [".jpg", ".jpeg", ".png", ".webp"];

    private const string Prefix = "native_pl_";

    /// <summary>Re-encodes the picked image; returns the coverArt URL (full image, with a cache-busting query).</summary>
    public static string Save(long playlistId, string sourcePath, string? previousCoverArt)
    {
        if (!Extensions.Contains(Path.GetExtension(sourcePath).ToLowerInvariant()))
            throw new NotSupportedException("Unsupported image format. Please use JPG, PNG, or WEBP.");
        using var bmp = SKBitmap.Decode(sourcePath) ?? throw new InvalidDataException("Image could not be decoded.");
        Directory.CreateDirectory(CoverPaths.CoverDir);
        var stem = Path.Combine(CoverPaths.CoverDir, $"{Prefix}{playlistId}_{DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()}");
        var full = stem + "_full.jpg";
        var thumb = stem + "_thumb.jpg";
        Write(bmp, full, 95);
        if (bmp.Width > 300)
        {
            var h = (int)Math.Round(bmp.Height * (300.0 / bmp.Width));
            using var small = bmp.Resize(new SKImageInfo(300, Math.Max(1, h)), new SKSamplingOptions(SKCubicResampler.Mitchell));
            Write(small ?? bmp, thumb, 80);
        }
        else Write(bmp, thumb, 80);
        Delete(previousCoverArt);
        return "file://" + full.Replace('\\', '/');
    }

    /// <summary>Removes a cover this build made; files it did not create (Electron imports) are left alone.</summary>
    public static void Delete(string? coverArt)
    {
        if (CoverPaths.ToLocalPath(coverArt) is not { } full) return;
        if (!Path.GetFileName(full).StartsWith(Prefix, StringComparison.Ordinal)) return;
        LibraryStore.TryDelete(full);
        LibraryStore.TryDelete(ThumbFor(full));
    }

    /// <summary>"x_full.jpg" → "x_thumb.jpg" (same pairing as album covers).</summary>
    public static string ThumbFor(string fullPath) =>
        fullPath.EndsWith("_full.jpg", StringComparison.OrdinalIgnoreCase) ? fullPath[..^"_full.jpg".Length] + "_thumb.jpg" : fullPath;

    private static void Write(SKBitmap bmp, string path, int quality)
    {
        using var img = SKImage.FromBitmap(bmp);
        using var data = img.Encode(SKEncodedImageFormat.Jpeg, quality);
        using var fs = File.Create(path);
        data.SaveTo(fs);
    }
}
