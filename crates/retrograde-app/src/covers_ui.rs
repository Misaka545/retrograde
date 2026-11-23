use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use iced::widget::{container, image, text};
use iced::{Alignment, Background, Border, Color, ContentFit, Element, Length};
use retrograde_core::{Album, CollectionsData, CoverPaths, PlaylistCovers};

use crate::theme;

/// Maximum full-resolution album/playlist covers retained in the GPU handle cache.
pub const MAX_FULL_COVERS: usize = 8;

/// Covers are sometimes PNG/WebP data saved under a `.jpg` name; the loader picks its decoder from the
/// extension for paths and then fails, leaving a blank cover. Those are read from bytes, which sniff the format.
fn load_handle(path: &Path) -> image::Handle {
    use std::io::Read;
    let mut head = [0u8; 12];
    let sniffed = std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_ok();
    if sniffed {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let is_png = head.starts_with(&[0x89, b'P', b'N', b'G']);
        let is_jpeg = head.starts_with(&[0xFF, 0xD8]);
        let is_webp = &head[..4] == b"RIFF" && &head[8..12] == b"WEBP";
        let mismatch = (is_png && ext != "png") || (is_jpeg && !matches!(ext.as_str(), "jpg" | "jpeg")) || (is_webp && ext != "webp");
        if mismatch && let Ok(bytes) = std::fs::read(path) {
            return image::Handle::from_bytes(bytes);
        }
    }
    image::Handle::from_path(path)
}

#[derive(Default)]
pub struct CoverCache {
    thumbs: HashMap<PathBuf, image::Handle>,
    fulls: HashMap<PathBuf, image::Handle>,
    full_lru: VecDeque<PathBuf>,
    playlist_live: HashSet<PathBuf>,
}

impl CoverCache {
    pub fn sync_albums(&mut self, albums: &[Album]) {
        let mut live: HashSet<PathBuf> = HashSet::with_capacity(albums.len() * 2 + self.playlist_live.len());
        live.extend(self.playlist_live.iter().cloned());
        for album in albums {
            // Register the small thumbnail for grid slots, or the full cover when no
            // thumbnail exists so the album never renders as a monogram.
            let preferred = album
                .cover_thumb_path
                .as_ref()
                .or(album.cover_full_path.as_ref());
            if let Some(p) = preferred {
                self.thumbs
                    .entry(p.clone())
                    .or_insert_with(|| load_handle(p));
            }
            if let Some(p) = &album.cover_thumb_path {
                live.insert(p.clone());
            }
            if let Some(p) = &album.cover_full_path {
                live.insert(p.clone());
            }
        }
        self.thumbs.retain(|k, _| live.contains(k));
        self.fulls.retain(|k, _| live.contains(k));
        self.full_lru.retain(|k| live.contains(k));
    }

    /// Registers custom playlist covers (thumbnails) from `collections`.
    pub fn sync_collections(&mut self, collections: &CollectionsData) {
        self.playlist_live.clear();
        for p in &collections.playlists {
            let Some(full) = CoverPaths::to_local_path(p.cover_art.as_deref()) else {
                continue;
            };
            let thumb = PlaylistCovers::thumb_for(&full);
            self.playlist_live.insert(full);
            self.playlist_live.insert(thumb.clone());
            self.thumbs
                .entry(thumb.clone())
                .or_insert_with(|| load_handle(&thumb));
        }
    }

    pub fn ensure_thumb(&mut self, path: &Path) {
        if !self.thumbs.contains_key(path) {
            self.thumbs
                .insert(path.to_path_buf(), load_handle(path));
        }
    }

    pub fn ensure_full(&mut self, path: &Path) {
        if self.fulls.contains_key(path) {
            self.touch_full(path);
            return;
        }
        self.insert_full_handle(path.to_path_buf(), load_handle(path));
    }

    fn touch_full(&mut self, path: &Path) {
        if let Some(pos) = self.full_lru.iter().position(|p| p == path)
            && let Some(key) = self.full_lru.remove(pos)
        {
            self.full_lru.push_back(key);
        }
    }

    fn insert_full_handle(&mut self, path: PathBuf, handle: image::Handle) {
        if !self.fulls.contains_key(&path) && self.full_lru.len() >= MAX_FULL_COVERS
            && let Some(evict) = self.full_lru.pop_front()
        {
            self.fulls.remove(&evict);
        }
        self.fulls.insert(path.clone(), handle);
        self.full_lru.retain(|p| p != &path);
        self.full_lru.push_back(path);
    }

    /// Drops stale cached handles for `paths` and reloads any existing files from bytes so
    /// Iced allocates new GPU texture atlas IDs even when a cover overwrites an existing path.
    pub fn invalidate_paths(&mut self, paths: &[PathBuf]) {
        for p in paths {
            let was_thumb = self.thumbs.remove(p).is_some();
            let was_full = self.fulls.remove(p).is_some();
            if was_full {
                self.full_lru.retain(|x| x != p);
            }
            if let Ok(bytes) = std::fs::read(p) {
                if was_thumb {
                    self.thumbs.insert(p.clone(), image::Handle::from_bytes(bytes.clone()));
                }
                if was_full || !was_thumb {
                    self.insert_full_handle(p.clone(), image::Handle::from_bytes(bytes));
                }
            }
        }
    }

    /// Returns the first cached handle among `paths`, in preference order
    /// (e.g. `[thumb, full]` for slots of 300px or less, or `[full, thumb]` for large headers).
    pub fn get_first(&self, paths: &[Option<&Path>]) -> Option<&image::Handle> {
        paths
            .iter()
            .flatten()
            .find_map(|p| self.thumbs.get(*p).or_else(|| self.fulls.get(*p)))
    }
}

pub fn render_cover<'a, M: 'a>(
    handle: Option<&image::Handle>,
    size: f32,
    radius: f32,
    accent: Color,
    monogram: &str,
) -> Element<'a, M> {
    if let Some(h) = handle {
        image(h.clone())
            .width(Length::Fixed(size))
            .height(Length::Fixed(size))
            .content_fit(ContentFit::Cover)
            .border_radius(radius)
            .into()
    } else {
        let ch = monogram
            .trim()
            .chars()
            .next()
            .unwrap_or('♪')
            .to_ascii_uppercase()
            .to_string();
        let font_size = (size * 0.36).clamp(11.0, 46.0);
        container(text(ch).size(font_size).color(theme::with_alpha(accent, 0.72)))
            .width(Length::Fixed(size))
            .height(Length::Fixed(size))
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .style(move |_theme| container::Style {
                background: Some(Background::Color(theme::RAISED)),
                border: Border {
                    color: theme::LINE,
                    width: 1.0,
                    radius: radius.into(),
                },
                ..Default::default()
            })
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_cover_lru_caps_at_8_and_promotes_on_hit() {
        let mut cache = CoverCache::default();
        let paths: Vec<PathBuf> = (0..10)
            .map(|i| PathBuf::from(format!("/tmp/cover_{i}.jpg")))
            .collect();
        for p in &paths[..8] {
            cache.ensure_full(p);
        }
        assert_eq!(cache.fulls.len(), MAX_FULL_COVERS);
        assert!(cache.get_first(&[Some(paths[0].as_path())]).is_some());

        // Adding a 9th cover evicts the oldest unused one (`paths[1]`, since `paths[0]`? wait: `get_first` is &self, `ensure_full` promotes!)
        cache.ensure_full(&paths[0]);
        cache.ensure_full(&paths[8]);
        assert_eq!(cache.fulls.len(), MAX_FULL_COVERS);
        assert!(cache.get_first(&[Some(paths[0].as_path())]).is_some());
        assert!(cache.get_first(&[Some(paths[1].as_path())]).is_none());
        assert!(cache.get_first(&[Some(paths[8].as_path())]).is_some());
    }
}
