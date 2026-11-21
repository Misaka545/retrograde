use std::fs;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};

use crate::paths::CoverPaths;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct SessionState {
    pub current_id: Option<String>,
    pub index: i32,
    pub position: f64,
    pub shuffle: bool,
    pub repeat_mode: i32,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            current_id: None,
            index: -1,
            position: 0.0,
            shuffle: false,
            repeat_mode: 0,
        }
    }
}

static GATE: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

pub struct SessionStore;

impl SessionStore {
    pub fn state_path() -> PathBuf {
        CoverPaths::app_dir().join("native-session.json")
    }

    pub fn queue_path() -> PathBuf {
        CoverPaths::app_dir().join("native-queue.json")
    }

    pub fn original_queue_path() -> PathBuf {
        CoverPaths::app_dir().join("native-queue-original.json")
    }

    pub fn load() -> Option<(SessionState, Vec<String>)> {
        let state_text = fs::read_to_string(Self::state_path()).ok()?;
        let queue_text = fs::read_to_string(Self::queue_path()).ok()?;
        let state = serde_json::from_str::<SessionState>(&state_text).ok()?;
        let queue = serde_json::from_str::<Vec<String>>(&queue_text).ok()?;
        Some((state, queue))
    }

    pub fn load_original_queue() -> Option<Vec<String>> {
        let text = fs::read_to_string(Self::original_queue_path()).ok()?;
        serde_json::from_str::<Vec<String>>(&text).ok()
    }

    pub fn save_state(state: &SessionState) {
        if let Ok(json) = serde_json::to_string(state) {
            write_atomic(&Self::state_path(), &json);
        }
    }

    pub fn save_queue(ids: &[String]) {
        if let Ok(json) = serde_json::to_string(ids) {
            write_atomic(&Self::queue_path(), &json);
        }
    }

    pub fn save_original_queue(ids: &[String]) {
        if let Ok(json) = serde_json::to_string(ids) {
            write_atomic(&Self::original_queue_path(), &json);
        }
    }
}

fn write_atomic(path: &std::path::Path, json: &str) {
    let Ok(_guard) = GATE.lock() else {
        return;
    };
    let dir = CoverPaths::app_dir();
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, json).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}
