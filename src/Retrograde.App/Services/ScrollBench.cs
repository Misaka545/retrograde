using System.Diagnostics;
using Avalonia.Controls;
using Avalonia.Threading;
using Avalonia.VisualTree;

namespace Retrograde.App.Services;

/// <summary>
/// RETRO_BENCH=scroll: dev measurement without input devices. A few seconds after the window opens the library
/// list is scrolled down and up at constant speed, one step per animation frame, and frame gaps plus process CPU
/// are written to retro-bench.txt in the temp folder; the app then exits. No debug overlays are involved.
/// </summary>
public static class ScrollBench
{
    public static bool Enabled => Environment.GetEnvironmentVariable("RETRO_BENCH") is "scroll" or "fullscreen";
    public static bool FullScreen => Environment.GetEnvironmentVariable("RETRO_BENCH") == "fullscreen";

    public static void Start(Window window)
    {
        // A hidden or covered window gets throttled frame callbacks from the compositor: keep it in front.
        window.Topmost = true;
        window.Activate();
        if (FullScreen) RunFullScreen(window);
        else DispatcherTimer.RunOnce(() => Run(window), TimeSpan.FromSeconds(6));
    }

    /// <summary>RETRO_BENCH=fullscreen (with RETRO_FSLOG=1): opens and closes the full-screen player three times;
    /// the frame gaps of each run go to retro-fs-frames.txt (see MainWindow.FullScreen).</summary>
    private static void RunFullScreen(Window window)
    {
        window.Width = 1900; window.Height = 1040; // like a maximized full-HD window
        var vm = window.DataContext as ViewModels.MainViewModel;
        int n = 0;
        var timer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(2.5) };
        timer.Tick += (_, _) =>
        {
            if (vm is null || n >= 6) { Environment.Exit(0); return; }
            vm.IsFullScreen = n % 2 == 0;
            n++;
        };
        DispatcherTimer.RunOnce(() => timer.Start(), TimeSpan.FromSeconds(6));
    }

    private static void Run(Window window)
    {
        var list = window.FindControl<ListBox>("LibraryList");
        var sv = list?.GetVisualDescendants().OfType<ScrollViewer>().FirstOrDefault();
        if (sv is null) { Write("no library list"); Environment.Exit(1); return; }
        const double speed = 2400;       // DIPs per second, a brisk wheel flick
        const double seconds = 8;
        var gaps = new List<double>();
        var proc = Process.GetCurrentProcess();
        var cpu0 = proc.TotalProcessorTime;
        var pssBefore = PssMb();
        var t0 = Stopwatch.GetTimestamp();
        var last = t0;
        var top = TopLevel.GetTopLevel(sv)!;
        var max = Math.Max(1, sv.Extent.Height - sv.Viewport.Height);
        void Tick(TimeSpan _)
        {
            var now = Stopwatch.GetTimestamp();
            var el = Stopwatch.GetElapsedTime(t0, now).TotalSeconds;
            gaps.Add(Stopwatch.GetElapsedTime(last, now).TotalMilliseconds);
            last = now;
            if (el >= seconds)
            {
                proc.Refresh();
                var cpu = (proc.TotalProcessorTime - cpu0).TotalSeconds / el * 100;
                gaps.RemoveAt(0);
                var s = gaps.OrderBy(g => g).ToList();
                var pssEnd = PssMb();
                GC.Collect(); GC.WaitForPendingFinalizers(); GC.Collect();
                Thread.Sleep(500);
                var pssGc = PssMb();
                Write($"backend={TopLevel.GetTopLevel(sv)?.TryGetPlatformHandle()?.HandleDescriptor} render={(Program.UseGpu ? "gpu" : "software")} extent={sv.Extent.Height:0} frames={gaps.Count} fps={gaps.Count / el:0.0} " +
                      $"avg={gaps.Average():0.0}ms p50={s[s.Count / 2]:0.0}ms p95={s[(int)(s.Count * 0.95)]:0.0}ms max={s[^1]:0.0}ms over20={gaps.Count(g => g > 20)} " +
                      $"cpu={cpu:0.0}% of one core pss={pssBefore}->{pssEnd}MB, after full GC {pssGc}MB");
                Environment.Exit(0);
                return;
            }
            // triangle wave: down, up, down ...
            var d = el * speed % (2 * max);
            sv.Offset = sv.Offset.WithY(d <= max ? d : 2 * max - d);
            top.RequestAnimationFrame(Tick);
        }
        top.RequestAnimationFrame(Tick);
    }

    /// <summary>Own PSS in MB from /proc/self/smaps_rollup (-1 when unavailable).</summary>
    private static long PssMb()
    {
        try
        {
            foreach (var l in File.ReadLines("/proc/self/smaps_rollup"))
                if (l.StartsWith("Pss:", StringComparison.Ordinal)) return long.Parse(l.Split(' ', StringSplitOptions.RemoveEmptyEntries)[1]) / 1024;
        }
        catch { }
        return -1;
    }

    private static void Write(string line) =>
        File.AppendAllText(Path.Combine(Path.GetTempPath(), "retro-bench.txt"), line + Environment.NewLine);
}
