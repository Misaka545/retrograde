use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use lofty::config::ParseOptions;
use lofty::file::{AudioFile, FileType, TaggedFileExt};
use lofty::picture::PictureType;
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey};
use rayon::prelude::*;
use regex::Regex;

use crate::covers::{ImageLimits, MAX_COVER_FILE_BYTES, PlaylistCovers, resize_to_jpeg};
use crate::library::{AlbumDto, LibraryError, LibraryMap, LibraryStore, TrackDto, try_delete};
use crate::paths::CoverPaths;

pub const AUDIO_EXTENSIONS: &[&str] = &[
    "flac", "mp3", "wav", "aac", "ogg", "m4a", "wma", "aiff", "alac", "opus",
];

const FOLDER_COVER_NAMES: &[&str] = &["cover", "folder", "front", "album", "albumart"];

static DISC_FOLDER_RX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(cd|disc|disk|dvd|vol|volume)[\s._-]*\d+").unwrap());
static ARTIST_SEP_RX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\s*(?:/|&|,|;|、|，|＆|\s+feat\.?\s+|\s+ft\.?\s+|\s+x\s+|\s+with\s+)\s*")
        .unwrap()
});
static SPLIT_ALBUM_RX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?P<base>.+) \((?P<who>[^()]+)\)(?: \d+)?$").unwrap());

#[derive(Debug, Clone)]
pub struct ScanProgress {
    pub phase: &'static str,
    pub scanned: usize,
    pub total: usize,
    pub current_file: String,
}

#[derive(Debug, Clone, Default)]
pub struct ScanResult {
    pub added_tracks: usize,
    pub new_albums: usize,
    pub covers: usize,
    pub updated_tracks: usize,
    pub removed_tracks: usize,
    pub missing_tracks: usize,
    pub merged_albums: usize,
}

impl ScanResult {
    pub fn changed(&self) -> bool {
        self.added_tracks > 0
            || self.covers > 0
            || self.updated_tracks > 0
            || self.removed_tracks > 0
            || self.merged_albums > 0
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ScanOptions {
    pub prune_missing: bool,
    pub max_threads: Option<usize>,
    pub force: bool,
}

#[derive(Default)]
struct Discovery {
    files: Vec<PathBuf>,
    walked_dirs: Vec<PathBuf>,
    unreadable_dirs: Vec<PathBuf>,
    gone_dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
struct Parsed {
    file_path: String,
    title: String,
    artist: String,
    album_artist: Option<String>,
    album: String,
    duration: f64,
    track_number: i32,
    disc_number: i32,
    year: i32,
    codec: String,
    sample_rate: i32,
    bits_per_sample: i32,
    channels: i32,
    has_picture: bool,
}

#[derive(Default)]
struct AlbumFolders {
    own: HashSet<String>,
    parents: HashSet<String>,
}

impl AlbumFolders {
    fn from_album(album: &AlbumDto) -> Self {
        let mut f = Self::default();
        if let Some(tracks) = &album.tracks {
            for t in tracks {
                if let Some(fp) = &t.file_path {
                    f.add(fp);
                }
            }
        }
        f
    }

    fn add(&mut self, file_path: &str) {
        let folder = album_folder(file_path);
        let key = folder.to_lowercase();
        if self.own.insert(key)
            && let Some(parent) = Path::new(&folder).parent()
        {
            let p = parent.to_string_lossy().to_lowercase();
            if !p.is_empty() {
                self.parents.insert(p);
            }
        }
    }

    fn is_same_folder(&self, file_path: &str) -> bool {
        self.own.contains(&album_folder(file_path).to_lowercase())
    }

    fn is_near(&self, file_path: &str) -> bool {
        let folder = album_folder(file_path);
        let key = folder.to_lowercase();
        if self.own.contains(&key) || self.parents.contains(&key) {
            return true;
        }
        if let Some(parent) = Path::new(&folder).parent() {
            let pk = parent.to_string_lossy().to_lowercase();
            return !pk.is_empty() && (self.parents.contains(&pk) || self.own.contains(&pk));
        }
        false
    }
}

pub struct Scanner;

impl Scanner {
    pub fn is_audio_file(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| AUDIO_EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
    }

    pub fn discover(paths: &[PathBuf]) -> Vec<PathBuf> {
        walk_paths(paths).files
    }

    pub fn scan(
        input_paths: &[PathBuf],
        options: ScanOptions,
        cancel: Option<&AtomicBool>,
        mut progress: Option<&mut dyn FnMut(ScanProgress)>,
    ) -> Result<ScanResult, LibraryError> {
        if let Some(cb) = progress.as_deref_mut() {
            cb(ScanProgress {
                phase: "discovering",
                scanned: 0,
                total: 0,
                current_file: "Discovering files...".to_owned(),
            });
        }

        let found = walk_paths(input_paths);
        let _guard = LibraryStore::write_lock()
            .lock()
            .expect("library write lock poisoned");
        scan_locked(found, options, cancel, progress)
    }
}

fn dir_identity(path: &Path) -> Option<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(path).ok().map(|m| (m.dev(), m.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

fn walk_paths(paths: &[PathBuf]) -> Discovery {
    let mut found = Discovery::default();
    let mut stack = Vec::new();
    let mut visited_dirs: HashSet<(u64, u64)> = HashSet::new();

    for p in paths {
        if p.is_dir() {
            let dir = fs::canonicalize(p).unwrap_or_else(|_| p.clone());
            if dir_identity(&dir).is_none_or(|id| visited_dirs.insert(id)) {
                stack.push(dir.clone());
                found.walked_dirs.push(dir);
            }
        } else if p.is_file() {
            if Scanner::is_audio_file(p) {
                found
                    .files
                    .push(fs::canonicalize(p).unwrap_or_else(|_| p.clone()));
            }
        } else if p.is_absolute() {
            // An absent root (moved folder, unmounted drive) counts its tracks as missing. They are
            // only reported; dropping them needs `prune_missing`, which the user has to confirm.
            found.gone_dirs.push(p.clone());
        }
    }

    while let Some(dir) = stack.pop() {
        match fs::read_dir(&dir) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let Ok(ft) = entry.file_type() else {
                        continue;
                    };
                    if ft.is_symlink() {
                        continue;
                    }
                    if ft.is_dir() {
                        if dir_identity(&path).is_none_or(|id| visited_dirs.insert(id)) {
                            stack.push(path);
                        }
                    } else if ft.is_file() && Scanner::is_audio_file(&path) {
                        found.files.push(path);
                    }
                }
            }
            Err(_) => {
                found.unreadable_dirs.push(dir);
            }
        }
    }
    found
}

fn scan_locked(
    found: Discovery,
    options: ScanOptions,
    cancel: Option<&AtomicBool>,
    mut progress: Option<&mut dyn FnMut(ScanProgress)>,
) -> Result<ScanResult, LibraryError> {
    let mut library = LibraryStore::load_raw()?;
    let mut dirty = false;
    let mut touched: HashSet<String> = HashSet::new();
    let mut mixed: HashSet<String> = HashSet::new();

    let merged = merge_split_albums(&mut library, &mut touched, &mut mixed);

    // Map lowercase filePath -> (album_key, track_index)
    let mut by_path: HashMap<String, (String, usize)> = HashMap::new();
    for (key, album) in &library {
        if let Some(tracks) = &album.tracks {
            for (idx, t) in tracks.iter().enumerate() {
                if let Some(fp) = &t.file_path {
                    by_path.entry(fp.to_lowercase()).or_insert((key.clone(), idx));
                }
            }
        }
    }

    let mut to_parse: Vec<PathBuf> = Vec::new();
    let mut changed: HashSet<String> = HashSet::new();
    let mut stamps: HashMap<String, (i64, i64)> = HashMap::new();
    let mut in_scan: HashSet<String> = HashSet::new();
    let mut seen_lower: HashSet<String> = HashSet::new();

    for f in &found.files {
        let f_str = f.to_string_lossy().into_owned();
        let f_low = f_str.to_lowercase();
        seen_lower.insert(f_low.clone());
        let Some(stamp) = file_stamp(f) else {
            continue;
        };
        stamps.insert(f_low.clone(), stamp);

        if let Some((album_key, track_idx)) = by_path.get(&f_low) {
            in_scan.insert(album_key.clone());
            if let Some(album) = library.get_mut(album_key)
                && let Some(tracks) = &mut album.tracks
                && let Some(track) = tracks.get_mut(*track_idx)
            {
                if track.mtime.is_none() || track.size.is_none() {
                    track.mtime = Some(stamp.0);
                    track.size = Some(stamp.1);
                    dirty = true;
                } else if options.force || track.mtime != Some(stamp.0) || track.size != Some(stamp.1) {
                    to_parse.push(f.clone());
                    changed.insert(f_low);
                }
            }
        } else {
            to_parse.push(f.clone());
        }
    }

    let gone = collect_missing(&library, &by_path, &found, &seen_lower);
    let missing = gone.len();
    let removed = if options.prune_missing {
        prune_tracks(&mut library, &gone, &mut touched)
    } else {
        0
    };

    let mut parsed_results: Vec<Parsed> = Vec::new();
    if !to_parse.is_empty() {
        let total = to_parse.len();
        if let Some(cb) = progress.as_deref_mut() {
            cb(ScanProgress {
                phase: "scanning",
                scanned: 0,
                total,
                current_file: String::new(),
            });
        }
        let threads = options
            .max_threads
            .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(4))
            .max(1);
        let done = AtomicUsize::new(0);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .map_err(std::io::Error::other)?;
        parsed_results = pool.install(|| {
            to_parse
                .par_iter()
                .filter_map(|p| {
                    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                        return None;
                    }
                    let res = parse_file(p);
                    done.fetch_add(1, Ordering::Relaxed);
                    res
                })
                .collect()
        });
        if let Some(cb) = progress.as_deref_mut() {
            cb(ScanProgress {
                phase: "scanning",
                scanned: total,
                total,
                current_file: String::new(),
            });
        }
    }

    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
        return Ok(ScanResult::default());
    }

    parsed_results.sort_by_key(|a| a.file_path.to_lowercase());

    let mut folder_cache: HashMap<String, AlbumFolders> = HashMap::new();
    let mut picture_source: HashMap<String, String> = HashMap::new();
    let mut added = 0usize;
    let mut updated = 0usize;
    let mut new_albums = 0usize;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    // Track existing file paths (lowercased) inside albums to avoid duplicate additions.
    let mut known_paths: HashSet<String> = HashSet::new();
    for album in library.values() {
        for t in album.tracks.iter().flatten() {
            if let Some(fp) = &t.file_path {
                known_paths.insert(fp.to_lowercase());
            }
        }
    }

    for r in parsed_results {
        let low = r.file_path.to_lowercase();
        let mut is_update = false;
        if changed.contains(&low)
            && let Some((old_key, _)) = by_path.get(&low).cloned()
            && let Some(old_album) = library.get_mut(&old_key)
            && let Some(tracks) = &mut old_album.tracks
        {
            tracks.retain(|t| {
                t.file_path
                    .as_ref()
                    .is_none_or(|fp| !fp.eq_ignore_ascii_case(&r.file_path))
            });
            folder_cache.remove(&old_key);
            touched.insert(old_key);
            known_paths.remove(&low);
            is_update = true;
        }

        let (key, created) = resolve_album(&mut library, &mut folder_cache, &r, now_ms);
        if created {
            new_albums += 1;
        }
        let album = library.get_mut(&key).expect("album just resolved");
        if album.year == 0 && r.year > 0 {
            album.year = r.year;
        }
        if !created
            && let Some(ra) = &r.album_artist
            && !ra.trim().is_empty()
            && let Some(aa) = &album.album_artist
            && !aa.trim().is_empty()
            && !same_artist(aa, ra)
        {
            mixed.insert(key.clone());
        } else if album.album_artist.as_deref().unwrap_or("").is_empty()
            && let Some(ra) = &r.album_artist
            && !ra.is_empty()
            && !mixed.contains(&key)
        {
            album.album_artist = Some(ra.clone());
        }

        if r.has_picture && !picture_source.contains_key(&key) {
            picture_source.insert(key.clone(), r.file_path.clone());
        }
        if known_paths.contains(&low) {
            continue;
        }

        let stamp = stamps.get(&low).copied();
        let track = TrackDto {
            id: Some(r.file_path.clone()),
            title: Some(r.title),
            artist: Some(r.artist),
            album: Some(key.clone()),
            duration: r.duration,
            file_path: Some(r.file_path.clone()),
            src: Some(CoverPaths::to_file_url(Path::new(&r.file_path))),
            track_number: Some(r.track_number),
            disc_number: if r.disc_number <= 0 { 1 } else { r.disc_number },
            codec: Some(r.codec),
            sample_rate: r.sample_rate,
            bits_per_sample: r.bits_per_sample,
            channels: r.channels,
            mtime: stamp.map(|s| s.0),
            size: stamp.map(|s| s.1),
            extra: Default::default(),
        };
        album.tracks.get_or_insert_with(Vec::new).push(track);
        folder_cache
            .entry(key.clone())
            .or_insert_with(|| AlbumFolders::from_album(album))
            .add(&r.file_path);
        known_paths.insert(low);
        touched.insert(key.clone());
        in_scan.insert(key);
        if is_update {
            updated += 1;
        } else {
            added += 1;
        }
    }

    for key in &touched {
        if let Some(a) = library.get(key)
            && a.album_artist.as_deref().unwrap_or("").trim().is_empty()
            && let Some(tracks) = &a.tracks
            && tracks.len() > 1
        {
            let distinct: HashSet<String> = tracks
                .iter()
                .map(|t| normalize_artist(t.artist.as_deref().unwrap_or("")))
                .collect();
            if distinct.len() > 1 {
                mixed.insert(key.clone());
            }
        }
    }
    for key in &mixed {
        if let Some(album) = library.get_mut(key)
            && let Some(tracks) = &album.tracks
            && !tracks.is_empty()
        {
            album.album_artist = None;
            album.artist = Some(common_artist(tracks.iter().map(|t| t.artist.as_deref())));
        }
    }

    for key in &touched {
        let empty = library
            .get(key)
            .is_some_and(|a| a.tracks.as_ref().is_none_or(|t| t.is_empty()));
        if empty {
            library.shift_remove(key);
            // Cover files only go with a confirmed prune; an orphaned cover is harmless.
            if options.prune_missing {
                try_delete(&CoverPaths::thumb_path(key));
                try_delete(&CoverPaths::full_path(key));
            }
            continue;
        }
        if let Some(album) = library.get_mut(key)
            && let Some(tracks) = &mut album.tracks
        {
            tracks.sort_by(|a, b| {
                let da = if a.disc_number <= 0 { 1 } else { a.disc_number };
                let db = if b.disc_number <= 0 { 1 } else { b.disc_number };
                da.cmp(&db)
                    .then_with(|| a.track_number.unwrap_or(0).cmp(&b.track_number.unwrap_or(0)))
            });
        }
    }

    let needing: Vec<String> = in_scan
        .into_iter()
        .filter(|k| {
            library
                .get(k)
                .is_some_and(|a| a.cover_art.as_deref().unwrap_or("").is_empty())
        })
        .collect();
    let mut covers = 0usize;
    let total_covers = needing.len();
    for (i, key) in needing.into_iter().enumerate() {
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            break;
        }
        if let Some(cb) = progress.as_deref_mut() {
            cb(ScanProgress {
                phase: "covers",
                scanned: i,
                total: total_covers,
                current_file: key.clone(),
            });
        }
        let pic_src = picture_source.get(&key).map(String::as_str);
        if let Some(album) = library.get(&key)
            && let Some((thumb, full)) = find_and_save_cover(&key, album, pic_src)
            && let Some(album_mut) = library.get_mut(&key)
        {
            album_mut.cover_art = Some(thumb);
            album_mut.cover_art_full = Some(full);
            covers += 1;
        }
    }

    let result = ScanResult {
        added_tracks: added,
        new_albums,
        covers,
        updated_tracks: updated,
        removed_tracks: removed,
        missing_tracks: if options.prune_missing { 0 } else { missing },
        merged_albums: merged,
    };
    if !mixed.is_empty() {
        dirty = true;
    }
    if result.changed() || dirty {
        LibraryStore::save_raw(&library)?;
    }
    Ok(result)
}

/// Library tracks under the scanned roots whose files are gone, as `(lowercase path, album key)`.
/// Tracks under unreadable folders are skipped; an empty or absent root counts as missing.
fn collect_missing(
    library: &LibraryMap,
    by_path: &HashMap<String, (String, usize)>,
    found: &Discovery,
    seen_lower: &HashSet<String>,
) -> Vec<(String, String)> {
    if found.walked_dirs.is_empty() && found.gone_dirs.is_empty() {
        return Vec::new();
    }
    by_path
        .iter()
        .filter_map(|(low_path, (album_key, idx))| {
            if seen_lower.contains(low_path) {
                return None;
            }
            let fp = library
                .get(album_key)
                .and_then(|a| a.tracks.as_ref())
                .and_then(|ts| ts.get(*idx))
                .and_then(|t| t.file_path.as_deref())?;
            if is_under_any(fp, &found.unreadable_dirs) {
                return None;
            }
            if !is_under_any(fp, &found.walked_dirs) && !is_under_any(fp, &found.gone_dirs) {
                return None;
            }
            if Path::new(fp).exists() {
                return None;
            }
            Some((low_path.clone(), album_key.clone()))
        })
        .collect()
}

/// Drops the given missing tracks from the library; returns how many were removed.
fn prune_tracks(
    library: &mut LibraryMap,
    gone: &[(String, String)],
    touched: &mut HashSet<String>,
) -> usize {
    let gone_paths: HashSet<&str> = gone.iter().map(|(p, _)| p.as_str()).collect();
    let mut removed = 0usize;
    for (_, album_key) in gone {
        if let Some(album) = library.get_mut(album_key)
            && let Some(tracks) = &mut album.tracks
        {
            let before = tracks.len();
            tracks.retain(|t| {
                t.file_path
                    .as_ref()
                    .is_none_or(|fp| !gone_paths.contains(fp.to_lowercase().as_str()))
            });
            removed += before - tracks.len();
            touched.insert(album_key.clone());
        }
    }
    removed
}

fn file_stamp(path: &Path) -> Option<(i64, i64)> {
    let meta = fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let ms = modified
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Some((ms, meta.len() as i64))
}

fn is_under_any(path: &str, dirs: &[PathBuf]) -> bool {
    let low_path = path.to_lowercase();
    for d in dirs {
        let ds = d.to_string_lossy();
        let trimmed = ds.trim_end_matches(['/', '\\']).to_lowercase();
        if low_path.len() > trimmed.len() + 1
            && low_path.starts_with(&trimmed)
            && matches!(low_path.as_bytes().get(trimmed.len()), Some(b'/') | Some(b'\\'))
        {
            return true;
        }
    }
    false
}

fn album_folder(file_path: &str) -> String {
    let p = Path::new(file_path);
    let Some(dir) = p.parent() else {
        return String::new();
    };
    if let Some(name) = dir.file_name().and_then(|n| n.to_str())
        && DISC_FOLDER_RX.is_match(name)
        && let Some(parent) = dir.parent()
    {
        let ps = parent.to_string_lossy();
        if !ps.is_empty() {
            return ps.into_owned();
        }
    }
    dir.to_string_lossy().into_owned()
}

fn same_artist(a: &str, b: &str) -> bool {
    normalize_artist(a) == normalize_artist(b)
}

fn normalize_artist(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

pub fn common_artist<'a>(track_artists: impl IntoIterator<Item = Option<&'a str>>) -> String {
    let mut common: Option<Vec<String>> = None;
    for artist in track_artists {
        let names: Vec<String> = ARTIST_SEP_RX
            .split(artist.unwrap_or(""))
            .map(|n| n.trim().to_owned())
            .filter(|n| !n.is_empty())
            .collect();
        match &mut common {
            None => common = Some(names),
            Some(prev) => {
                let keys: HashSet<String> = names.iter().map(|n| normalize_artist(n)).collect();
                prev.retain(|n| keys.contains(&normalize_artist(n)));
                if prev.is_empty() {
                    break;
                }
            }
        }
    }
    common
        .and_then(|c| c.into_iter().next())
        .unwrap_or_else(|| "Various Artists".to_owned())
}

fn merge_split_albums(
    library: &mut LibraryMap,
    touched: &mut HashSet<String>,
    mixed: &mut HashSet<String>,
) -> usize {
    let mut merged = 0usize;
    let keys: Vec<String> = library.keys().cloned().collect();
    for key in keys {
        let Some(cap) = SPLIT_ALBUM_RX.captures(&key) else {
            continue;
        };
        let base_key = cap["base"].to_owned();
        let who = cap["who"].to_owned();
        if base_key == key || !library.contains_key(&base_key) {
            continue;
        }
        let Some(split) = library.get(&key).cloned() else {
            continue;
        };
        let Some(split_tracks) = &split.tracks else {
            continue;
        };
        if split_tracks.is_empty() {
            continue;
        }
        let matches_who = split
            .album_artist
            .as_deref()
            .is_some_and(|a| same_artist(&who, a))
            || split.artist.as_deref().is_some_and(|a| same_artist(&who, a));
        if !matches_who {
            continue;
        }

        let target_folders: HashSet<String> = library
            .get(&base_key)
            .and_then(|a| a.tracks.as_ref())
            .into_iter()
            .flatten()
            .filter_map(|t| t.file_path.as_deref())
            .map(|fp| album_folder(fp).to_lowercase())
            .collect();
        if target_folders.is_empty() {
            continue;
        }
        let shares_folder = split_tracks
            .iter()
            .filter_map(|t| t.file_path.as_deref())
            .any(|fp| target_folders.contains(&album_folder(fp).to_lowercase()));
        if !shares_folder {
            continue;
        }

        let Some(mut split_owned) = library.shift_remove(&key) else {
            continue;
        };
        let Some(target) = library.get_mut(&base_key) else {
            library.insert(key, split_owned);
            continue;
        };
        if let Some(tracks) = &mut split_owned.tracks {
            let target_tracks = target.tracks.get_or_insert_with(Vec::new);
            for mut t in tracks.drain(..) {
                t.album = Some(base_key.clone());
                target_tracks.push(t);
            }
        }
        if target.cover_art.as_deref().unwrap_or("").is_empty()
            && !split_owned.cover_art.as_deref().unwrap_or("").is_empty()
        {
            target.cover_art = split_owned.cover_art;
            target.cover_art_full = split_owned.cover_art_full;
        } else {
            try_delete(&CoverPaths::thumb_path(&key));
            try_delete(&CoverPaths::full_path(&key));
        }
        if target.year == 0 {
            target.year = split_owned.year;
        }
        let t_at = if target.added_at == 0 { i64::MAX } else { target.added_at };
        let s_at = if split_owned.added_at == 0 { i64::MAX } else { split_owned.added_at };
        let min_at = t_at.min(s_at);
        target.added_at = if min_at == i64::MAX { 0 } else { min_at };

        let s_aa = split_owned
            .album_artist
            .as_deref()
            .or(split_owned.artist.as_deref())
            .unwrap_or("");
        let t_aa = target
            .album_artist
            .as_deref()
            .or(target.artist.as_deref())
            .unwrap_or("");
        if !same_artist(s_aa, t_aa) {
            mixed.insert(base_key.clone());
        }
        touched.insert(base_key);
        merged += 1;
    }
    merged
}

fn same_album(
    cache: &mut HashMap<String, AlbumFolders>,
    key: &str,
    album: &AlbumDto,
    r: &Parsed,
) -> bool {
    if album.tracks.as_ref().is_none_or(|t| t.is_empty()) {
        return true;
    }
    let folders = cache
        .entry(key.to_owned())
        .or_insert_with(|| AlbumFolders::from_album(album));
    if folders.is_same_folder(&r.file_path) {
        return true;
    }
    if let Some(aa) = album.album_artist.as_deref().filter(|s| !s.trim().is_empty())
        && let Some(ra) = r.album_artist.as_deref().filter(|s| !s.trim().is_empty())
    {
        return same_artist(aa, ra);
    }
    if folders.is_near(&r.file_path) {
        return true;
    }
    let candidate = r.album_artist.as_deref().unwrap_or(&r.artist);
    album
        .artist
        .as_deref()
        .is_some_and(|a| same_artist(a, candidate))
        || album
            .album_artist
            .as_deref()
            .is_some_and(|a| same_artist(a, &r.artist))
}

fn resolve_album(
    library: &mut LibraryMap,
    cache: &mut HashMap<String, AlbumFolders>,
    r: &Parsed,
    now_ms: i64,
) -> (String, bool) {
    let artist = r
        .album_artist
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&r.artist)
        .to_owned();

    for attempt in 0..=20 {
        let key = match attempt {
            0 => r.album.clone(),
            1 => format!("{} ({})", r.album, artist),
            n => format!("{} ({}) {}", r.album, artist, n),
        };
        if !library.contains_key(&key) {
            library.insert(
                key.clone(),
                AlbumDto {
                    name: Some(key.clone()),
                    artist: Some(artist),
                    album_artist: r
                        .album_artist
                        .as_deref()
                        .filter(|s| !s.trim().is_empty())
                        .map(str::to_owned),
                    year: 0,
                    added_at: now_ms,
                    tracks: Some(Vec::new()),
                    ..AlbumDto::default()
                },
            );
            return (key, true);
        }
        if attempt >= 20 {
            return (key, false);
        }
        let existing = &library[&key];
        if same_album(cache, &key, existing, r) {
            return (key, false);
        }
    }
    unreachable!()
}

fn parse_file(path: &Path) -> Option<Parsed> {
    let probe = Probe::open(path)
        .ok()?
        .options(ParseOptions::new().read_cover_art(false));
    let tagged = probe.read().ok()?;
    let props = tagged.properties();
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());

    let file_path = path.to_string_lossy().into_owned();
    let album_artist = tag
        .and_then(|t| t.get_string(ItemKey::AlbumArtist))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let artist = tag
        .and_then(|t| t.artist())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .or_else(|| album_artist.clone())
        .unwrap_or_else(|| "Unknown Artist".to_owned());
    let title = tag
        .and_then(|t| t.title())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown")
                .to_owned()
        });
    let album = tag
        .and_then(|t| t.album())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Unknown Album".to_owned());

    let track_number = tag.and_then(|t| t.track()).unwrap_or(0) as i32;
    let disc_number = tag.and_then(|t| t.disk()).filter(|&d| d > 0).unwrap_or(1) as i32;
    let year = tag
        .and_then(|t| t.date().map(|ts| ts.year as i32))
        .unwrap_or(0);

    // Quick cover presence check without reading large blobs for FLAC/MP3/MP4 where supported;
    // Note: with read_cover_art(false), tag.pictures() may be empty, so cover lookup inspects the
    // first tracks of albums lacking cover art in stage 6.
    let has_picture = tag.is_some_and(|t| !t.pictures().is_empty());

    Some(Parsed {
        file_path,
        title,
        artist,
        album_artist,
        album,
        duration: props.duration().as_secs_f64(),
        track_number,
        disc_number,
        year,
        codec: codec_name(tagged.file_type(), path),
        sample_rate: props.sample_rate().unwrap_or(0) as i32,
        bits_per_sample: props.bit_depth().unwrap_or(0) as i32,
        channels: props.channels().unwrap_or(0) as i32,
        has_picture,
    })
}

fn codec_name(ft: FileType, path: &Path) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ft {
        FileType::Flac => "FLAC".to_owned(),
        FileType::Mpeg => "MPEG 1 Layer 3".to_owned(),
        FileType::Opus => "Opus".to_owned(),
        FileType::Vorbis => "Vorbis I".to_owned(),
        FileType::Wav | FileType::Aiff => "PCM".to_owned(),
        FileType::Aac => "AAC".to_owned(),
        FileType::Mp4 => {
            if ext == "alac" {
                "ALAC".to_owned()
            } else {
                "AAC".to_owned()
            }
        }
        _ => match ext.as_str() {
            "wma" => "Windows Media Audio".to_owned(),
            "" => "AUDIO".to_owned(),
            other => other.to_ascii_uppercase(),
        },
    }
}

fn find_and_save_cover(
    album_key: &str,
    album: &AlbumDto,
    picture_source: Option<&str>,
) -> Option<(String, String)> {
    let mut data: Option<Vec<u8>> = picture_source.and_then(|p| read_embedded_picture(Path::new(p)));
    let tracks: Vec<&str> = album
        .tracks
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .filter_map(|t| t.file_path.as_deref())
        .collect();

    if data.is_none() {
        for &p in tracks.iter().take(3) {
            if picture_source.is_some_and(|ps| ps.eq_ignore_ascii_case(p)) {
                continue;
            }
            if let Some(bytes) = read_embedded_picture(Path::new(p)) {
                data = Some(bytes);
                break;
            }
        }
    }

    if data.is_none() {
        let mut folders: Vec<String> = Vec::new();
        for &p in &tracks {
            if let Some(dir) = Path::new(p).parent() {
                let d = dir.to_string_lossy().into_owned();
                if !d.is_empty() && !folders.iter().any(|x| x.eq_ignore_ascii_case(&d)) {
                    folders.push(d);
                }
            }
            let af = album_folder(p);
            if !af.is_empty() && !folders.iter().any(|x| x.eq_ignore_ascii_case(&af)) {
                folders.push(af);
            }
            if folders.len() >= 8 {
                break;
            }
        }
        for folder in folders.into_iter().take(8) {
            if let Some(bytes) = read_folder_image(Path::new(&folder)) {
                data = Some(bytes);
                break;
            }
        }
    }

    save_cover(album_key, &data?)
}

fn read_embedded_picture(source_file: &Path) -> Option<Vec<u8>> {
    let tagged = Probe::open(source_file).ok()?.read().ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    let pic = tag
        .get_picture_type(PictureType::CoverFront)
        .or_else(|| tag.pictures().first())?;
    if pic.data().is_empty() {
        None
    } else {
        Some(pic.data().to_vec())
    }
}

fn read_folder_image(folder: &Path) -> Option<Vec<u8>> {
    let entries = fs::read_dir(folder).ok()?;
    let images: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && PlaylistCovers::is_supported_ext(p))
        .collect();
    for &target_name in FOLDER_COVER_NAMES {
        if let Some(match_path) = images.iter().find(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|stem| stem.eq_ignore_ascii_case(target_name))
        }) && let Ok(meta) = fs::metadata(match_path)
            && meta.len() <= MAX_COVER_FILE_BYTES
        {
            return fs::read(match_path).ok();
        }
    }
    None
}

fn save_cover(album_name: &str, data: &[u8]) -> Option<(String, String)> {
    if !ImageLimits::is_sane_bytes(data) {
        return None;
    }
    fs::create_dir_all(CoverPaths::cover_dir()).ok()?;
    let thumb = CoverPaths::thumb_path(album_name);
    let full = CoverPaths::full_path(album_name);
    if !full.exists() {
        fs::write(&full, data).ok()?;
    }
    if !thumb.exists() {
        let thumb_bytes = resize_to_jpeg(data, 400, 80).unwrap_or_else(|_| data.to_vec());
        fs::write(&thumb, thumb_bytes).ok()?;
    }
    Some((
        CoverPaths::to_file_url(&thumb),
        CoverPaths::to_file_url(&full),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_artist_splits_separators() {
        let res = common_artist([
            Some("Cö shu Nie & Wuthering Waves"),
            Some("Wuthering Waves, SoYeon"),
        ]);
        assert_eq!(res, "Wuthering Waves");
        assert_eq!(
            common_artist([Some("Alice"), Some("Bob")]),
            "Various Artists"
        );
    }

    #[test]
    fn merges_split_albums_in_same_folder() {
        let mut lib = LibraryMap::new();
        lib.insert(
            "EP".to_owned(),
            AlbumDto {
                name: Some("EP".to_owned()),
                artist: Some("Artist A".to_owned()),
                album_artist: Some("Artist A".to_owned()),
                tracks: Some(vec![TrackDto {
                    file_path: Some("/music/ep/01.flac".to_owned()),
                    artist: Some("Artist A".to_owned()),
                    ..TrackDto::default()
                }]),
                ..AlbumDto::default()
            },
        );
        lib.insert(
            "EP (Artist B)".to_owned(),
            AlbumDto {
                name: Some("EP (Artist B)".to_owned()),
                artist: Some("Artist B".to_owned()),
                album_artist: Some("Artist B".to_owned()),
                tracks: Some(vec![TrackDto {
                    file_path: Some("/music/ep/02.flac".to_owned()),
                    artist: Some("Artist B".to_owned()),
                    ..TrackDto::default()
                }]),
                ..AlbumDto::default()
            },
        );

        let mut touched = HashSet::new();
        let mut mixed = HashSet::new();
        let merged = merge_split_albums(&mut lib, &mut touched, &mut mixed);
        assert_eq!(merged, 1);
        assert!(lib.contains_key("EP"));
        assert!(!lib.contains_key("EP (Artist B)"));
        assert_eq!(lib["EP"].tracks.as_ref().unwrap().len(), 2);
        assert!(mixed.contains("EP"));
    }

    fn album_with(paths: &[&str]) -> AlbumDto {
        AlbumDto {
            tracks: Some(
                paths
                    .iter()
                    .map(|p| TrackDto {
                        file_path: Some((*p).to_owned()),
                        ..TrackDto::default()
                    })
                    .collect(),
            ),
            ..AlbumDto::default()
        }
    }

    #[test]
    fn offline_roots_report_missing_tracks_and_prune_only_on_request() {
        let base = std::env::temp_dir().join(format!("retrograde-prune-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        // An unmounted drive: the mount point exists but is empty.
        let empty_mount = base.join("usb");
        fs::create_dir_all(&empty_mount).unwrap();
        let empty_mount = fs::canonicalize(&empty_mount).unwrap();
        // A root that is gone together with its parent.
        let absent = base.join("gone-parent").join("music");
        // A healthy root with its file present.
        let healthy = base.join("ok");
        fs::create_dir_all(&healthy).unwrap();
        let healthy = fs::canonicalize(&healthy).unwrap();
        let present = healthy.join("01.flac");
        fs::write(&present, b"").unwrap();

        let usb_track = empty_mount.join("Album").join("01.flac");
        let absent_track = absent.join("Album").join("02.flac");
        let elsewhere = base.join("not-scanned").join("03.flac");
        let s = |p: &Path| p.to_string_lossy().into_owned();
        let mut library = LibraryMap::new();
        library.insert("USB".to_owned(), album_with(&[&s(&usb_track)]));
        library.insert("Absent".to_owned(), album_with(&[&s(&absent_track)]));
        library.insert("Ok".to_owned(), album_with(&[&s(&present)]));
        library.insert("Elsewhere".to_owned(), album_with(&[&s(&elsewhere)]));

        let mut by_path = HashMap::new();
        for (key, album) in &library {
            for (idx, t) in album.tracks.iter().flatten().enumerate() {
                by_path.insert(t.file_path.as_ref().unwrap().to_lowercase(), (key.clone(), idx));
            }
        }
        let found = walk_paths(&[empty_mount.clone(), absent.clone(), healthy.clone()]);
        let seen: HashSet<String> = found
            .files
            .iter()
            .map(|f| f.to_string_lossy().to_lowercase())
            .collect();

        let mut gone = collect_missing(&library, &by_path, &found, &seen);
        gone.sort();
        let keys: Vec<&str> = gone.iter().map(|(_, k)| k.as_str()).collect();
        assert_eq!(keys, vec!["Absent", "USB"]);

        let mut touched = HashSet::new();
        assert_eq!(prune_tracks(&mut library, &gone, &mut touched), 2);
        assert!(library["USB"].tracks.as_ref().unwrap().is_empty());
        assert!(library["Absent"].tracks.as_ref().unwrap().is_empty());
        assert_eq!(library["Ok"].tracks.as_ref().unwrap().len(), 1);
        assert_eq!(library["Elsewhere"].tracks.as_ref().unwrap().len(), 1);
        assert!(touched.contains("USB") && touched.contains("Absent"));

        let _ = fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn discover_skips_symlink_cycles() {
        let root = std::env::temp_dir().join(format!("retrograde-symlink-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let sub = root.join("album");
        fs::create_dir_all(&sub).unwrap();
        let flac = sub.join("track.flac");
        fs::write(&flac, b"").unwrap();
        std::os::unix::fs::symlink(&root, sub.join("loop")).unwrap();

        let files = Scanner::discover(std::slice::from_ref(&root));
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("track.flac"));

        let _ = fs::remove_dir_all(&root);
    }
}
