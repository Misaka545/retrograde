namespace Retrograde.Core;

/// <summary>
/// Cover cache naming shared with the Electron build (~/.retrograde/covers/{hash}_thumb.jpg).
/// </summary>
public static class CoverPaths
{
    /// <summary>~/.retrograde, or RETRO_APPDIR when set (tests run against a scratch copy).</summary>
    public static string AppDir => Environment.GetEnvironmentVariable("RETRO_APPDIR") is { Length: > 0 } dir
        ? dir
        : Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), ".retrograde");
    public static string CoverDir => Path.Combine(AppDir, "covers");

    /// <summary>
    /// Bit-exact port of the JS hashName():
    /// hash = ((hash &lt;&lt; 5) - hash) + charCode, truncated to int32, then Math.abs(...).toString(36).
    /// JS charCodeAt and C# char are both UTF-16 code units, so iterating chars matches.
    /// </summary>
    public static string HashName(string str)
    {
        int hash = 0;
        foreach (char c in str)
        {
            hash = unchecked((hash << 5) - hash + c);
        }
        // Math.abs(int.MinValue) is 2147483648 in JS; widen to avoid the C# overflow.
        long abs = Math.Abs((long)hash);
        return ToBase36(abs);
    }

    public static string ThumbPath(string albumName) => Path.Combine(CoverDir, $"{HashName(albumName)}_thumb.jpg");
    public static string FullPath(string albumName) => Path.Combine(CoverDir, $"{HashName(albumName)}_full.jpg");

    private static string ToBase36(long value)
    {
        const string digits = "0123456789abcdefghijklmnopqrstuvwxyz";
        if (value == 0) return "0";
        Span<char> buf = stackalloc char[16];
        int i = buf.Length;
        while (value > 0)
        {
            buf[--i] = digits[(int)(value % 36)];
            value /= 36;
        }
        return new string(buf[i..]);
    }

    /// <summary>Turns "file:///C:/x/y.jpg" (Electron URL) or a plain path into a local path.</summary>
    public static string? ToLocalPath(string? urlOrPath)
    {
        if (string.IsNullOrWhiteSpace(urlOrPath)) return null;
        if (urlOrPath.StartsWith("file://", StringComparison.OrdinalIgnoreCase))
        {
            var p = urlOrPath["file://".Length..];
            if (p.StartsWith('/') && p.Length > 2 && p[2] == ':') p = p[1..]; // file:///C:/...
            return Uri.UnescapeDataString(p).Replace('/', Path.DirectorySeparatorChar);
        }
        return urlOrPath;
    }
}
