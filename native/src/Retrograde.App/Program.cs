using Avalonia;

namespace Retrograde.App;

internal static class Program
{
    [STAThread]
    public static void Main(string[] args)
    {
        if (!Services.SingleInstance.Acquire(args)) return; // handed over to the running instance
        BuildAvaloniaApp().StartWithClassicDesktopLifetime(args, Avalonia.Controls.ShutdownMode.OnExplicitShutdown);
    }

    public static AppBuilder BuildAvaloniaApp() =>
        AppBuilder.Configure<App>()
            .UsePlatformDetect()
            .With(RenderOptions())
            .LogToTrace();

    // Software (CPU Skia) rendering by default: measured on this app it idles at ~0.6% CPU
    // and ~67MB private memory, versus ~16% / ~286MB with ANGLE (GPU driver loaded in-process).
    // Set RETRO_RENDER=gpu to use ANGLE/D3D11 instead.
    private static Win32PlatformOptions RenderOptions()
    {
        var mode = Environment.GetEnvironmentVariable("RETRO_RENDER");
        return new Win32PlatformOptions
        {
            RenderingMode = string.Equals(mode, "gpu", StringComparison.OrdinalIgnoreCase)
                ? [Win32RenderingMode.AngleEgl, Win32RenderingMode.Software]
                : [Win32RenderingMode.Software],
        };
    }
}
