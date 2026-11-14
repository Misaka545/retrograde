using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.Presenters;
using Avalonia.Input;
using Avalonia.Threading;
using Avalonia.VisualTree;
using Retrograde.App.ViewModels;

namespace Retrograde.App.Views;

/// <summary>Queue popup placement and drag and drop, and keeping the sung lyrics line in view.</summary>
public partial class MainWindow
{
    // ---------- The player-bar popups: one shared slot ----------
    // Queue, equalizer and output open in the same place: 320 wide, right edge on the full-screen
    // button's right edge (the end of the bar's button group), bottom PopupGap above the player bar.
    // Switching between them swaps the content of one panel instead of moving a box sideways.

    /// <summary>The right edge all three popups share, in window coordinates.</summary>
    private double SlotRight(Visual relativeTo) =>
        FullScreenButton.TranslatePoint(new Point(FullScreenButton.Bounds.Width, 0), relativeTo)?.X ?? 0;

    /// <summary>
    /// Where the popups end: PopupGap above the player bar, clear of the seek strip that straddles its
    /// top edge. In the coordinates of <paramref name="relativeTo"/>.
    /// </summary>
    private double SlotBottom(Visual relativeTo) =>
        (PlayerBar.TranslatePoint(new Point(0, 0), relativeTo)?.Y ?? 0) - PopupGap;

    /// <summary>
    /// A menu opens above its own button, right-aligned to it: shift it to the shared right edge and
    /// the shared bottom line.
    /// </summary>
    private void AlignPopup(Flyout? flyout, Control button)
    {
        if (flyout is null || button.TranslatePoint(new Point(button.Bounds.Width, 0), this) is not { } corner) return;
        flyout.HorizontalOffset = SlotRight(this) - corner.X;
        flyout.VerticalOffset = SlotBottom(this) - corner.Y;
    }

    /// <summary>Space between the popups and the top of the player bar.</summary>
    private const double PopupGap = 16;

    /// <summary>The queue popup in the shared slot.</summary>
    private void PlaceQueuePopup()
    {
        if (QueuePopup.GetVisualParent() is not Visual parent) return;
        var right = Math.Max(8, parent.Bounds.Width - SlotRight(parent));
        var bottom = Math.Max(8, parent.Bounds.Height - SlotBottom(parent));
        QueuePopup.Margin = new Thickness(0, 0, right, bottom);
        // At most 60% of the window and 520px, and never above the top of the window
        QueuePopup.MaxHeight = Math.Clamp(Math.Min(parent.Bounds.Height * 0.6, parent.Bounds.Height - bottom - 48), 160, 520);
    }

    // ---------- Queue: drag a row by its grip ----------

    private int _dragFrom = -1;
    private int _dragSlot = -1;
    private double _dragRowHeight;
    /// <summary>Pointer position in the queue's viewport, kept for scrolling with the pointer still.</summary>
    private double _dragViewportY;
    private DispatcherTimer? _dragScroll;

    private void InitQueueDrag()
    {
        // The pointer is captured by the list itself: the row it started on may scroll out of view and
        // be recycled for another one, which would end a capture held by the row
        QueueItems.PointerMoved += (_, e) => { if (_dragFrom >= 0) TrackDrag(e); };
        QueueItems.PointerReleased += OnQueueDragReleased;
        QueueItems.PointerCaptureLost += (_, _) => EndDrag();
        // Wheel scrolling during a drag moves the rows under a still pointer: follow them
        QueueScroll.ScrollChanged += (_, _) => { if (_dragFrom >= 0) UpdateDropLine(); };
    }

    private void OnQueueGripPressed(object? sender, PointerPressedEventArgs e)
    {
        if (sender is not Control grip || grip.DataContext is not QueueRow row || Player is null) return;
        if (!e.GetCurrentPoint(grip).Properties.IsLeftButtonPressed) return;
        var from = IndexOfRow(row);
        if (from < 0 || grip.FindAncestorOfType<ContentPresenter>() is not { } container) return;
        _dragFrom = from;
        _dragRowHeight = container.Bounds.Height;
        e.Pointer.Capture(QueueItems);
        e.Handled = true; // not a click on the row: that would play it
        TrackDrag(e);
        _dragScroll ??= new DispatcherTimer(TimeSpan.FromMilliseconds(16), DispatcherPriority.Input, (_, _) => AutoScroll());
        _dragScroll.Start();
    }

    private void TrackDrag(PointerEventArgs e)
    {
        _dragViewportY = e.GetPosition(QueueScroll).Y;
        UpdateDropLine();
    }

    private void OnQueueDragReleased(object? sender, PointerReleasedEventArgs e)
    {
        if (_dragFrom < 0) return;
        var from = _dragFrom;
        var slot = _dragSlot;
        EndDrag();
        e.Pointer.Capture(null);
        e.Handled = true;
        // A slot is a gap between rows: past the row itself, the row's own removal shifts it by one
        var to = slot > from ? slot - 1 : slot;
        if (slot >= 0 && to != from) Player?.MoveUpcoming(from, to);
    }

    private void EndDrag()
    {
        _dragFrom = -1;
        _dragSlot = -1;
        _dragScroll?.Stop();
        QueueDropLine.IsVisible = false;
    }

    private int IndexOfRow(QueueRow row)
    {
        var rows = Player?.Upcoming;
        if (rows is null) return -1;
        for (int i = 0; i < rows.Count; i++)
            if (ReferenceEquals(rows[i], row)) return i;
        return -1;
    }

    /// <summary>The gap under the pointer (0 = before the first row) and the gold line there. Rows are all one height.</summary>
    private void UpdateDropLine()
    {
        var count = Player?.Upcoming.Count ?? 0;
        if (count == 0 || _dragRowHeight <= 0) return;
        if (QueueItems.TranslatePoint(new Point(0, 0), QueueScroll) is not { } itemsTop) return;
        var y = _dragViewportY - itemsTop.Y; // pointer in the list's own coordinates
        _dragSlot = Math.Clamp((int)Math.Round(y / _dragRowHeight), 0, count);
        QueueDropLine.Margin = new Thickness(0, Math.Max(0, _dragSlot * _dragRowHeight - 1), 0, 0);
        QueueDropLine.IsVisible = _dragSlot != _dragFrom && _dragSlot != _dragFrom + 1;
    }

    /// <summary>Near the top or bottom edge of the list the queue scrolls by itself, faster closer to the edge.</summary>
    private void AutoScroll()
    {
        if (_dragFrom < 0) { _dragScroll?.Stop(); return; }
        const double zone = 40, maxSpeed = 14; // DIPs, DIPs per frame
        var h = QueueScroll.Viewport.Height;
        var speed = _dragViewportY < zone ? -maxSpeed * (zone - _dragViewportY) / zone
                  : _dragViewportY > h - zone ? maxSpeed * (_dragViewportY - (h - zone)) / zone
                  : 0;
        if (speed == 0) return;
        var max = Math.Max(0, QueueScroll.Extent.Height - h);
        var next = Math.Clamp(QueueScroll.Offset.Y + Math.Clamp(speed, -maxSpeed, maxSpeed), 0, max);
        if (next != QueueScroll.Offset.Y) QueueScroll.Offset = QueueScroll.Offset.WithY(next); // ScrollChanged moves the line
    }

    // ---------- Lyrics: keep the sung line in the middle ----------

    private DispatcherTimer? _lyricScroll;
    private double _lyricScrollFrom, _lyricScrollTo;
    private DateTime _lyricScrollStart;

    private void InitLyrics(PlayerViewModel player)
    {
        player.LyricLineChanged -= OnLyricLineChanged;
        player.LyricLineChanged += OnLyricLineChanged;
    }

    private void OnLyricLineChanged(int index)
    {
        if (!LyricsScroll.IsEffectivelyVisible) return;
        // Layout first: the rows of a new track may not be measured yet
        Dispatcher.UIThread.Post(() => ScrollToLyric(index), DispatcherPriority.Loaded);
    }

    private void ScrollToLyric(int index)
    {
        if (LyricsItems.ContainerFromIndex(index) is not Control row) return;
        if (row.TranslatePoint(new Point(0, 0), LyricsItems) is not { } at) return;
        var viewport = LyricsScroll.Viewport.Height;
        var max = Math.Max(0, LyricsScroll.Extent.Height - viewport);
        // A third from the top: the lines to come are what the eye looks for
        var target = Math.Clamp(at.Y + LyricsItems.Margin.Top - viewport * 0.35 + row.Bounds.Height / 2, 0, max);
        _lyricScrollFrom = LyricsScroll.Offset.Y;
        _lyricScrollTo = target;
        _lyricScrollStart = DateTime.UtcNow;
        _lyricScroll ??= new DispatcherTimer(TimeSpan.FromMilliseconds(16), DispatcherPriority.Render, (_, _) => StepLyricScroll());
        _lyricScroll.Start();
    }

    /// <summary>Eased scroll over 350 ms (ease-out cubic).</summary>
    private void StepLyricScroll()
    {
        var t = Math.Min(1, (DateTime.UtcNow - _lyricScrollStart).TotalMilliseconds / 350);
        var e = 1 - Math.Pow(1 - t, 3);
        LyricsScroll.Offset = new Vector(LyricsScroll.Offset.X, _lyricScrollFrom + (_lyricScrollTo - _lyricScrollFrom) * e);
        if (t >= 1) _lyricScroll?.Stop();
    }
}
