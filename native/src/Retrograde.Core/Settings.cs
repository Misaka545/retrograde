using System.Text.Json;
using System.Text.Json.Serialization;

namespace Retrograde.Core;

public sealed class AppSettings
{
    public double Volume { get; set; } = 0.5;
    public bool ExclusiveMode { get; set; }
    /// <summary>mpv --audio-device name; "auto" follows the Windows default device.</summary>
    public string AudioDevice { get; set; } = "auto";
    public string LibrarySort { get; set; } = "added_desc";
    /// <summary>Close button behaviour: null = ask (BACKGROUND_PLAY), true = hide to tray, false = quit.</summary>
    public bool? MinimizeToTray { get; set; }

    private static string FilePath => Path.Combine(CoverPaths.AppDir, "native-settings.json");

    public static AppSettings Load()
    {
        try
        {
            if (File.Exists(FilePath))
                return JsonSerializer.Deserialize(File.ReadAllText(FilePath), SettingsJsonContext.Default.AppSettings) ?? new();
        }
        catch { /* fall back to defaults */ }
        return new AppSettings { MinimizeToTray = ElectronMinimizeToTray() };
    }

    /// <summary>First run: reuse the choice saved by the Electron build (%APPDATA%\retrograde\retrograde-config.json).</summary>
    private static bool? ElectronMinimizeToTray()
    {
        try
        {
            var path = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "retrograde", "retrograde-config.json");
            if (!File.Exists(path)) return null;
            using var doc = JsonDocument.Parse(File.ReadAllText(path));
            return doc.RootElement.TryGetProperty("minimizeToTray", out var v) && v.ValueKind is JsonValueKind.True or JsonValueKind.False
                ? v.GetBoolean()
                : null;
        }
        catch { return null; }
    }

    public void Save()
    {
        try
        {
            Directory.CreateDirectory(CoverPaths.AppDir);
            File.WriteAllText(FilePath, JsonSerializer.Serialize(this, SettingsJsonContext.Default.AppSettings));
        }
        catch { /* settings are best-effort */ }
    }
}

[JsonSourceGenerationOptions(WriteIndented = true)]
[JsonSerializable(typeof(AppSettings))]
internal partial class SettingsJsonContext : JsonSerializerContext;
