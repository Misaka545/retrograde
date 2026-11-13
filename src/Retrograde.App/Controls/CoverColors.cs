using Avalonia.Media;
using SkiaSharp;

namespace Retrograde.App.Controls;

/// <summary>Colours picked from cover art, shared by the full-screen backdrop and the album page header.</summary>
public static class CoverColors
{
    /// <summary>
    /// The most prominent vivid colour: pixels are grouped by hue (24 bins), each weighted by its
    /// saturation and brightness, and the heaviest bin's average colour wins. Near-black, near-white
    /// and grey pixels count for little, so a dark cover with a red logo comes out red, not black.
    /// Grey-only covers fall back to their average tone (<paramref name="vivid"/> is then false).
    /// The colour is returned as found; callers set its saturation and brightness.
    /// </summary>
    public static SKColor Dominant(SKBitmap src, out bool vivid)
    {
        using var px = src.Resize(new SKImageInfo(48, 48), new SKSamplingOptions(SKFilterMode.Linear)) ?? src;
        const int bins = 24;
        var weight = new double[bins];
        var r = new double[bins];
        var g = new double[bins];
        var b = new double[bins];
        double ar = 0, ag = 0, ab = 0;
        int n = 0;
        for (int y = 0; y < px.Height; y++)
        for (int x = 0; x < px.Width; x++)
        {
            var c = px.GetPixel(x, y);
            ar += c.Red; ag += c.Green; ab += c.Blue; n++;
            c.ToHsv(out var h, out var s, out var v);
            s /= 100f; v /= 100f;
            if (v < 0.12 || s < 0.15) continue;
            var w = s * s * v;              // favour vivid over pale
            var i = (int)(h / 360f * bins) % bins;
            weight[i] += w;
            r[i] += c.Red * w; g[i] += c.Green * w; b[i] += c.Blue * w;
        }
        var best = 0;
        for (int i = 1; i < bins; i++) if (weight[i] > weight[best]) best = i;
        vivid = weight[best] > n * 0.01;
        return vivid
            ? new SKColor((byte)(r[best] / weight[best]), (byte)(g[best] / weight[best]), (byte)(b[best] / weight[best]))
            : new SKColor((byte)(ar / n), (byte)(ag / n), (byte)(ab / n));
    }

    /// <summary>The colour with its saturation and brightness (0–100) clamped.</summary>
    public static Color Clamp(SKColor c, float minS, float maxS, float minV, float maxV)
    {
        c.ToHsv(out var h, out var s, out var v);
        var p = SKColor.FromHsv(h, Math.Clamp(s, minS, maxS), Math.Clamp(v, minV, maxV));
        return Color.FromRgb(p.Red, p.Green, p.Blue);
    }

    /// <summary>Decodes a cover small (an eighth of its size or more), enough to pick colours from.</summary>
    public static SKBitmap? DecodeSmall(string path)
    {
        using var codec = SKCodec.Create(path);
        if (codec is null) return null;
        var info = codec.Info;
        if ((long)info.Width * info.Height > 64_000_000) return null;
        var scale = Math.Min(1f, 256f / Math.Max(1, Math.Min(info.Width, info.Height)));
        var size = codec.GetScaledDimensions(Math.Max(scale, 0.125f));
        var bmp = new SKBitmap(new SKImageInfo(size.Width, size.Height, SKColorType.Bgra8888, SKAlphaType.Premul));
        var result = codec.GetPixels(bmp.Info, bmp.GetPixels());
        if (result is SKCodecResult.Success or SKCodecResult.IncompleteInput) return bmp;
        bmp.Dispose();
        return null;
    }
}
