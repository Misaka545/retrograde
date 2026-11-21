use std::collections::HashMap;

use indexmap::IndexMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::models::{Album, Track};
use crate::paths::CoverPaths;

#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("I/O error accessing library.json: {0}")]
    Io(#[from] std::io::Error),
    #[error(
        "library.json is damaged and there is no usable backup. It was copied to library.json.corrupt; remove library.json to start a new library."
    )]
    Corrupt,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AlbumDto {
    pub name: Option<String>,
    pub artist: Option<String>,
    pub cover_art: Option<String>,
    pub cover_art_full: Option<String>,
    #[serde(deserialize_with = "de_i32_lenient")]
    pub year: i32,
    #[serde(deserialize_with = "de_i64_lenient")]
    pub added_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album_artist: Option<String>,
    pub tracks: Option<Vec<TrackDto>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TrackDto {
    pub id: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    #[serde(deserialize_with = "de_f64_lenient")]
    pub duration: f64,
    pub file_path: Option<String>,
    pub src: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "de_opt_i32_lenient")]
    pub track_number: Option<i32>,
    #[serde(deserialize_with = "de_i32_lenient")]
    pub disc_number: i32,
    pub codec: Option<String>,
    #[serde(deserialize_with = "de_i32_lenient")]
    pub sample_rate: i32,
    #[serde(deserialize_with = "de_i32_lenient")]
    pub bits_per_sample: i32,
    #[serde(deserialize_with = "de_i32_lenient")]
    pub channels: i32,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "de_opt_i64_lenient")]
    pub mtime: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "de_opt_i64_lenient")]
    pub size: Option<i64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Keeps the order of the keys in library.json, like the C# dictionary (ties in "Recently Added" depend on it).
pub type LibraryMap = IndexMap<String, AlbumDto>;

static WRITE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static RESTORED_FROM_BACKUP: LazyLock<Mutex<bool>> = LazyLock::new(|| Mutex::new(false));

pub struct LibraryStore;

impl LibraryStore {
    pub fn library_path() -> PathBuf {
        CoverPaths::app_dir().join("library.json")
    }

    pub fn backup_path() -> PathBuf {
        CoverPaths::app_dir().join("library.json.bak")
    }

    pub fn damaged_path() -> PathBuf {
        CoverPaths::app_dir().join("library.json.corrupt")
    }

    pub fn restored_from_backup() -> bool {
        RESTORED_FROM_BACKUP.lock().map(|g| *g).unwrap_or(false)
    }

    pub fn write_lock() -> &'static Mutex<()> {
        &WRITE_LOCK
    }

    pub fn load() -> Result<Vec<Album>, LibraryError> {
        let raw = Self::load_raw()?;
        Ok(raw.into_values().map(map_album).collect())
    }

    pub fn load_raw() -> Result<LibraryMap, LibraryError> {
        let path = Self::library_path();
        if !path.exists() {
            return Ok(LibraryMap::new());
        }
        if let Ok(bytes) = fs::read(&path)
            && let Ok(parsed) = serde_json::from_slice::<LibraryMap>(&bytes)
        {
            return Ok(parsed);
        }

        let _ = fs::copy(&path, Self::damaged_path());
        let backup = Self::backup_path();
        if backup.exists()
            && let Ok(bytes) = fs::read(&backup)
            && let Ok(parsed) = serde_json::from_slice::<LibraryMap>(&bytes)
        {
            let _ = fs::copy(&backup, &path);
            if let Ok(mut flag) = RESTORED_FROM_BACKUP.lock() {
                *flag = true;
            }
            return Ok(parsed);
        }
        Err(LibraryError::Corrupt)
    }

    pub fn save_raw(library: &LibraryMap) -> Result<(), LibraryError> {
        let dir = CoverPaths::app_dir();
        fs::create_dir_all(&dir)?;
        let path = Self::library_path();
        let backup = Self::backup_path();
        let tmp = CoverPaths::app_dir().join("library.json.tmp");
        let bytes = serde_json::to_vec(library).map_err(std::io::Error::other)?;
        fs::write(&tmp, bytes)?;
        if path.exists() {
            let _ = fs::copy(&path, &backup);
        }
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn delete_albums(names: &[String]) -> Result<usize, LibraryError> {
        let _guard = WRITE_LOCK.lock().expect("library lock poisoned");
        let mut library = Self::load_raw()?;
        let mut removed = 0;
        for name in names {
            if library.shift_remove(name).is_some() {
                removed += 1;
                try_delete(&CoverPaths::thumb_path(name));
                try_delete(&CoverPaths::full_path(name));
            }
        }
        if removed > 0 {
            Self::save_raw(&library)?;
        }
        Ok(removed)
    }

    pub fn clear_covers(names: &[String]) -> Result<(), LibraryError> {
        let _guard = WRITE_LOCK.lock().expect("library lock poisoned");
        let mut library = Self::load_raw()?;
        let mut changed = false;
        for name in names {
            if let Some(album) = library.get_mut(name) {
                album.cover_art = None;
                album.cover_art_full = None;
                try_delete(&CoverPaths::thumb_path(name));
                try_delete(&CoverPaths::full_path(name));
                changed = true;
            }
        }
        if changed {
            Self::save_raw(&library)?;
        }
        Ok(())
    }
}

pub(crate) fn try_delete(path: &Path) {
    if path.exists() {
        let _ = fs::remove_file(path);
    }
}

pub fn map_album(a: AlbumDto) -> Album {
    let name = a.name.clone().unwrap_or_else(|| "Unknown Album".to_owned());
    let album_artist = a
        .artist
        .clone()
        .unwrap_or_else(|| "Unknown Artist".to_owned());
    let thumb = CoverPaths::to_local_path(a.cover_art.as_deref());
    let full = CoverPaths::to_local_path(a.cover_art_full.as_deref()).or_else(|| thumb.clone());

    let tracks = a
        .tracks
        .unwrap_or_default()
        .into_iter()
        .filter_map(|t| {
            let file_path = t.file_path?;
            if file_path.is_empty() {
                return None;
            }
            let title = match t.title {
                Some(s) if !s.is_empty() => s,
                _ => Path::new(&file_path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Unknown")
                    .to_owned(),
            };
            let id = t.id.unwrap_or_else(|| file_path.clone());
            Some(Track {
                id,
                title,
                artist: t.artist.unwrap_or_else(|| album_artist.clone()),
                album: name.clone(),
                duration: t.duration,
                file_path,
                track_number: t.track_number.unwrap_or(0),
                disc_number: if t.disc_number <= 0 { 1 } else { t.disc_number },
                codec: t.codec.unwrap_or_default(),
                sample_rate: t.sample_rate,
                bits_per_sample: t.bits_per_sample,
                channels: t.channels,
                cover_thumb_path: thumb.clone(),
                cover_full_path: full.clone(),
            })
        })
        .collect();

    Album {
        name,
        artist: album_artist,
        cover_thumb_path: thumb,
        cover_full_path: full,
        year: a.year,
        added_at: a.added_at,
        tracks,
    }
}

pub fn build_track_index(albums: &[Album]) -> HashMap<String, (Track, Album)> {
    let mut map = HashMap::new();
    for album in albums {
        for track in &album.tracks {
            map.insert(track.id.clone(), (track.clone(), album.clone()));
        }
    }
    map
}

fn de_i32_lenient<'de, D: Deserializer<'de>>(d: D) -> Result<i32, D::Error> {
    Ok(de_opt_i32_lenient(d)?.unwrap_or(0))
}

fn de_opt_i32_lenient<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i32>, D::Error> {
    let v = Option::<Value>::deserialize(d)?;
    Ok(match v {
        Some(Value::Number(n)) => n.as_i64().map(|x| x as i32).or_else(|| n.as_f64().map(|x| x as i32)),
        Some(Value::String(s)) => s.trim().parse::<i32>().ok(),
        _ => None,
    })
}

fn de_i64_lenient<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    Ok(de_opt_i64_lenient(d)?.unwrap_or(0))
}

fn de_opt_i64_lenient<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    let v = Option::<Value>::deserialize(d)?;
    Ok(match v {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|x| x as i64)),
        Some(Value::String(s)) => s.trim().parse::<i64>().ok(),
        _ => None,
    })
}

fn de_f64_lenient<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let v = Option::<Value>::deserialize(d)?;
    Ok(match v {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => s.trim().parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    })
}
