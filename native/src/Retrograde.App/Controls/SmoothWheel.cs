using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Interactivity;
using Avalonia.VisualTree;

namespace Retrograde.App.Controls;

/// <summary>
/// Lenis-style smooth wheel scrolling for a ScrollViewer (or a ListBox's inner one).
/// Each wheel notch moves a target by <see cref="Step"/> DIPs; every rendered frame the offset
/// closes a fixed share of the remaining distance (exponential smoothing, the "lerp" Lenis uses).
/// Notches that arrive while it is moving add to the same target, so a fast flick of the wheel
/// becomes one continuous glide that slows down softly instead of a series of separate jumps.
/// The smoothing is frame-rate independent, so it feels the same at 60 Hz and 144 Hz.
/// </summary>
public static class SmoothWheel
{
    public const double Step = 100;        // Chromium on Windows: ~100 CSS px per wheel notch
    // Share of the remaining distance closed per second: Lenis' default lerp of 0.1 per 60 Hz
    // frame is about 6.3/s. Slightly firmer here, so a list of thousands of albums never feels sluggish.
    private const double Lambda = 8.5;
    private const double SettleDip = 0.5;  // close enough: snap and stop
    private const double TakeoverDip = 2;  // offset moved by something else (scrollbar drag, keyboard)

    public static readonly AttachedProperty<bool> IsEnabledProperty =
        AvaloniaProperty.RegisterAttached<Control, bool>("IsEnabled", typeof(SmoothWheel));

    public static bool GetIsEnabled(Control c) => c.GetValue(IsEnabledProperty);
    public static void SetIsEnabled(Control c, bool v) => c.SetValue(IsEnabledProperty, v);

    static SmoothWheel()
    {
        IsEnabledProperty.Changed.AddClassHandler<Control>((c, e) =>
        {
            if (e.NewValue is true) c.AddHandler(InputElement.PointerWheelChangedEvent, OnWheel, RoutingStrategies.Tunnel);
            else c.RemoveHandler(InputElement.PointerWheelChangedEvent, OnWheel);
        });
    }

    private sealed class State
    {
        public double Target;
        public double Current;   // where the animation put the offset last frame
        public long LastTicks;
        public bool Running;
        public Control? Host;
    }

    private static readonly System.Runtime.CompilerServices.ConditionalWeakTable<ScrollViewer, State> States = new();

    private static double MaxOffset(ScrollViewer sv) => Math.Max(0, sv.Extent.Height - sv.Viewport.Height);

    private static void OnWheel(object? sender, PointerWheelEventArgs e)
    {
        if (sender is not Control host || e.KeyModifiers != KeyModifiers.None) return;
        var sv = host as ScrollViewer ?? host.GetVisualDescendants().OfType<ScrollViewer>().FirstOrDefault();
        if (sv is null || sv.Extent.Height <= sv.Viewport.Height) return;
        // Let a nested scrollable (e.g. a flyout list) keep its own wheel.
        if (e.Source is Visual v && v.FindAncestorOfType<ScrollViewer>() is { } inner && inner != sv && inner.Extent.Height > inner.Viewport.Height) return;

        var st = States.GetOrCreateValue(sv);
        var max = MaxOffset(sv);
        // A notch while gliding extends the glide; otherwise start from where the list is now.
        var from = st.Running ? st.Target : sv.Offset.Y;
        st.Target = Math.Clamp(from - e.Delta.Y * Step, 0, max);
        e.Handled = true;
        if (st.Running) return;

        st.Current = sv.Offset.Y;
        st.LastTicks = System.Diagnostics.Stopwatch.GetTimestamp();
        var top = TopLevel.GetTopLevel(sv);
        if (top is null)
        {
            sv.Offset = sv.Offset.WithY(st.Target);
            return;
        }
        st.Running = true;
        st.Host = host;
        host.Classes.Add("scrolling"); // like Chromium: no hover effects while content moves under the pointer
        top.RequestAnimationFrame(_ => Tick(sv, st, top));
    }

    /// <summary>Stops a glide in progress, e.g. before the view jumps to another page.</summary>
    public static void Stop(ScrollViewer sv)
    {
        if (States.TryGetValue(sv, out var st)) Finish(st);
    }

    private static void Finish(State st)
    {
        st.Running = false;
        st.Host?.Classes.Remove("scrolling");
        st.Host = null;
    }

    // RETRO_FPS=1: log frame gaps of each scroll animation to %TEMP%\retro-frames.txt (dev measurement).
    private static readonly bool LogFrames = Environment.GetEnvironmentVariable("RETRO_FPS") == "1";
    private static readonly List<double> Gaps = [];

    private static void Tick(ScrollViewer sv, State st, TopLevel top)
    {
        if (!st.Running) return;
        var now = System.Diagnostics.Stopwatch.GetTimestamp();
        var dt = Math.Clamp(System.Diagnostics.Stopwatch.GetElapsedTime(st.LastTicks, now).TotalSeconds, 0, 0.1);
        st.LastTicks = now;
        if (LogFrames) Gaps.Add(dt * 1000);

        // Something else moved the list (scrollbar drag, keyboard, a jump to the top): give way to it.
        if (Math.Abs(sv.Offset.Y - st.Current) > TakeoverDip)
        {
            Finish(st);
            return;
        }

        // Virtualized lists re-estimate their height while scrolling, so the end can move.
        st.Target = Math.Clamp(st.Target, 0, MaxOffset(sv));
        var remaining = st.Target - st.Current;
        var y = Math.Abs(remaining) <= SettleDip
            ? st.Target
            : st.Current + remaining * (1 - Math.Exp(-Lambda * dt));
        st.Current = y;
        sv.Offset = sv.Offset.WithY(y);

        if (y != st.Target)
        {
            top.RequestAnimationFrame(_ => Tick(sv, st, top));
            return;
        }
        Finish(st);
        if (LogFrames && Gaps.Count > 0)
        {
            var sorted = Gaps.OrderBy(g => g).ToList();
            File.AppendAllText(Path.Combine(Path.GetTempPath(), "retro-frames.txt"),
                $"frames={Gaps.Count} avg={Gaps.Average():0.0}ms p95={sorted[(int)(sorted.Count * 0.95)]:0.0}ms max={sorted[^1]:0.0}ms over33={Gaps.Count(g => g > 33.4)}{Environment.NewLine}");
            Gaps.Clear();
        }
    }
}
