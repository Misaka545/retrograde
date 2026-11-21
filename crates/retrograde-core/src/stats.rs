use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::paths::CoverPaths;

static WRITE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct PlayStat {
    pub count: i32,
    pub last: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct PlayStats {
    pub tracks: HashMap<String, PlayStat>,
}

impl PlayStats {
    pub fn file_path() -> PathBuf {
        CoverPaths::app_dir().join("native-stats.json")
    }

    pub fn backup_path() -> PathBuf {
        CoverPaths::app_dir().join("native-stats.json.bak")
    }

    pub fn corrupt_path() -> PathBuf {
        CoverPaths::app_dir().join("native-stats.json.corrupt")
    }

    pub fn load() -> Self {
        let path = Self::file_path();
        if path.exists() {
            if let Ok(text) = fs::read_to_string(&path)
                && let Ok(stats) = serde_json::from_str::<Self>(&text)
            {
                return stats;
            }
            let _ = fs::copy(&path, Self::corrupt_path());
            let backup = Self::backup_path();
            if backup.exists()
                && let Ok(text) = fs::read_to_string(&backup)
                && let Ok(stats) = serde_json::from_str::<Self>(&text)
            {
                let _ = fs::copy(&backup, &path);
                return stats;
            }
        }
        Self::default()
    }

    pub fn get(&self, id: &str) -> Option<&PlayStat> {
        self.tracks.get(id).or_else(|| {
            self.tracks
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(id))
                .map(|(_, v)| v)
        })
    }

    pub fn record_now(&mut self, id: &str) {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        self.record_at(id, now_ms);
    }

    pub fn record_at(&mut self, id: &str, when_ms: i64) {
        let key = if self.tracks.contains_key(id) {
            id.to_owned()
        } else {
            self.tracks
                .keys()
                .find(|k| k.eq_ignore_ascii_case(id))
                .cloned()
                .unwrap_or_else(|| id.to_owned())
        };
        let entry = self.tracks.entry(key).or_default();
        entry.count += 1;
        entry.last = when_ms;
    }

    /// Seconds a track must be heard before it counts as played
    /// (C# `Math.Clamp(d * 0.5, Math.Min(d, 15), 240)`, 30 s when the length is unknown).
    pub fn threshold(duration_secs: f64) -> f64 {
        if duration_secs <= 0.0 {
            30.0
        } else {
            (duration_secs * 0.5).clamp(duration_secs.min(15.0), 240.0)
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
        if let Ok(json) = serde_json::to_string(self)
            && fs::write(&tmp, json).is_ok()
        {
            if path.exists() {
                let _ = fs::copy(&path, &backup);
            }
            let _ = fs::rename(&tmp, &path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_handles_short_and_long_durations() {
        assert_eq!(PlayStats::threshold(0.0), 30.0);
        assert_eq!(PlayStats::threshold(-1.0), 30.0);
        // Short tracks must be heard in full, up to 15 s (C# formula).
        assert_eq!(PlayStats::threshold(2.0), 2.0);
        assert_eq!(PlayStats::threshold(10.0), 10.0);
        assert_eq!(PlayStats::threshold(14.0), 14.0);
        assert_eq!(PlayStats::threshold(20.0), 15.0);
        assert_eq!(PlayStats::threshold(30.0), 15.0);
        assert_eq!(PlayStats::threshold(180.0), 90.0);
        assert_eq!(PlayStats::threshold(480.0), 240.0);
        assert_eq!(PlayStats::threshold(600.0), 240.0);
    }
}
