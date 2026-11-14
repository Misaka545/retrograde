using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Threading;
using Retrograde.App.ViewModels;

namespace Retrograde.App.Views;

/// <summary>Queue popup drag and drop, and keeping the sung lyrics line in view.</summary>
public partial class MainWindow
{
    private int _dragFrom = -1;
    private int _dragSlot = -1;

    // ---------- Queue: drag a row by its grip ----------

    private void OnQueueGripPressed(object? sender, PointerPressedEventArgs e)
    {
        if (sender is not Control grip || grip.DataContext is not QueueRow row || Player is null) return;
        if (!e.GetCurrentPoint(grip).Properties.IsLeftButtonPressed) return;
        _dragFrom = IndexOfRow(row);
        if (_dragFrom < 0) return;
        e.Pointer.Capture(grip);
        e.Handled = true; // not a click on the row: that would play it
        UpdateDrop(e);
    }

    private void OnQueueGripMoved(object? sender, PointerEventArgs e)
    {
        if (_dragFrom >= 0) UpdateDrop(e);
    }

    private void OnQueueGripReleased(object? sender, PointerReleasedEventArgs e)
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

    private void OnQueueGripCaptureLost(object? sender, PointerCaptureLostEventArgs e) => EndDrag();

    private void EndDrag()
    {
        _dragFrom = -1;
        _dragSlot = -1;
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
    private void UpdateDrop(PointerEventArgs e)
    {
        var count = Player?.Upcoming.Count ?? 0;
        if (count == 0 || QueueItems.ContainerFromIndex(Math.Max(0, _dragFrom)) is not Control container) return;
        var rowHeight = container.Bounds.Height;
        if (rowHeight <= 0) return;
        var y = e.GetPosition(QueueItems).Y;
        _dragSlot = Math.Clamp((int)Math.Round(y / rowHeight), 0, count);
        QueueDropLine.Margin = new Thickness(0, Math.Max(0, _dragSlot * rowHeight - 1), 0, 0);
        QueueDropLine.IsVisible = _dragSlot != _dragFrom && _dragSlot != _dragFrom + 1;
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
