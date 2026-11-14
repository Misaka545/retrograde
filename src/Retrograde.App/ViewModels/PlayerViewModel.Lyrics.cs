using System.Globalization;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Retrograde.Core;

namespace Retrograde.App.ViewModels;

/// <summary>
/// Lyrics in the full-screen player: a .lrc file next to the track or the lyrics tag. Synced lyrics
/// follow playback (the line being sung lit, the list scrolled to it) and a click on a line seeks there.
/// </summary>
public partial class PlayerViewModel
{
    private int _lyricsVersion;
    private int _lyricIndex = -1;

    [ObservableProperty] [NotifyPropertyChangedFor(nameof(HasLyrics), nameof(LyricsStatus))]
    private IReadOnlyList<LyricRow> _lyricRows = [];

    [ObservableProperty] [NotifyPropertyChangedFor(nameof(LyricsStatus))]
    private bool _isLyricsLoading;

    [ObservableProperty] private string _lyricsSource = "";

    /// <summary>The full-screen side panel shows lyrics instead of the queue.</summary>
    [ObservableProperty] private bool _isLyricsTab;

    private bool _isFullScreenOpen;

    public bool HasLyrics => LyricRows.Count > 0;
    public string LyricsStatus => IsLyricsLoading ? "READING_LYRICS..." : HasLyrics ? "" : "NO LYRICS FOUND";

    /// <summary>Raised with the index of the line now being sung, so the view can scroll to it.</summary>
    public event Action<int>? LyricLineChanged;

    /// <summary>Set by the main view model when the full-screen player opens or closes.</summary>
    public bool IsFullScreenOpen
    {
        get => _isFullScreenOpen;
        set
        {
            if (_isFullScreenOpen == value) return;
            _isFullScreenOpen = value;
            UpdatePollRate();
        }
    }

    [RelayCommand] private void ShowQueueTab() => IsLyricsTab = false;
    [RelayCommand] private void ShowLyricsTab() => IsLyricsTab = true;

    partial void OnIsLyricsTabChanged(bool value)
    {
        UpdatePollRate();
        if (value && _lyricIndex >= 0) LyricLineChanged?.Invoke(_lyricIndex);
    }

    /// <summary>Position updates per second: 4 while synced lyrics are on screen (lines change quickly), else 2.</summary>
    private int UiTickMs => _isFullScreenOpen && IsLyricsTab && LyricRows.Count > 0 && LyricRows[0].IsSynced ? 250 : 500;

    private void UpdatePollRate()
    {
        var interval = TimeSpan.FromMilliseconds(UiTickMs);
        foreach (var engine in StartedEngines()) engine.PollInterval = interval;
    }

    private async void LoadLyrics(Track? track)
    {
        var version = ++_lyricsVersion;
        _lyricIndex = -1;
        LyricRows = [];
        LyricsSource = "";
        if (track is null) { IsLyricsLoading = false; return; }
        IsLyricsLoading = true;
        var doc = await Task.Run(() => LyricsLoader.Load(track.FilePath));
        if (version != _lyricsVersion) return; // the track changed meanwhile
        IsLyricsLoading = false;
        if (doc is not null)
        {
            LyricRows = doc.Lines.Select(l => new LyricRow(l, doc.IsSynced, this)).ToList();
            LyricsSource = doc.Source;
            _lyricsDoc = doc;
        }
        else
        {
            _lyricsDoc = null;
        }
        UpdatePollRate();
        UpdateLyricLine(_lastKnownPosition);
    }

    private LyricsDoc? _lyricsDoc;

    /// <summary>Lights the line being sung at this position (UI thread).</summary>
    private void UpdateLyricLine(double t)
    {
        if (_lyricsDoc is not { IsSynced: true } doc || LyricRows.Count != doc.Lines.Count) return;
        var index = doc.IndexAt(t + 0.15); // a little ahead: the poll lags behind the sound
        if (index == _lyricIndex) return;
        _lyricIndex = index;
        for (int i = 0; i < LyricRows.Count; i++)
        {
            LyricRows[i].IsCurrent = i == index;
            LyricRows[i].IsPast = i < index;
        }
        if (index >= 0) LyricLineChanged?.Invoke(index);
    }

    internal async void SeekToLyric(LyricRow row)
    {
        if (!row.IsSynced || CurrentTrack is null) return;
        await SeekAsync(row.Time);
        if (!IsPlaying) await TogglePlay();
        UpdateLyricLine(row.Time);
    }
}

public sealed partial class LyricRow(LyricLine line, bool synced, PlayerViewModel player) : ObservableObject
{
    public double Time { get; } = line.Time;
    /// <summary>Empty lines (stanza breaks) show as a small gap.</summary>
    public string Text { get; } = line.Text.Length == 0 ? "" : line.Text;
    public bool IsBreak => Text.Length == 0;
    public bool IsSynced { get; } = synced;
    public string TimeText => IsSynced ? TimeSpan.FromSeconds(Time).ToString(@"m\:ss", CultureInfo.InvariantCulture) : "";

    [ObservableProperty] private bool _isCurrent;
    [ObservableProperty] private bool _isPast;

    [RelayCommand] private void Seek() => player.SeekToLyric(this);
}
