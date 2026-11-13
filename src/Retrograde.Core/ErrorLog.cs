namespace Retrograde.Core;

/// <summary>
/// Unexpected errors go to ~/.retrograde/native-error.log, so a crash leaves something to look at.
/// Best effort: logging never throws.
/// </summary>
public static class ErrorLog
{
    private const long MaxBytes = 512 * 1024;
    private static readonly object Gate = new();

    public static string FilePath => Path.Combine(CoverPaths.AppDir, "native-error.log");

    public static void Write(string where, Exception ex)
    {
        try
        {
            lock (Gate)
            {
                Directory.CreateDirectory(CoverPaths.AppDir);
                var info = new FileInfo(FilePath);
                if (info.Exists && info.Length > MaxBytes) File.Move(FilePath, FilePath + ".old", overwrite: true);
                File.AppendAllText(FilePath, $"{DateTime.Now:yyyy-MM-dd HH:mm:ss} [{where}] {ex}{Environment.NewLine}{Environment.NewLine}");
            }
        }
        catch { /* nothing else to do */ }
    }
}
