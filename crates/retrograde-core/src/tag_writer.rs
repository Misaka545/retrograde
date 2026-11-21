use std::fs::File;
use std::path::{Path, PathBuf};

use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::{Picture, PictureType};
use lofty::probe::Probe;
use lofty::tag::items::Timestamp;
use lofty::tag::{Accessor, ItemKey, Tag};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TagWriteError {
    #[error("Failed to open or save audio file: {0}")]
    Lofty(String),
    #[error("Failed to read cover image: {0}")]
    Io(#[from] std::io::Error),
    #[error("Unsupported audio file (no tag container available)")]
    NoTag,
}

#[derive(Debug, Clone, Default)]
pub struct TagEdit {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub year: Option<u32>,
    pub track: Option<u32>,
    pub disc: Option<u32>,
    pub cover_image: Option<PathBuf>,
    pub remove_cover: bool,
}

impl TagEdit {
    pub fn changes_cover(&self) -> bool {
        self.cover_image.is_some() || self.remove_cover
    }
}

pub struct TagWriter;

impl TagWriter {
    /// Writes `edit` to every path in `paths`, returning `(written_count, failures)`
    /// without stopping on individual file errors.
    pub fn write_batch(paths: &[PathBuf], edit: &TagEdit) -> (usize, Vec<(PathBuf, String)>) {
        let mut ok = 0usize;
        let mut errs = Vec::new();
        for p in paths {
            match Self::write(p, edit) {
                Ok(()) => ok += 1,
                Err(e) => errs.push((p.clone(), e.to_string())),
            }
        }
        (ok, errs)
    }

    pub fn write(path: &Path, edit: &TagEdit) -> Result<(), TagWriteError> {
        let mut tagged = Probe::open(path)
            .map_err(|e| TagWriteError::Lofty(e.to_string()))?
            .read()
            .map_err(|e| TagWriteError::Lofty(e.to_string()))?;
        let tag_type = tagged.primary_tag_type();
        if tagged.primary_tag().is_none() {
            tagged.insert_tag(Tag::new(tag_type));
        }
        let tag = tagged.primary_tag_mut().ok_or(TagWriteError::NoTag)?;

        if let Some(title) = &edit.title {
            let trimmed = title.trim();
            if trimmed.is_empty() {
                tag.remove_title();
            } else {
                tag.set_title(trimmed.to_owned());
            }
        }
        if let Some(artist) = &edit.artist {
            let trimmed = artist.trim();
            if trimmed.is_empty() {
                tag.remove_artist();
            } else {
                tag.set_artist(trimmed.to_owned());
            }
        }
        if let Some(album) = &edit.album {
            let trimmed = album.trim();
            if trimmed.is_empty() {
                tag.remove_album();
            } else {
                tag.set_album(trimmed.to_owned());
            }
        }
        if let Some(album_artist) = &edit.album_artist {
            let trimmed = album_artist.trim();
            if trimmed.is_empty() {
                tag.remove_key(ItemKey::AlbumArtist);
            } else {
                tag.insert_text(ItemKey::AlbumArtist, trimmed.to_owned());
            }
        }
        if let Some(year) = edit.year {
            if year == 0 {
                tag.remove_date();
            } else {
                tag.set_date(Timestamp {
                    year: year.min(u16::MAX as u32) as u16,
                    ..Timestamp::default()
                });
            }
        }
        if let Some(track) = edit.track {
            if track == 0 {
                tag.remove_track();
            } else {
                tag.set_track(track);
            }
        }
        if let Some(disc) = edit.disc {
            if disc == 0 {
                tag.remove_disk();
            } else {
                tag.set_disk(disc);
            }
        }
        if let Some(image_path) = &edit.cover_image {
            let mut file = File::open(image_path)?;
            let mut picture =
                Picture::from_reader(&mut file).map_err(|e| TagWriteError::Lofty(e.to_string()))?;
            picture.set_pic_type(PictureType::CoverFront);
            picture.set_description(Some("Cover".to_owned()));
            while !tag.pictures().is_empty() {
                tag.remove_picture(0);
            }
            tag.push_picture(picture);
        } else if edit.remove_cover {
            while !tag.pictures().is_empty() {
                tag.remove_picture(0);
            }
        }

        tagged
            .save_to_path(path, WriteOptions::default())
            .map_err(|e| TagWriteError::Lofty(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_flac_bytes() -> Vec<u8> {
        let mut v = Vec::with_capacity(42);
        v.extend_from_slice(b"fLaC");
        v.extend_from_slice(&[0x80, 0x00, 0x00, 0x22]); // last block = STREAMINFO (34 bytes)
        v.extend_from_slice(&[0x10, 0x00, 0x10, 0x00]); // min/max block 4096
        v.extend_from_slice(&[0x00; 6]); // min/max frame size unknown
        // 44100 Hz (0x0AC44), 2 ch (1), 16 bps (15), 44100 samples
        v.extend_from_slice(&[0x0A, 0xC4, 0x42, 0xF0, 0x00, 0x00, 0xAC, 0x44]);
        v.extend_from_slice(&[0x00; 16]); // MD5
        v
    }

    #[test]
    fn write_batch_preserves_untouched_fields_and_reports_errors() {
        let dir = std::env::temp_dir().join(format!(
            "retro-tag-batch-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let a = dir.join("a.flac");
        let b = dir.join("b.flac");
        let missing = dir.join("missing.flac");
        std::fs::write(&a, minimal_flac_bytes()).expect("write a");
        std::fs::write(&b, minimal_flac_bytes()).expect("write b");

        let seed_a = TagEdit {
            title: Some("Track One".into()),
            artist: Some("Artist A".into()),
            album: Some("Old Album".into()),
            year: Some(2020),
            ..Default::default()
        };
        let seed_b = TagEdit {
            title: Some("Track Two".into()),
            artist: Some("Artist B".into()),
            album: Some("Old Album".into()),
            year: Some(2020),
            ..Default::default()
        };
        TagWriter::write(&a, &seed_a).expect("seed a");
        TagWriter::write(&b, &seed_b).expect("seed b");

        // Album-level edit: changes only album + year; track titles & artists stay intact.
        let batch = TagEdit {
            album: Some("New Album".into()),
            year: Some(2024),
            ..Default::default()
        };
        let (written, errs) = TagWriter::write_batch(&[a.clone(), missing.clone(), b.clone()], &batch);
        assert_eq!(written, 2);
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].0, missing);

        let ta = Probe::open(&a).unwrap().read().unwrap();
        let tb = Probe::open(&b).unwrap().read().unwrap();
        let p_a = ta.primary_tag().unwrap();
        let p_b = tb.primary_tag().unwrap();
        assert_eq!(p_a.title().as_deref(), Some("Track One"));
        assert_eq!(p_a.artist().as_deref(), Some("Artist A"));
        assert_eq!(p_a.album().as_deref(), Some("New Album"));
        assert_eq!(p_b.title().as_deref(), Some("Track Two"));
        assert_eq!(p_b.artist().as_deref(), Some("Artist B"));
        assert_eq!(p_b.album().as_deref(), Some("New Album"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
