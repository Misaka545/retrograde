use std::fs;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::paths::CoverPaths;

pub type ExtraMap = Map<String, Value>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlaylistData {
    pub id: i64,
    pub name: String,
    pub cover_art: Option<String>,
    pub track_ids: Vec<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SmartRule {
    pub field: String,
    pub op: String,
    #[serde(deserialize_with = "de_string_lenient")]
    pub value: String,
}

impl Default for SmartRule {
    fn default() -> Self {
        Self {
            field: "plays".to_owned(),
            op: "gte".to_owned(),
            value: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SmartPlaylistData {
    pub id: i64,
    pub name: String,
    pub match_all: bool,
    pub rules: Vec<SmartRule>,
    pub sort: String,
    #[serde(deserialize_with = "de_usize_lenient")]
    pub limit: usize,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for SmartPlaylistData {
    fn default() -> Self {
        Self {
            id: 0,
            name: String::new(),
            match_all: true,
            rules: Vec::new(),
            sort: "album".to_owned(),
            limit: 0,
            extra: Map::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CollectionsData {
    pub liked: Vec<String>,
    pub playlists: Vec<PlaylistData>,
    pub smart: Option<Vec<SmartPlaylistData>>,
    pub imported_from: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

static RESTORED_FROM_BACKUP: LazyLock<Mutex<bool>> = LazyLock::new(|| Mutex::new(false));
static WRITE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

pub struct CollectionsStore;

impl CollectionsStore {
    pub fn file_path() -> PathBuf {
        CoverPaths::app_dir().join("native-collections.json")
    }

    pub fn backup_path() -> PathBuf {
        CoverPaths::app_dir().join("native-collections.json.bak")
    }

    pub fn restored_from_backup() -> bool {
        RESTORED_FROM_BACKUP.lock().map(|g| *g).unwrap_or(false)
    }

    pub fn load() -> CollectionsData {
        Self::load_existing().unwrap_or_default()
    }

    /// Like `load`, but when there is nothing readable (first run, or a damaged file without a
    /// usable `.bak`) seeds liked songs and playlists once from the Electron build's
    /// localStorage and saves them, as the C# `CollectionsStore.Load` does.
    pub fn load_or_import() -> CollectionsData {
        if let Some(data) = Self::load_existing() {
            return data;
        }
        match crate::electron_import::try_read() {
            Some(imported) => {
                Self::save(&imported);
                imported
            }
            None => CollectionsData::default(),
        }
    }

    /// The saved file, else its `.bak` when the file is unreadable; `None` when neither reads.
    fn load_existing() -> Option<CollectionsData> {
        let path = Self::file_path();
        if path.exists() {
            if let Some(data) = Self::try_read(&path) {
                return Some(data);
            }
            let corrupt = CoverPaths::app_dir().join("native-collections.json.corrupt");
            let _ = fs::copy(&path, corrupt);
            let backup = Self::backup_path();
            if let Some(prev) = Self::try_read(&backup) {
                if let Ok(mut flag) = RESTORED_FROM_BACKUP.lock() {
                    *flag = true;
                }
                return Some(prev);
            }
        }
        None
    }

    fn try_read(path: &std::path::Path) -> Option<CollectionsData> {
        let text = fs::read_to_string(path).ok()?;
        serde_json::from_str::<CollectionsData>(&text).ok()
    }

    pub fn save(data: &CollectionsData) {
        let Ok(_guard) = WRITE_LOCK.lock() else {
            return;
        };
        let dir = CoverPaths::app_dir();
        if fs::create_dir_all(&dir).is_err() {
            return;
        }
        let path = Self::file_path();
        let backup = Self::backup_path();
        let tmp = CoverPaths::app_dir().join("native-collections.json.tmp");
        let Ok(json) = serde_json::to_string_pretty(data) else {
            return;
        };
        if fs::write(&tmp, json).is_err() {
            return;
        }
        if path.exists() {
            let _ = fs::copy(&path, &backup);
        }
        let _ = fs::rename(&tmp, &path);
    }
}

fn de_string_lenient<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let v = Option::<Value>::deserialize(d)?;
    Ok(match v {
        Some(Value::String(s)) => s,
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    })
}

fn de_usize_lenient<'de, D: Deserializer<'de>>(d: D) -> Result<usize, D::Error> {
    let v = Option::<Value>::deserialize(d)?;
    Ok(match v {
        Some(Value::Number(n)) => n
            .as_u64()
            .map(|x| x as usize)
            .or_else(|| n.as_i64().filter(|&x| x > 0).map(|x| x as usize))
            .unwrap_or(0),
        Some(Value::String(s)) => s.trim().parse::<isize>().ok().filter(|&x| x > 0).unwrap_or(0) as usize,
        _ => 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lenient_smart_rule_value_and_negative_limit() {
        let json = r#"{
            "smart": [{
                "id": 1,
                "name": "Test",
                "limit": -1,
                "rules": [{"field": "year", "op": "gte", "value": 2020}]
            }]
        }"#;
        let data: CollectionsData = serde_json::from_str(json).unwrap();
        let sp = &data.smart.unwrap()[0];
        assert_eq!(sp.limit, 0);
        assert_eq!(sp.rules[0].value, "2020");
    }
}
