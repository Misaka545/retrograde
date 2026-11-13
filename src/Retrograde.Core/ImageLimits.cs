using SkiaSharp;

namespace Retrograde.Core;

/// <summary>
/// Checks an image header before decoding it. A decoded bitmap takes 4 bytes per pixel whatever the
/// file size, so a tiny crafted PNG can ask for gigabytes; nothing here needs more than a cover scan.
/// </summary>
public static class ImageLimits
{
    /// <summary>8000 x 8000: above any real cover, about 256 MB decoded.</summary>
    public const long MaxPixels = 64_000_000;

    public static bool IsSane(byte[] data)
    {
        try
        {
            using var stream = new SKMemoryStream(data);
            using var codec = SKCodec.Create(stream);
            return Fits(codec);
        }
        catch { return false; }
    }

    public static bool IsSane(string path)
    {
        try
        {
            using var codec = SKCodec.Create(path);
            return Fits(codec);
        }
        catch { return false; }
    }

    private static bool Fits(SKCodec? codec) =>
        codec is not null && codec.Info.Width > 0 && codec.Info.Height > 0
        && (long)codec.Info.Width * codec.Info.Height <= MaxPixels;
}
