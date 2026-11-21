using System.Globalization;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;

namespace Retrograde.App.ViewModels;

/// <summary>
/// 10-band graphic equalizer (octave bands, ±12 dB) in mpv's filter chain: one labelled lavfi
/// equalizer per band, so moving a band is one af-command and playback never stutters. A preamp
/// lowers the level by the largest boost, so boosted bands do not clip.
/// </summary>
public partial class PlayerViewModel
{
    private static readonly int[] BandHz = [32, 64, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
    public const double MaxGain = 12;

    private static readonly (string Name, double[] Gains)[] Presets =
    [
        ("FLAT", [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
        ("BASS", [6, 5, 4, 2, 0, 0, 0, 0, 0, 0]),
        ("TREBLE", [0, 0, 0, 0, 0, 1, 2, 4, 5, 6]),
        ("VOCAL", [-2, -2, -1, 1, 3, 4, 3, 1, 0, -1]),
        ("V-SHAPE", [5, 4, 2, 0, -2, -2, 0, 2, 4, 5]),
        ("WARM", [3, 3, 2, 1, 0, -1, -1, -2, -2, -3]),
    ];

    public IReadOnlyList<EqBandItem> EqBands { get; private set; } = [];
    public IReadOnlyList<EqPresetItem> EqPresets { get; private set; } = [];

    [ObservableProperty] private bool _isEqEnabled;
    [ObservableProperty] private string _eqPresetName = "FLAT";

    private bool _applyingPreset;

    private void InitEqualizer()
    {
        var saved = _settings.EqGains;
        EqBands = BandHz.Select((hz, i) => new EqBandItem(i, hz, saved.Count == BandHz.Length ? Math.Clamp(saved[i], -MaxGain, MaxGain) : 0, this)).ToList();
        EqPresets = Presets.Select(p => new EqPresetItem(p.Name, this)).ToList();
#pragma warning disable MVVMTK0034 // backing fields: no save while constructing
        _isEqEnabled = _settings.EqEnabled;
        _eqPresetName = string.IsNullOrEmpty(_settings.EqPreset) ? "FLAT" : _settings.EqPreset;
#pragma warning restore MVVMTK0034
        foreach (var p in EqPresets) p.IsSelected = p.Name == EqPresetName;
    }

    [RelayCommand] private void ToggleEq() => IsEqEnabled = !IsEqEnabled;

    /// <summary>The sound popup shows its OUTPUT tab (device, ReplayGain) instead of the equalizer.</summary>
    [ObservableProperty] private bool _isOutputTab;
    [RelayCommand] private void ShowAudioTab(string tab) => IsOutputTab = tab == "output";

    public void ApplyPreset(string name)
    {
        var preset = Presets.FirstOrDefault(p => p.Name == name);
        if (preset.Gains is null) return;
        _applyingPreset = true;
        try
        {
            for (int i = 0; i < EqBands.Count; i++) EqBands[i].Gain = preset.Gains[i];
        }
        finally
        {
            _applyingPreset = false;
        }
        EqPresetName = name;
        if (!IsEqEnabled) IsEqEnabled = true; // picking a curve means wanting to hear it
        else SendEqGains();
        SaveEq();
    }

    partial void OnEqPresetNameChanged(string value)
    {
        foreach (var p in EqPresets) p.IsSelected = p.Name == value;
    }

    partial void OnIsEqEnabledChanged(bool value)
    {
        SaveEq();
        var filters = EqualizerFilters();
        foreach (var engine in new[] { _engine, _spare })
            if (engine is not null) _ = engine.SetAudioFiltersAsync(filters); // also kept for an engine not started yet
    }

    /// <summary>A band moved (drag, wheel, double-click reset).</summary>
    internal void OnBandChanged(EqBandItem band)
    {
        if (_applyingPreset) return;
        if (EqPresetName != "CUSTOM") EqPresetName = "CUSTOM";
        if (IsEqEnabled) SendEqGains();
        SaveEq();
    }

    private void SendEqGains()
    {
        if (_bitPerfect) return;
        var preamp = Num(-Math.Max(0, EqBands.Max(b => b.Gain)));
        foreach (var engine in StartedEngines())
        {
            _ = engine.FilterCommandAsync("eqpre", "volume", "volume", preamp + "dB");
            foreach (var b in EqBands) _ = engine.FilterCommandAsync("eq" + b.Index, "equalizer", "g", Num(b.Gain));
        }
    }

    /// <summary>The af chain for mpv: empty when the equalizer is off.</summary>
    private string EqualizerFilters()
    {
        if (!IsEqEnabled || _bitPerfect) return ""; // direct output bypasses the equalizer
        var parts = new List<string>
        {
            $"@eqpre:lavfi=[volume={Num(-Math.Max(0, EqBands.Max(b => b.Gain)))}dB]",
        };
        foreach (var b in EqBands)
            parts.Add($"@eq{b.Index}:lavfi=[equalizer=f={b.Hz}:t=o:w=1:g={Num(b.Gain)}]");
        return string.Join(",", parts);
    }

    private void SaveEq()
    {
        _settings.EqEnabled = IsEqEnabled;
        _settings.EqPreset = EqPresetName;
        _settings.EqGains = EqBands.Select(b => b.Gain).ToList();
        _settings.Save();
    }

    private static string Num(double value) => value.ToString("0.#", CultureInfo.InvariantCulture);
}

/// <summary>One equalizer band: its frequency label and gain in dB.</summary>
public sealed partial class EqBandItem(int index, int hz, double gain, PlayerViewModel player) : ObservableObject
{
    public int Index { get; } = index;
    public int Hz { get; } = hz;
    public string Label { get; } = hz >= 1000 ? $"{hz / 1000}K" : hz.ToString(CultureInfo.InvariantCulture);
    public string Tip => $"{Label}Hz  {(Gain > 0 ? "+" : "")}{Gain:0.#} dB";

    [ObservableProperty] [NotifyPropertyChangedFor(nameof(Tip))]
    private double _gain = gain;

    partial void OnGainChanged(double value) => player.OnBandChanged(this);
}

public sealed partial class EqPresetItem(string name, PlayerViewModel player) : ObservableObject
{
    public string Name { get; } = name;
    [ObservableProperty] private bool _isSelected;
    [RelayCommand] private void Select() => player.ApplyPreset(Name);
}
