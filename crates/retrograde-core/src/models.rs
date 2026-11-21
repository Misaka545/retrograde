use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Track {
    /// Track identity is the absolute file path (shared with Electron & C# builds).
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: f64,
    pub file_path: String,
    pub track_number: i32,
    pub disc_number: i32,
    pub codec: String,
    pub sample_rate: i32,
    pub bits_per_sample: i32,
    pub channels: i32,
    pub cover_thumb_path: Option<PathBuf>,
    pub cover_full_path: Option<PathBuf>,
}

impl Track {
    pub fn short_codec(&self) -> String {
        let raw = if self.codec.is_empty() {
            Path::new(&self.file_path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
        } else {
            self.codec.as_str()
        };
        let u = raw.trim().to_ascii_uppercase();
        if u.contains("FLAC") {
            "FLAC".to_owned()
        } else if u.contains("ALAC") {
            "ALAC".to_owned()
        } else if u.contains("AAC") || u == "M4A" {
            "AAC".to_owned()
        } else if u.contains("OPUS") {
            "OPUS".to_owned()
        } else if u.contains("VORBIS") || u == "OGG" {
            "OGG".to_owned()
        } else if u.contains("MP3") || u.contains("MPEG") {
            "MP3".to_owned()
        } else if u.contains("WAV") || u.contains("PCM") {
            "WAV".to_owned()
        } else if u.contains("AIFF") {
            "AIFF".to_owned()
        } else {
            u
        }
    }

    pub fn quality_label(&self) -> String {
        let mut parts = vec![self.short_codec()];
        if self.bits_per_sample > 0 {
            parts.push(format!("{}BIT", self.bits_per_sample));
        }
        if self.sample_rate > 0 {
            if self.sample_rate >= 1000 {
                let sr = self.sample_rate;
                let khz = if sr % 1000 == 0 {
                    format!("{}KHZ", sr / 1000)
                } else {
                    format!("{:.1}KHZ", sr as f64 / 1000.0)
                };
                parts.push(khz);
            } else {
                parts.push(format!("{}HZ", self.sample_rate));
            }
        }
        parts.join(" / ")
    }

    pub fn album_quality_label(tracks: &[Track]) -> Option<String> {
        let first = tracks.first()?;
        let first_codec = first.short_codec();
        let mixed = tracks.iter().skip(1).any(|t| t.short_codec() != first_codec);
        let mut parts = vec![if mixed { "MIXED".to_owned() } else { first_codec }];
        if first.bits_per_sample > 0 {
            parts.push(format!("{}BIT", first.bits_per_sample));
        }
        if first.sample_rate > 0 {
            if first.sample_rate >= 1000 {
                let sr = first.sample_rate;
                parts.push(if sr % 1000 == 0 {
                    format!("{}KHZ", sr / 1000)
                } else {
                    format!("{:.1}KHZ", sr as f64 / 1000.0)
                });
            } else {
                parts.push(format!("{}HZ", first.sample_rate));
            }
        }
        Some(parts.join(" / "))
    }
}

#[derive(Debug, Clone)]
pub struct Album {
    pub name: String,
    pub artist: String,
    pub cover_thumb_path: Option<PathBuf>,
    pub cover_full_path: Option<PathBuf>,
    pub year: i32,
    pub added_at: i64,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone)]
pub struct ArtistSummary {
    pub name: String,
    pub albums: Vec<Album>,
    pub track_count: usize,
    pub newest_added_at: i64,
}
