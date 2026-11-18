using System.Runtime.InteropServices;
using Avalonia;

namespace Retrograde.App;

internal static partial class Program
{
    [LibraryImport("libc", EntryPoint = "setenv", StringMarshalling = StringMarshalling.Utf8)]
    private static partial int SetEnv(string name, string value, int overwrite);

    /// <summary>GPU rendering (the default GL device, i.e. the integrated GPU on laptops) unless RETRO_RENDER=software.</summary>
    internal static bool UseGpu =>
        !string.Equals(Environment.GetEnvironmentVariable("RETRO_RENDER"), "software", StringComparison.OrdinalIgnoreCase);

    [STAThread]
    public static void Main(string[] args)
    {
        if (UseGpu)
        {
            // Swap without waiting for vblank (Mesa: Intel iris, AMD radeonsi; NVIDIA proprietary). Under XWayland the
            // swap otherwise blocks on frame callbacks, down to 1 fps; Avalonia paces frames itself. Set through libc
            // because .NET's Environment.SetEnvironmentVariable does not reach the native environment.
            // The GL device is left to the system: the default one is the integrated GPU; DRI_PRIME is the user's call.
            SetEnv("vblank_mode", "0", 0);
            SetEnv("__GL_SYNC_TO_VBLANK", "0", 0);
        }
        if (!Services.SingleInstance.Acquire(args)) return; // handed over to the running instance
        // Last-resort records: a crash, or a background task whose failure nobody awaited.
        AppDomain.CurrentDomain.UnhandledException += (_, e) =>
        {
            if (e.ExceptionObject is Exception ex) Retrograde.Core.ErrorLog.Write("fatal", ex);
        };
        TaskScheduler.UnobservedTaskException += (_, e) =>
        {
            Retrograde.Core.ErrorLog.Write("background task", e.Exception);
            e.SetObserved();
        };
        BuildAvaloniaApp().StartWithClassicDesktopLifetime(args, Avalonia.Controls.ShutdownMode.OnExplicitShutdown);
    }

    public static AppBuilder BuildAvaloniaApp()
    {
        var builder = AppBuilder.Configure<App>().UsePlatformDetect();
        // Native Wayland (Avalonia.Wayland, experimental in 12.1) is opt-in: RETRO_BACKEND=wayland. Default stays X11/XWayland.
        if (Environment.GetEnvironmentVariable("WAYLAND_DISPLAY") is { Length: > 0 }
            && string.Equals(Environment.GetEnvironmentVariable("RETRO_BACKEND"), "wayland", StringComparison.OrdinalIgnoreCase))
            builder = builder.UseWaylandWithFallback();
        return Configure(builder);
    }

    private static AppBuilder Configure(AppBuilder builder) =>
        builder
            .With(new X11PlatformOptions
            {
                // GL first, CPU Skia as the fallback; RETRO_RENDER=software skips GL.
                RenderingMode = UseGpu
                    ? [X11RenderingMode.Glx, X11RenderingMode.Egl, X11RenderingMode.Software]
                    : [X11RenderingMode.Software],
            })
            .LogToTrace();
}
