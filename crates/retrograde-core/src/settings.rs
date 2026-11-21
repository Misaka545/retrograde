use std::fs;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::paths::CoverPaths;

static WRITE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GpuPreference {
    /// Lock to integrated GPU on hybrid laptops (keeps discrete GPU suspended, saves ~150 MB RAM).
    #[default]
    Igpu,
    /// Let the system / wgpu choose the adapter automatically.
    Auto,
    /// Prefer discrete/high-performance GPU when available.
    Dgpu,
    /// Force CPU software renderer (`tiny-skia`).
    Software,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DisplayBackend {
    #[default]
    Auto,
    Wayland,
    X11,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PerformanceMode {
    /// Automatically follow UPower PowerProfilesDaemon (or Balanced if unavailable).
    #[default]
    Auto,
    Performance,
    Balanced,
    PowerSaver,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DirectVolumeBridge {
    /// Expose a temporary PipeWire null-audio-sink while direct ALSA holds the DAC so system
    /// volume sliders (Noctalia, Waybar, GNOME/KDE, media keys) control the DAC's hardware mixer 2-way.
    #[default]
    VirtualSink,
    /// Only adjust ALSA hardware mixer via Retrograde's own knob, MPRIS, and DAC buttons.
    HardwareOnly,
}

/// Persistent settings stored in `~/.retrograde/native-settings.json` (PascalCase for C# parity).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct AppSettings {
    pub volume: f64,
    pub audio_device: String,
    pub library_sort: String,
    pub replay_gain: String,
    pub library_folders: Vec<String>,
    pub minimize_to_tray: Option<bool>,
    pub sidebar_visible: bool,
    pub crossfade_seconds: i32,
    pub eq_enabled: bool,
    pub eq_preset: String,
    pub eq_gains: Vec<f64>,
    pub watch_folders: bool,
    pub accent_from_cover: bool,
    pub mini_x: Option<i32>,
    pub mini_y: Option<i32>,
    pub library_view: String,

    // Cross-distro / Linux integration options (with safe defaults for all distros)
    pub gpu_preference: GpuPreference,
    pub display_backend: DisplayBackend,
    pub performance_mode: PerformanceMode,
    pub direct_volume_bridge: DirectVolumeBridge,
    pub enable_tray: bool,
    pub enable_mpris: bool,

    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            volume: 0.5,
            audio_device: "auto".to_owned(),
            library_sort: "added_desc".to_owned(),
            replay_gain: "no".to_owned(),
            library_folders: Vec::new(),
            minimize_to_tray: None,
            sidebar_visible: true,
            crossfade_seconds: 0,
            eq_enabled: false,
            eq_preset: "FLAT".to_owned(),
            eq_gains: Vec::new(),
            watch_folders: true,
            accent_from_cover: true,
            mini_x: None,
            mini_y: None,
            library_view: "albums".to_owned(),
            gpu_preference: GpuPreference::Igpu,
            display_backend: DisplayBackend::Auto,
            performance_mode: PerformanceMode::Auto,
            direct_volume_bridge: DirectVolumeBridge::VirtualSink,
            enable_tray: true,
            enable_mpris: true,
            extra: Map::new(),
        }
    }
}

/// Every `LibrarySort` key the C# build writes (`MainViewModel.cs`), in menu order.
/// `library_sort` itself stays an opaque string so any of these round-trips unchanged.
pub const LIBRARY_SORT_KEYS: [&str; 10] = [
    "added_desc",
    "added_asc",
    "name_asc",
    "name_desc",
    "artist_asc",
    "artist_desc",
    "year_desc",
    "year_asc",
    "played_desc",
    "plays_desc",
];

/// Maps a stored sort key to one of [`LIBRARY_SORT_KEYS`]: the old Rust `title_asc` reads as
/// `name_asc`, anything unknown as the default `added_desc`.
pub fn canonical_library_sort(key: &str) -> &'static str {
    if key == "title_asc" {
        return "name_asc";
    }
    LIBRARY_SORT_KEYS
        .iter()
        .copied()
        .find(|k| *k == key)
        .unwrap_or("added_desc")
}

impl AppSettings {
    /// Parses settings JSON. A field whose value does not fit (an unknown enum value written by
    /// another version, a wrong type) falls back to its default instead of failing the whole file.
    /// Returns `None` only when the text is not a JSON object.
    pub fn parse_lenient(text: &str) -> Option<Self> {
        if let Ok(parsed) = serde_json::from_str::<Self>(text) {
            return Some(parsed);
        }
        let Ok(Value::Object(input)) = serde_json::from_str::<Value>(text) else {
            return None;
        };
        let Ok(Value::Object(mut merged)) = serde_json::to_value(Self::default()) else {
            return None;
        };
        for (key, val) in input {
            let previous = merged.insert(key.clone(), val);
            if serde_json::from_value::<Self>(Value::Object(merged.clone())).is_err() {
                match previous {
                    Some(prev) => merged.insert(key, prev),
                    None => merged.remove(&key),
                };
            }
        }
        serde_json::from_value(Value::Object(merged)).ok()
    }

    pub fn file_path() -> PathBuf {
        CoverPaths::app_dir().join("native-settings.json")
    }

    pub fn backup_path() -> PathBuf {
        CoverPaths::app_dir().join("native-settings.json.bak")
    }

    pub fn corrupt_path() -> PathBuf {
        CoverPaths::app_dir().join("native-settings.json.corrupt")
    }

    pub fn load() -> Self {
        let path = Self::file_path();
        if path.exists() {
            if let Ok(text) = fs::read_to_string(&path)
                && let Some(parsed) = Self::parse_lenient(&text)
            {
                return parsed;
            }
            let _ = fs::copy(&path, Self::corrupt_path());
            let backup = Self::backup_path();
            if backup.exists()
                && let Ok(text) = fs::read_to_string(&backup)
                && let Some(parsed) = Self::parse_lenient(&text)
            {
                let _ = fs::copy(&backup, &path);
                return parsed;
            }
        }
        Self {
            minimize_to_tray: electron_minimize_to_tray(),
            ..Self::default()
        }
    }

    pub fn save(&self) {
        let Ok(_guard) = WRITE_LOCK.lock() else {
            return;
        };
        let dir = CoverPaths::app_dir();
        if fs::create_dir_all(&dir).is_err() {
            return;
        }
        let path = Self::file_path();
        let backup = Self::backup_path();
        let tmp = path.with_extension("json.tmp");
        if let Ok(json) = serde_json::to_string_pretty(self)
            && fs::write(&tmp, json).is_ok()
        {
            if path.exists() {
                let _ = fs::copy(&path, &backup);
            }
            let _ = fs::rename(&tmp, &path);
        }
    }
}

fn electron_minimize_to_tray() -> Option<bool> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    let path = base.join("retrograde").join("retrograde-config.json");
    let text = fs::read_to_string(path).ok()?;
    let val: Value = serde_json::from_str(&text).ok()?;
    val.get("minimizeToTray").and_then(|v| v.as_bool())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_legacy_csharp_settings_with_new_defaults() {
        let json = r#"{
            "Volume": 0.6,
            "AudioDevice": "alsa/hw:CARD=Pro,DEV=0",
            "LibrarySort": "added_desc",
            "ReplayGain": "track",
            "LibraryFolders": ["/home/okarin/Music"],
            "MinimizeToTray": false,
            "SidebarVisible": true,
            "CrossfadeSeconds": 0,
            "EqEnabled": false,
            "EqPreset": "CUSTOM",
            "EqGains": [0, 0, 0, 0, 0, -3, 0, 0, 0, 0],
            "WatchFolders": true,
            "AccentFromCover": true,
            "MiniX": 760,
            "MiniY": 496,
            "LibraryView": "albums"
        }"#;
        let s: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.volume, 0.6);
        assert_eq!(s.audio_device, "alsa/hw:CARD=Pro,DEV=0");
        assert_eq!(s.gpu_preference, GpuPreference::Igpu);
        assert_eq!(s.display_backend, DisplayBackend::Auto);
        assert_eq!(s.performance_mode, PerformanceMode::Auto);
        assert_eq!(s.direct_volume_bridge, DirectVolumeBridge::VirtualSink);
        assert!(s.enable_tray);
        assert!(s.enable_mpris);
    }

    #[test]
    fn one_bad_field_falls_back_without_resetting_the_rest() {
        let json = r#"{
            "Volume": 0.3,
            "GpuPreference": "quantum",
            "PerformanceMode": 7,
            "CrossfadeSeconds": "six",
            "LibraryFolders": ["/music"],
            "LibrarySort": "played_desc",
            "SomethingNew": {"a": 1}
        }"#;
        let s = AppSettings::parse_lenient(json).expect("lenient parse");
        assert_eq!(s.volume, 0.3);
        assert_eq!(s.gpu_preference, GpuPreference::Igpu);
        assert_eq!(s.performance_mode, PerformanceMode::Auto);
        assert_eq!(s.crossfade_seconds, 0);
        assert_eq!(s.library_folders, vec!["/music".to_owned()]);
        assert_eq!(s.library_sort, "played_desc");
        assert!(s.extra.contains_key("SomethingNew"));
        assert!(AppSettings::parse_lenient("not json").is_none());
        assert!(AppSettings::parse_lenient("[1, 2]").is_none());
    }

    #[test]
    fn csharp_sort_keys_roundtrip_unchanged() {
        // Fixture shaped like a native-settings.json written by the C# build.
        let template = r#"{
  "Volume": 0.5,
  "AudioDevice": "auto",
  "LibrarySort": "KEY",
  "ReplayGain": "no",
  "LibraryFolders": [],
  "MinimizeToTray": null,
  "SidebarVisible": true,
  "CrossfadeSeconds": 0,
  "EqEnabled": false,
  "EqPreset": "FLAT",
  "EqGains": [],
  "WatchFolders": true,
  "AccentFromCover": true,
  "MiniX": null,
  "MiniY": null,
  "LibraryView": "albums"
}"#;
        for key in LIBRARY_SORT_KEYS.iter().copied().chain(["title_asc"]) {
            let s = AppSettings::parse_lenient(&template.replace("KEY", key)).unwrap();
            assert_eq!(s.library_sort, key);
            let back = AppSettings::parse_lenient(&serde_json::to_string(&s).unwrap()).unwrap();
            assert_eq!(back.library_sort, key);
            let expected = if key == "title_asc" { "name_asc" } else { key };
            assert_eq!(canonical_library_sort(key), expected);
        }
        assert_eq!(canonical_library_sort("bogus"), "added_desc");
    }

    #[test]
    fn roundtrips_custom_linux_options() {
        let s = AppSettings {
            gpu_preference: GpuPreference::Dgpu,
            display_backend: DisplayBackend::Wayland,
            performance_mode: PerformanceMode::PowerSaver,
            direct_volume_bridge: DirectVolumeBridge::HardwareOnly,
            enable_tray: false,
            ..Default::default()
        };

        let raw = serde_json::to_string_pretty(&s).unwrap();
        let back: AppSettings = serde_json::from_str(&raw).unwrap();
        assert_eq!(back.gpu_preference, GpuPreference::Dgpu);
        assert_eq!(back.display_backend, DisplayBackend::Wayland);
        assert_eq!(back.performance_mode, PerformanceMode::PowerSaver);
        assert_eq!(back.direct_volume_bridge, DirectVolumeBridge::HardwareOnly);
        assert!(!back.enable_tray);
    }
}
