using System.Collections.ObjectModel;
using System.Globalization;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Retrograde.Core;

namespace Retrograde.App.ViewModels;

/// <summary>
/// Smart playlists: rules over the library (format, bit depth, plays, last played, added, favorite...)
/// worked out each time the list is shown, so they follow the library and the play counts by themselves.
/// Stored next to the playlists in native-collections.json.
/// </summary>
public partial class MainViewModel
{
    public ObservableCollection<SmartPlaylistItem> SmartPlaylists { get; } = [];

    [ObservableProperty] private SmartEditorVm? _smartEditor;

    /// <summary>Loads the smart playlists from the collections file; the first time, a few examples.</summary>
    private void LoadSmartPlaylists()
    {
        if (_collections.Smart is null)
        {
            _collections.Smart = Retrograde.Core.SmartPlaylists.Examples();
            CollectionsStore.Save(_collections);
        }
        SmartPlaylists.Clear();
        foreach (var s in _collections.Smart) SmartPlaylists.Add(new SmartPlaylistItem(s, this));
        RefreshSmartPlaylists();
    }

    private IEnumerable<(Track Track, Album Album)> LibraryEntries() =>
        _allAlbums.SelectMany(a => a.Album.Tracks.Select(t => (t, a.Album)));

    public List<Track> EvaluateSmart(SmartPlaylistData data) =>
        Retrograde.Core.SmartPlaylists.Evaluate(data, LibraryEntries(), _stats.Get, id => _liked.Contains(id), DateTimeOffset.Now);

    /// <summary>Track counts and covers in the sidebar (cheap: a pass over the library per playlist).</summary>
    private void RefreshSmartPlaylists()
    {
        foreach (var s in SmartPlaylists) s.Refresh();
    }

    public void OpenSmart(SmartPlaylistItem item) => OpenSmart(item, false);

    private void OpenSmart(SmartPlaylistItem item, bool keepScroll)
    {
        var tracks = EvaluateSmart(item.Data);
        item.Refresh(tracks);
        ShowTrackList(new DetailHeaderRow(this, DetailKind.Smart, item.Name, item.RulesText, item.CoverFull, tracks, 0) { Smart = item },
            () => OpenSmart(item, false), keepScroll);
    }

    private void SaveSmart()
    {
        _collections.Smart = SmartPlaylists.Select(s => s.Data).ToList();
        CollectionsStore.Save(_collections);
    }

    // ---------- Editor ----------

    [RelayCommand]
    private void StartCreateSmart()
    {
        SmartEditor = new SmartEditorVm(this, null);
        IsAlbumsMode = false;
    }

    [RelayCommand]
    private void EditCurrentSmart()
    {
        if (CurrentHeader is { Smart: { } s }) SmartEditor = new SmartEditorVm(this, s);
    }

    [RelayCommand] private void CancelSmartEditor() => SmartEditor = null;

    /// <summary>The editor's SAVE: a new playlist is added, an edited one is changed in place.</summary>
    public void CommitSmart(SmartEditorVm editor)
    {
        var data = editor.ToData();
        if (editor.Editing is { } item)
        {
            item.Data.Name = data.Name;
            item.Data.MatchAll = data.MatchAll;
            item.Data.Rules = data.Rules;
            item.Data.Sort = data.Sort;
            item.Data.Limit = data.Limit;
            item.NotifyName();
            SaveSmart();
            item.Refresh();
            if (CurrentHeader?.Smart == item) OpenSmart(item, keepScroll: true);
            ShowToast($"SMART_PLAYLIST_SAVED // {data.Name}", ToastKind.Success);
        }
        else
        {
            data.Id = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
            var added = new SmartPlaylistItem(data, this);
            SmartPlaylists.Add(added);
            SaveSmart();
            added.Refresh();
            OpenSmart(added);
            ShowToast($"SMART_PLAYLIST_CREATED // {data.Name}", ToastKind.Success);
        }
        SmartEditor = null;
    }

    [RelayCommand]
    private void DeleteCurrentSmart()
    {
        if (CurrentHeader is not { Smart: { } s }) return;
        ConfirmDialog = new ConfirmDialogVm("WARNING: DELETE_SMART_PLAYLIST", s.Name, "THE TRACKS STAY IN THE LIBRARY.", "CONFIRM", () =>
        {
            var wasOpen = CurrentHeader?.Smart == s;
            SmartPlaylists.Remove(s);
            SaveSmart();
            if (wasOpen) CurrentView = AppView.Library;
            ShowToast($"SMART_PLAYLIST_DELETED // {s.Name}", ToastKind.Info);
        });
    }
}

/// <summary>Sidebar entry for a smart playlist.</summary>
public sealed partial class SmartPlaylistItem(SmartPlaylistData data, MainViewModel owner) : ObservableObject
{
    public SmartPlaylistData Data { get; } = data;
    public string Name => Data.Name;
    [ObservableProperty] private string _unitsText = "";
    [ObservableProperty] private string? _cover;
    public string? CoverFull { get; private set; }

    /// <summary>"FORMAT IS FLAC + PLAYS AT LEAST 5", for the page header.</summary>
    public string RulesText
    {
        get
        {
            var rules = Data.Rules.Where(r => !Retrograde.Core.SmartPlaylists.NeedsValue(r.Op) || r.Value.Trim().Length > 0).ToList();
            if (rules.Count == 0) return "EVERY TRACK";
            var parts = rules.Select(r =>
            {
                var info = Retrograde.Core.SmartPlaylists.Field(r.Field);
                var op = Retrograde.Core.SmartPlaylists.OpLabels.GetValueOrDefault(r.Op, r.Op);
                if (!Retrograde.Core.SmartPlaylists.NeedsValue(r.Op)) return $"{info.Label} {op}";
                var unit = r.Field switch { "played" or "added" => " DAYS", "duration" => " MIN", "rate" => " KHZ", "bits" => " BIT", _ => "" };
                return $"{info.Label} {op} {r.Value.Trim().ToUpper(CultureInfo.InvariantCulture)}{unit}";
            });
            return string.Join(Data.MatchAll ? " + " : " / ", parts);
        }
    }

    public void Refresh() => Refresh(owner.EvaluateSmart(Data));

    public void Refresh(IReadOnlyList<Track> tracks)
    {
        UnitsText = $"{tracks.Count} UNITS";
        Cover = tracks.FirstOrDefault(t => t.CoverThumbPath is not null)?.CoverThumbPath;
        CoverFull = tracks.FirstOrDefault(t => t.CoverFullPath is not null)?.CoverFullPath;
    }

    public void NotifyName() => OnPropertyChanged(nameof(Name));

    [RelayCommand] private void Open() => owner.OpenSmart(this);
}

/// <summary>A choice in one of the editor's drop-down menus.</summary>
public sealed partial class OptionVm(string label, bool selected, Action pick) : ObservableObject
{
    public string Label { get; } = label;
    public bool IsSelected { get; } = selected;
    [RelayCommand] private void Pick() => pick();
}

/// <summary>One rule row in the smart playlist editor.</summary>
public sealed partial class SmartRuleVm : ObservableObject
{
    private readonly SmartEditorVm _editor;

    public SmartRuleVm(SmartEditorVm editor, SmartRule rule)
    {
        _editor = editor;
        _field = Retrograde.Core.SmartPlaylists.Field(rule.Field).Key;
        var ops = Retrograde.Core.SmartPlaylists.Field(_field).Ops;
        _op = ops.Contains(rule.Op) ? rule.Op : ops[0];
        _value = rule.Value;
    }

    [ObservableProperty] [NotifyPropertyChangedFor(nameof(FieldLabel), nameof(Hint), nameof(FieldOptions), nameof(OpOptions))]
    private string _field;

    [ObservableProperty] [NotifyPropertyChangedFor(nameof(OpLabel), nameof(NeedsValue), nameof(OpOptions))]
    private string _op;

    [ObservableProperty] private string _value;

    public string FieldLabel => Retrograde.Core.SmartPlaylists.Field(Field).Label;
    public string OpLabel => Retrograde.Core.SmartPlaylists.OpLabels.GetValueOrDefault(Op, Op);
    public string Hint => Retrograde.Core.SmartPlaylists.Field(Field).Hint;
    public bool NeedsValue => Retrograde.Core.SmartPlaylists.NeedsValue(Op);

    public IReadOnlyList<OptionVm> FieldOptions => Retrograde.Core.SmartPlaylists.Fields
        .Select(f => new OptionVm(f.Label, f.Key == Field, () => SetField(f.Key))).ToList();

    public IReadOnlyList<OptionVm> OpOptions => Retrograde.Core.SmartPlaylists.Field(Field).Ops
        .Select(o => new OptionVm(Retrograde.Core.SmartPlaylists.OpLabels[o], o == Op, () => Op = o)).ToList();

    private void SetField(string key)
    {
        if (key == Field) return;
        Field = key;
        Op = Retrograde.Core.SmartPlaylists.Field(key).Ops[0];
        Value = "";
    }

    partial void OnFieldChanged(string value) => _editor.Changed();
    partial void OnOpChanged(string value) => _editor.Changed();
    partial void OnValueChanged(string value) => _editor.Changed();

    [RelayCommand] private void Remove() => _editor.RemoveRule(this);

    public SmartRule ToRule() => new() { Field = Field, Op = Op, Value = NeedsValue ? Value.Trim() : "" };
}

/// <summary>The NEW / EDIT smart playlist dialog, with a live count of the matching tracks.</summary>
public sealed partial class SmartEditorVm : ObservableObject
{
    private readonly MainViewModel _owner;
    private bool _previewPending;

    public SmartEditorVm(MainViewModel owner, SmartPlaylistItem? editing)
    {
        _owner = owner;
        Editing = editing;
        var d = editing?.Data;
        _name = d?.Name ?? "";
        _matchAll = d?.MatchAll ?? true;
        _sort = d?.Sort ?? "album";
        _limitText = d is { Limit: > 0 } ? d.Limit.ToString(CultureInfo.InvariantCulture) : "";
        var rules = d?.Rules is { Count: > 0 } r ? r : [new SmartRule { Field = "plays", Op = "gte", Value = "5" }];
        foreach (var rule in rules) Rules.Add(new SmartRuleVm(this, rule));
        UpdatePreview();
    }

    public SmartPlaylistItem? Editing { get; }
    public string Heading => Editing is null ? "NEW_SMART_PLAYLIST" : "EDIT_SMART_PLAYLIST";
    public string SaveText => Editing is null ? "CREATE" : "SAVE";
    public ObservableCollection<SmartRuleVm> Rules { get; } = [];

    [ObservableProperty] [NotifyPropertyChangedFor(nameof(CanSave))] private string _name;
    [ObservableProperty] [NotifyPropertyChangedFor(nameof(IsMatchAny))] private bool _matchAll;
    [ObservableProperty] [NotifyPropertyChangedFor(nameof(SortLabel), nameof(SortOptions))] private string _sort;
    [ObservableProperty] private string _limitText;
    [ObservableProperty] private string _previewText = "";

    public bool IsMatchAny => !MatchAll;
    public bool CanSave => Name.Trim().Length > 0;
    public string SortLabel => Retrograde.Core.SmartPlaylists.Sorts.FirstOrDefault(s => s.Key == Sort).Label ?? "ALBUM ORDER";
    public IReadOnlyList<OptionVm> SortOptions => Retrograde.Core.SmartPlaylists.Sorts
        .Select(s => new OptionVm(s.Label, s.Key == Sort, () => Sort = s.Key)).ToList();

    partial void OnMatchAllChanged(bool value) => Changed();
    partial void OnLimitTextChanged(string value) => Changed();

    [RelayCommand] private void SetMatchAll() => MatchAll = true;
    [RelayCommand] private void SetMatchAny() => MatchAll = false;

    [RelayCommand]
    private void AddRule()
    {
        Rules.Add(new SmartRuleVm(this, new SmartRule { Field = "artist", Op = "contains" }));
        Changed();
    }

    public void RemoveRule(SmartRuleVm rule)
    {
        Rules.Remove(rule);
        Changed();
    }

    [RelayCommand]
    private void Save()
    {
        if (!CanSave) return;
        _owner.CommitSmart(this);
    }

    /// <summary>Recounts after the edits of this moment (typing a value changes it per key).</summary>
    public void Changed()
    {
        if (_previewPending) return;
        _previewPending = true;
        Avalonia.Threading.Dispatcher.UIThread.Post(() =>
        {
            _previewPending = false;
            UpdatePreview();
        }, Avalonia.Threading.DispatcherPriority.Background);
    }

    private void UpdatePreview()
    {
        var n = _owner.EvaluateSmart(ToData()).Count;
        PreviewText = n == 1 ? "1 TRACK MATCHES" : $"{n} TRACKS MATCH";
    }

    public SmartPlaylistData ToData() => new()
    {
        Name = Name.Trim(),
        MatchAll = MatchAll,
        // A rule left without a value matches nothing to filter by: it is not kept
        Rules = Rules.Select(r => r.ToRule()).Where(r => !Retrograde.Core.SmartPlaylists.NeedsValue(r.Op) || r.Value.Length > 0).ToList(),
        Sort = Sort,
        Limit = int.TryParse(LimitText.Trim(), NumberStyles.Integer, CultureInfo.InvariantCulture, out var l) && l > 0 ? l : 0,
    };
}
