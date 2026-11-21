use std::collections::HashSet;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::collections::{SmartPlaylistData, SmartRule};
use crate::models::{Album, Track};
use crate::stats::PlayStat;

#[derive(Debug, Clone)]
pub struct SmartField {
    pub key: &'static str,
    pub label: &'static str,
    pub ops: &'static [&'static str],
    pub hint: &'static str,
}

pub struct SmartPlaylists;

impl SmartPlaylists {
    pub const FIELDS: &'static [SmartField] = &[
        SmartField {
            key: "title",
            label: "TITLE",
            ops: &["contains", "not_contains", "is"],
            hint: "TEXT",
        },
        SmartField {
            key: "artist",
            label: "ARTIST",
            ops: &["contains", "not_contains", "is"],
            hint: "TEXT",
        },
        SmartField {
            key: "album",
            label: "ALBUM",
            ops: &["contains", "not_contains", "is"],
            hint: "TEXT",
        },
        SmartField {
            key: "codec",
            label: "FORMAT",
            ops: &["is", "is_not"],
            hint: "FLAC, MP3, AAC...",
        },
        SmartField {
            key: "bits",
            label: "BIT DEPTH",
            ops: &["gte", "lte", "eq"],
            hint: "16, 24...",
        },
        SmartField {
            key: "rate",
            label: "SAMPLE RATE",
            ops: &["gte", "lte", "eq"],
            hint: "KHZ: 44.1, 96...",
        },
        SmartField {
            key: "year",
            label: "YEAR",
            ops: &["gte", "lte", "eq"],
            hint: "1999",
        },
        SmartField {
            key: "duration",
            label: "LENGTH",
            ops: &["gte", "lte"],
            hint: "MINUTES",
        },
        SmartField {
            key: "plays",
            label: "PLAYS",
            ops: &["gte", "lte", "eq"],
            hint: "COUNT",
        },
        SmartField {
            key: "played",
            label: "LAST PLAYED",
            ops: &["within", "not_within", "never"],
            hint: "DAYS",
        },
        SmartField {
            key: "added",
            label: "ADDED",
            ops: &["within", "not_within"],
            hint: "DAYS",
        },
        SmartField {
            key: "liked",
            label: "FAVORITE",
            ops: &["yes", "no"],
            hint: "",
        },
    ];

    pub const SORTS: &'static [(&'static str, &'static str)] = &[
        ("album", "ALBUM ORDER"),
        ("added_desc", "RECENTLY ADDED"),
        ("plays_desc", "MOST PLAYED"),
        ("played_desc", "RECENTLY PLAYED"),
        ("title", "TITLE [A-Z]"),
        ("random", "RANDOM"),
    ];

    pub fn op_label(op: &str) -> &'static str {
        match op {
            "contains" => "CONTAINS",
            "not_contains" => "DOESN'T CONTAIN",
            "is" => "IS",
            "is_not" => "IS NOT",
            "gte" => "AT LEAST",
            "lte" => "AT MOST",
            "eq" => "EXACTLY",
            "within" => "IN THE LAST",
            "not_within" => "NOT IN THE LAST",
            "never" => "NEVER",
            "yes" => "YES",
            "no" => "NO",
            _ => "",
        }
    }

    pub fn field(key: &str) -> &'static SmartField {
        Self::FIELDS
            .iter()
            .find(|f| f.key == key)
            .unwrap_or(&Self::FIELDS[0])
    }

    pub fn needs_value(op: &str) -> bool {
        !matches!(op, "never" | "yes" | "no")
    }

    pub fn examples() -> Vec<SmartPlaylistData> {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(1);
        vec![
            SmartPlaylistData {
                id,
                name: "Hi-Res 24 bit".to_owned(),
                rules: vec![SmartRule {
                    field: "bits".to_owned(),
                    op: "gte".to_owned(),
                    value: "24".to_owned(),
                }],
                ..SmartPlaylistData::default()
            },
            SmartPlaylistData {
                id: id + 1,
                name: "Added in 30 days".to_owned(),
                sort: "added_desc".to_owned(),
                rules: vec![SmartRule {
                    field: "added".to_owned(),
                    op: "within".to_owned(),
                    value: "30".to_owned(),
                }],
                ..SmartPlaylistData::default()
            },
            SmartPlaylistData {
                id: id + 2,
                name: "On Repeat".to_owned(),
                sort: "plays_desc".to_owned(),
                limit: 50,
                rules: vec![SmartRule {
                    field: "plays".to_owned(),
                    op: "gte".to_owned(),
                    value: "5".to_owned(),
                }],
                ..SmartPlaylistData::default()
            },
            SmartPlaylistData {
                id: id + 3,
                name: "Forgotten Favorites".to_owned(),
                sort: "random".to_owned(),
                rules: vec![
                    SmartRule {
                        field: "liked".to_owned(),
                        op: "yes".to_owned(),
                        value: String::new(),
                    },
                    SmartRule {
                        field: "played".to_owned(),
                        op: "not_within".to_owned(),
                        value: "90".to_owned(),
                    },
                ],
                ..SmartPlaylistData::default()
            },
        ]
    }

    /// Evaluates a smart playlist over `(Track, Album)` entries at `now_ms` (Unix milliseconds).
    pub fn evaluate<'a, FStats, FLiked>(
        playlist: &SmartPlaylistData,
        library: impl IntoIterator<Item = (&'a Track, &'a Album)>,
        stats: FStats,
        liked: FLiked,
        now_ms: i64,
    ) -> Vec<Track>
    where
        FStats: Fn(&str) -> Option<&'a PlayStat>,
        FLiked: Fn(&str) -> bool,
    {
        let rules: Vec<&SmartRule> = playlist
            .rules
            .iter()
            .filter(|r| !Self::needs_value(&r.op) || !r.value.trim().is_empty())
            .collect();

        let mut hits: Vec<(&'a Track, &'a Album)> = Vec::new();
        for (track, album) in library {
            if rules.is_empty() {
                hits.push((track, album));
                continue;
            }
            let ok = if playlist.match_all {
                rules
                    .iter()
                    .all(|r| matches_rule(r, track, album, &stats, &liked, now_ms))
            } else {
                rules
                    .iter()
                    .any(|r| matches_rule(r, track, album, &stats, &liked, now_ms))
            };
            if ok {
                hits.push((track, album));
            }
        }

        match playlist.sort.as_str() {
            "added_desc" => hits.sort_by_key(|a| std::cmp::Reverse(a.1.added_at)),
            "plays_desc" => hits.sort_by(|a, b| {
                let sa = stats(&a.0.id);
                let sb = stats(&b.0.id);
                let ca = sa.map(|s| s.count).unwrap_or(0);
                let cb = sb.map(|s| s.count).unwrap_or(0);
                cb.cmp(&ca)
                    .then_with(|| sb.map(|s| s.last).unwrap_or(0).cmp(&sa.map(|s| s.last).unwrap_or(0)))
            }),
            "played_desc" => hits.sort_by(|a, b| {
                let la = stats(&a.0.id).map(|s| s.last).unwrap_or(0);
                let lb = stats(&b.0.id).map(|s| s.last).unwrap_or(0);
                lb.cmp(&la)
            }),
            "title" => hits.sort_by(|a, b| {
                a.0.title
                    .to_lowercase()
                    .cmp(&b.0.title.to_lowercase())
            }),
            "random" => {
                let day = now_ms.div_euclid(86_400_000) as u64;
                hits.sort_by_key(|(t, _)| daily_random_key(&t.id, playlist.id as u64, day));
            }
            _ => {}
        }

        let iter = hits.into_iter().map(|(t, _)| t.clone());
        if playlist.limit > 0 {
            iter.take(playlist.limit).collect()
        } else {
            iter.collect()
        }
    }

    pub fn evaluate_albums<'a, FStats>(
        playlist: &SmartPlaylistData,
        albums: &'a [Album],
        stats: FStats,
        liked_ids: &HashSet<String>,
        now_ms: i64,
    ) -> Vec<Track>
    where
        FStats: Fn(&str) -> Option<&'a PlayStat>,
    {
        let entries = albums
            .iter()
            .flat_map(|a| a.tracks.iter().map(move |t| (t, a)));
        Self::evaluate(
            playlist,
            entries,
            stats,
            |id| liked_ids.contains(id),
            now_ms,
        )
    }
}

fn daily_random_key(track_id: &str, playlist_id: u64, day: u64) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ playlist_id ^ day.rotate_left(17);
    for b in track_id.to_lowercase().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    // SplitMix64 finalizer so consecutive paths spread evenly
    h ^= h >> 30;
    h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^ (h >> 31)
}

fn matches_rule<'a, FStats, FLiked>(
    r: &SmartRule,
    t: &Track,
    a: &Album,
    stats: &FStats,
    liked: &FLiked,
    now_ms: i64,
) -> bool
where
    FStats: Fn(&str) -> Option<&'a PlayStat>,
    FLiked: Fn(&str) -> bool,
{
    let v = r.value.trim();
    match r.field.as_str() {
        "title" => cmp_text(&t.title, &r.op, v),
        "artist" => {
            if r.op == "not_contains" {
                cmp_text(&t.artist, &r.op, v) && cmp_text(&a.artist, &r.op, v)
            } else {
                cmp_text(&t.artist, &r.op, v) || cmp_text(&a.artist, &r.op, v)
            }
        }
        "album" => cmp_text(&a.name, &r.op, v),
        "codec" => {
            let same = Codecs::short(&t.codec, &t.file_path)
                .eq_ignore_ascii_case(&Codecs::short(v, ""));
            if r.op == "is_not" { !same } else { same }
        }
        "bits" => cmp_number(t.bits_per_sample as f64, &r.op, v, 0.001),
        "rate" => cmp_number(t.sample_rate as f64 / 1000.0, &r.op, v, 0.05),
        "year" => cmp_number(a.year as f64, &r.op, v, 0.001),
        "duration" => cmp_number(t.duration / 60.0, &r.op, v, 0.001),
        "plays" => cmp_number(stats(&t.id).map(|s| s.count).unwrap_or(0) as f64, &r.op, v, 0.001),
        "played" => {
            let last = stats(&t.id).map(|s| s.last).unwrap_or(0);
            if r.op == "never" {
                last == 0
            } else {
                within(last, now_ms, &r.op, v)
            }
        }
        "added" => within(a.added_at, now_ms, &r.op, v),
        "liked" => liked(&t.id) == (r.op == "yes"),
        _ => false,
    }
}

fn cmp_text(field: &str, op: &str, v: &str) -> bool {
    let f = field.to_lowercase();
    let needle = v.to_lowercase();
    match op {
        "contains" => f.contains(&needle),
        "not_contains" => !f.contains(&needle),
        "is" => f.trim() == needle,
        _ => false,
    }
}

fn cmp_number(field: f64, op: &str, v: &str, tolerance: f64) -> bool {
    let Ok(n) = v.replace(',', ".").parse::<f64>() else {
        return false;
    };
    match op {
        "gte" => field >= n - tolerance,
        "lte" => field <= n + tolerance,
        "eq" => (field - n).abs() <= tolerance,
        _ => false,
    }
}

fn within(when_ms: i64, now_ms: i64, op: &str, v: &str) -> bool {
    let Ok(days) = v.replace(',', ".").parse::<f64>() else {
        return false;
    };
    let inside = when_ms > 0 && ((now_ms - when_ms) as f64) <= days * 86_400_000.0;
    if op == "within" { inside } else { !inside }
}

/// Short format names (`FLAC`, `MP3`, `AAC`...) from codec descriptions or file extensions.
pub struct Codecs;

impl Codecs {
    pub fn short(codec: &str, file_path: &str) -> String {
        if codec.is_empty() {
            let ext = Path::new(file_path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            return if ext.is_empty() {
                "AUDIO".to_owned()
            } else {
                ext.to_ascii_uppercase()
            };
        }
        let c = codec.to_ascii_uppercase();
        if c.contains("MPEG") || c == "MP3" {
            return "MP3".to_owned();
        }
        if c.contains("VORBIS") || c == "OGG" {
            return "OGG".to_owned();
        }
        if c.contains("PCM") || c == "WAV" {
            return "WAV".to_owned();
        }
        if c.contains("WINDOWS MEDIA") {
            return "WMA".to_owned();
        }
        if c.contains("ALAC") || c.contains("APPLE LOSSLESS") {
            return "ALAC".to_owned();
        }
        if c.contains("AAC") {
            return "AAC".to_owned();
        }
        if c.contains("OPUS") {
            return "OPUS".to_owned();
        }
        if c.contains("FLAC") {
            return "FLAC".to_owned();
        }
        let ext = Path::new(file_path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if !ext.is_empty() {
            return ext.to_ascii_uppercase();
        }
        if c.len() > 8 {
            c[..8].trim().to_owned()
        } else {
            c
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_track(id: &str, bits: i32, rate: i32, codec: &str) -> (Track, Album) {
        let track = Track {
            id: id.to_owned(),
            title: format!("Track {id}"),
            artist: "Ado".to_owned(),
            album: "Kyogen".to_owned(),
            duration: 210.0,
            file_path: format!("/music/{id}.flac"),
            track_number: 1,
            disc_number: 1,
            codec: codec.to_owned(),
            sample_rate: rate,
            bits_per_sample: bits,
            channels: 2,
            cover_thumb_path: None,
            cover_full_path: None,
        };
        let album = Album {
            name: "Kyogen".to_owned(),
            artist: "Ado".to_owned(),
            cover_thumb_path: None,
            cover_full_path: None,
            year: 2022,
            added_at: 1_700_000_000_000,
            tracks: vec![track.clone()],
        };
        (track, album)
    }

    #[test]
    fn evaluates_hires_and_codec_rules() {
        let (t1, a1) = sample_track("1", 16, 44100, "FLAC");
        let (t2, a2) = sample_track("2", 24, 96000, "FLAC");
        let pl = SmartPlaylistData {
            id: 42,
            name: "Hi-Res".to_owned(),
            match_all: true,
            rules: vec![
                SmartRule {
                    field: "bits".to_owned(),
                    op: "gte".to_owned(),
                    value: "24".to_owned(),
                },
                SmartRule {
                    field: "rate".to_owned(),
                    op: "gte".to_owned(),
                    value: "96".to_owned(),
                },
                SmartRule {
                    field: "codec".to_owned(),
                    op: "is".to_owned(),
                    value: "flac".to_owned(),
                },
            ],
            ..SmartPlaylistData::default()
        };
        let hits = SmartPlaylists::evaluate(
            &pl,
            [(&t1, &a1), (&t2, &a2)],
            |_| None,
            |_| false,
            1_700_100_000_000,
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "2");
    }
}
