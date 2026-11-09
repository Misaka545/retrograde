using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Interactivity;
using Avalonia.VisualTree;

namespace Retrograde.App.Controls;

/// <summary>
/// Chromium-style smooth wheel scrolling for a ScrollViewer (or a ListBox's inner one):
/// each notch moves the target by <see cref="Step"/> DIPs and the offset eases toward it over
/// ~150ms, one step per rendered frame. Without this, Avalonia jumps the whole step at once.
/// </summary>
public static class SmoothWheel
{
    public const double Step = 100;      // Chromium on Windows: ~100 CSS px per wheel notch
    private const double DurationMs = 160;

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
        public double From, Target;
        public long StartTicks;
        public bool Running;
        public Control? Host;
    }

    private static readonly System.Runtime.CompilerServices.ConditionalWeakTable<ScrollViewer, State> States = new();

    private static void OnWheel(object? sender, PointerWheelEventArgs e)
    {
        if (sender is not Control host || e.KeyModifiers != KeyModifiers.None) return;
        var sv = host as ScrollViewer ?? host.GetVisualDescendants().OfType<ScrollViewer>().FirstOrDefault();
        if (sv is null || sv.Extent.Height <= sv.Viewport.Height) return;
        // Let a nested scrollable (e.g. a flyout list) keep its own wheel.
        if (e.Source is Visual v && v.FindAncestorOfType<ScrollViewer>() is { } inner && inner != sv && inner.Extent.Height > inner.Viewport.Height) return;

        var st = States.GetOrCreateValue(sv);
        var max = Math.Max(0, sv.Extent.Height - sv.Viewport.Height);
        var current = sv.Offset.Y;
        var baseTarget = st.Running ? st.Target : current;
        st.Target = Math.Clamp(baseTarget - e.Delta.Y * Step, 0, max);
        st.From = current;
        st.StartTicks = Environment.TickCount64;
        e.Handled = true;
        if (st.Running) return;
        st.Running = true;
        st.Host = host;
        host.Classes.Add("scrolling"); // like Chromium: no hover effects while content moves under the pointer
        var top = TopLevel.GetTopLevel(sv);
        if (top is null) { sv.Offset = sv.Offset.WithY(st.Target); st.Running = false; host.Classes.Remove("scrolling"); return; }
        top.RequestAnimationFrame(_ => Tick(sv, st, top));
    }

    // RETRO_FPS=1: log frame gaps of each scroll animation to %TEMP%\retro-frames.txt (dev measurement).
    private static readonly bool LogFrames = Environment.GetEnvironmentVariable("RETRO_FPS") == "1";
    private static readonly List<double> Gaps = [];
    private static long _lastFrame;

    private static void Tick(ScrollViewer sv, State st, TopLevel top)
    {
        if (LogFrames)
        {
            var now = System.Diagnostics.Stopwatch.GetTimestamp();
            if (_lastFrame != 0) Gaps.Add(System.Diagnostics.Stopwatch.GetElapsedTime(_lastFrame, now).TotalMilliseconds);
            _lastFrame = now;
        }
        var t = Math.Clamp((Environment.TickCount64 - st.StartTicks) / DurationMs, 0, 1);
        var eased = 1 - Math.Pow(1 - t, 3); // ease-out cubic
        var y = st.From + (st.Target - st.From) * eased;
        sv.Offset = sv.Offset.WithY(y);
        if (t < 1) top.RequestAnimationFrame(_ => Tick(sv, st, top));
        else
        {
            st.Running = false;
            st.Host?.Classes.Remove("scrolling");
            if (LogFrames && Gaps.Count > 0)
            {
                var sorted = Gaps.OrderBy(g => g).ToList();
                File.AppendAllText(Path.Combine(Path.GetTempPath(), "retro-frames.txt"),
                    $"frames={Gaps.Count} avg={Gaps.Average():0.0}ms p95={sorted[(int)(sorted.Count * 0.95)]:0.0}ms max={sorted[^1]:0.0}ms over33={Gaps.Count(g => g > 33.4)}{Environment.NewLine}");
                Gaps.Clear();
                _lastFrame = 0;
            }
        }
    }
}
