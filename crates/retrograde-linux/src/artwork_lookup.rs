use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static CACHE_TMP_SEQ: AtomicU64 = AtomicU64::new(1);

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::debug;

const USER_AGENT: &str = concat!(
    "Retrograde/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/Okarin2507/rusty-retrograde)"
);

const CACHE_VERSION: u32 = 1;
const HIT_TTL_SECS: u64 = 30 * 86_400;
const MISS_TTL_SECS: u64 = 86_400;
const TRANSIENT_BACKOFF: Duration = Duration::from_secs(60);
const MAX_CACHE_ENTRIES: usize = 1000;
const MAX_HTTP_BODY_BYTES: u64 = 256 * 1024;
const HTTP_TIMEOUT: Duration = Duration::from_secs(4);
const MUSICBRAINZ_MIN_GAP: Duration = Duration::from_millis(1100);
const MAX_DISCORD_IMAGE_LEN: usize = 256;

const ALLOWED_ARTWORK_HOSTS: &[&str] = &["mzstatic.com", "coverartarchive.org", "archive.org"];

const SINGLE_WORD_EDITION_KEYWORDS: &[&str] = &[
    "disc",
    "cd",
    "deluxe",
    "edition",
    "remaster",
    "remastered",
    "version",
    "flac",
    "mp3",
    "aac",
    "wav",
    "alac",
    "bit",
    "khz",
    "explicit",
    "clean",
    "bonus",
    "complete",
    "expanded",
    "special",
    "limited",
    "anniversary",
    "standard",
];

const AMBIGUOUS_ALBUM_ONLY_TITLES: &[&str] = &[
    "greatest hits",
    "best of",
    "soundtrack",
    "original soundtrack",
    "ost",
    "anthology",
    "collection",
    "compilation",
    "live",
    "unplugged",
    "demos",
    "b sides",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtworkQuery {
    pub artist: String,
    pub album: String,
    pub title: String,
    pub use_album: bool,
    pub cache_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupOutcome {
    Found(String),
    NotFound,
    TransientError,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheLookup {
    Hit(Option<String>),
    Backoff,
    Miss,
}

pub trait ArtworkResolver: Send + Sync {
    fn resolve(&self, query: &ArtworkQuery, cancel: &AtomicBool) -> LookupOutcome;
}

impl ArtworkQuery {
    /// Builds a normalized lookup query and stable cache key from track tags.
    pub fn from_metadata(artist: &str, album: &str, title: &str) -> Option<Self> {
        let artist = collapse_spaces(artist);
        let album = collapse_spaces(album);
        let title = collapse_spaces(title);

        let has_album = !is_generic_album(&album);
        let has_artist = !is_generic_artist(&artist);
        let has_title = !title.is_empty();

        let (use_album, cache_key) = if has_album {
            let key = if has_artist {
                format!("album::{}::{}", norm_key(&artist), norm_key(&album))
            } else if is_ambiguous_album_only(&album) {
                return None;
            } else {
                format!("album_only::{}", norm_key(&album))
            };
            (true, key)
        } else if has_title && has_artist {
            (false, format!("track::{}::{}", norm_key(&artist), norm_key(&title)))
        } else {
            return None;
        };

        Some(Self {
            artist,
            album,
            title,
            use_album,
            cache_key,
        })
    }

    fn has_cjk(&self) -> bool {
        contains_cjk(&self.artist) || contains_cjk(&self.album) || contains_cjk(&self.title)
    }

    fn has_searchable_artist(&self) -> bool {
        !is_generic_artist(&self.artist)
    }
}

pub fn now_unix_u64() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

pub fn is_generic_album(s: &str) -> bool {
    let lower = collapse_spaces(s).to_lowercase();
    lower.is_empty()
        || lower.starts_with("unknown album")
        || matches!(
            lower.as_str(),
            "unknown"
                | "singles"
                | "single"
                | "misc"
                | "various"
                | "none"
                | "n/a"
                | "downloads"
                | "music"
        )
}

pub fn is_generic_artist(s: &str) -> bool {
    let lower = collapse_spaces(s).to_lowercase();
    lower.is_empty()
        || matches!(
            lower.as_str(),
            "unknown artist"
                | "unknown"
                | "various artists"
                | "various"
                | "v.a."
                | "va"
                | "none"
                | "n/a"
        )
}

fn is_ambiguous_album_only(album: &str) -> bool {
    let norm = normalize_for_match(album);
    norm.chars().count() < 4 || AMBIGUOUS_ALBUM_ONLY_TITLES.contains(&norm.as_str())
}

/// Extracts the leading artist name when tags list multiple collaborations or features.
pub fn primary_artist(artist: &str) -> &str {
    let trimmed = artist.trim();
    let lower = trimmed.to_ascii_lowercase();
    let seps = [";", " / ", " feat. ", " ft. ", " featuring ", " × "];
    let mut earliest: Option<usize> = None;
    for sep in seps {
        if let Some(idx) = lower.find(sep)
            && earliest.is_none_or(|cur| idx < cur)
        {
            earliest = Some(idx);
        }
    }
    match earliest {
        Some(idx) => {
            let head = trimmed[..idx].trim();
            if head.is_empty() { trimmed } else { head }
        }
        None => trimmed,
    }
}

/// Strips trailing edition/disc/format annotations for fallback album searches.
pub fn clean_album_title(album: &str) -> String {
    let mut current = collapse_spaces(album);
    loop {
        let trimmed = current.trim_end();
        if let Some(stripped) = strip_trailing_bracket_edition(trimmed, '(', ')')
            .or_else(|| strip_trailing_bracket_edition(trimmed, '[', ']'))
            .or_else(|| strip_trailing_disc_suffix(trimmed))
        {
            let next = stripped.trim().to_owned();
            if !next.is_empty() && next != current {
                current = next;
                continue;
            }
        }
        break;
    }
    current
}

fn strip_trailing_bracket_edition(s: &str, open: char, close: char) -> Option<&str> {
    if !s.ends_with(close) {
        return None;
    }
    let start = s.rfind(open)?;
    let inner = s[start + open.len_utf8()..s.len() - close.len_utf8()].to_lowercase();
    let words: Vec<&str> = inner
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let has_edition_word = words
        .iter()
        .any(|w| SINGLE_WORD_EDITION_KEYWORDS.contains(w))
        || words
            .windows(2)
            .any(|w| w == ["live", "at"] || w == ["hi", "res"]);
    if has_edition_word {
        Some(&s[..start])
    } else {
        None
    }
}

fn strip_trailing_disc_suffix(s: &str) -> Option<&str> {
    let lower = s.to_ascii_lowercase();
    for prefix in [" disc ", " cd "] {
        if let Some(idx) = lower.rfind(prefix) {
            let tail = lower[idx + prefix.len()..].trim();
            if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) {
                return Some(&s[..idx]);
            }
        }
    }
    None
}

pub fn contains_cjk(s: &str) -> bool {
    s.chars().any(|ch| {
        matches!(
            ch,
            '\u{1100}'..='\u{11ff}'
                | '\u{3040}'..='\u{30ff}'
                | '\u{3400}'..='\u{4dbf}'
                | '\u{4e00}'..='\u{9fff}'
                | '\u{ac00}'..='\u{d7af}'
                | '\u{ff65}'..='\u{ff9f}'
        )
    })
}

/// Validates that a remote artwork URL is safe, HTTPS, within Discord's 256-char limit,
/// and hosted on one of the expected CDN domains.
pub fn validate_artwork_url(url: &str) -> Option<String> {
    let trimmed = url.trim();
    if trimmed.len() > MAX_DISCORD_IMAGE_LEN || !trimmed.starts_with("https://") {
        return None;
    }
    if !trimmed.bytes().all(|b| matches!(b, 0x21..=0x7e)) {
        return None;
    }
    let after_scheme = &trimmed["https://".len()..];
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    if authority.is_empty() || authority.contains(['@', '\\', '%']) {
        return None;
    }
    let host = authority
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let allowed = ALLOWED_ARTWORK_HOSTS
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")));
    if !allowed {
        return None;
    }
    Some(trimmed.to_owned())
}

pub fn upgrade_itunes_artwork_url(raw: &str) -> Option<String> {
    let upgraded = if raw.contains("/100x100bb") {
        raw.replacen("/100x100bb", "/512x512bb", 1)
    } else if raw.contains("/60x60bb") {
        raw.replacen("/60x60bb", "/512x512bb", 1)
    } else {
        raw.to_owned()
    };
    validate_artwork_url(&upgraded)
}

pub fn is_valid_mbid(id: &str) -> bool {
    if id.len() != 36 {
        return false;
    }
    id.char_indices().all(|(idx, ch)| match idx {
        8 | 13 | 18 | 23 => ch == '-',
        _ => ch.is_ascii_hexdigit(),
    })
}

pub fn escape_lucene(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(
            ch,
            '+' | '-'
                | '&'
                | '|'
                | '!'
                | '('
                | ')'
                | '{'
                | '}'
                | '['
                | ']'
                | '^'
                | '"'
                | '~'
                | '*'
                | '?'
                | ':'
                | '\\'
                | '/'
        ) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn collapse_spaces(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn norm_key(s: &str) -> String {
    collapse_spaces(s).to_lowercase()
}

fn normalize_for_match(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = true;
    for ch in s.chars() {
        if matches!(ch, '\'' | '\u{2018}' | '\u{2019}') {
            continue;
        }
        for lc in ch.to_lowercase() {
            if lc.is_alphanumeric() {
                out.push(lc);
                last_space = false;
            } else if !last_space {
                out.push(' ');
                last_space = true;
            }
        }
    }
    out.trim().to_owned()
}

fn token_subsequence_match(haystack: &str, needle: &str) -> bool {
    let h: Vec<&str> = haystack.split_whitespace().collect();
    let n: Vec<&str> = needle.split_whitespace().collect();
    !n.is_empty() && h.len() >= n.len() && h.windows(n.len()).any(|w| w == n.as_slice())
}

fn string_similarity_score(
    candidate_raw: &str,
    targets: &[&str],
    exact_pts: i32,
    partial_pts: i32,
) -> (i32, bool) {
    let cand = normalize_for_match(candidate_raw);
    if cand.is_empty() {
        return (0, false);
    }
    let mut best = 0;
    for target in targets {
        let t = normalize_for_match(target);
        if t.is_empty() {
            continue;
        }
        if cand == t {
            return (exact_pts, true);
        }
        if token_subsequence_match(&cand, &t) || token_subsequence_match(&t, &cand) {
            best = best.max(partial_pts);
        } else if contains_cjk(&cand) && contains_cjk(&t) {
            let (shorter, longer) = if cand.chars().count() <= t.chars().count() {
                (&cand, &t)
            } else {
                (&t, &cand)
            };
            if shorter.chars().count() >= 2 && longer.contains(shorter.as_str()) {
                best = best.max(partial_pts);
            }
        }
    }
    (best, false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItunesEntity {
    Album,
    Song,
}

impl ItunesEntity {
    fn as_str(self) -> &'static str {
        match self {
            Self::Album => "album",
            Self::Song => "song",
        }
    }
}

fn is_album_only_candidate_safe(cand_artist: &str, query_album: &str) -> bool {
    let a = normalize_for_match(cand_artist);
    if matches!(
        a.as_str(),
        "various artists" | "soundtrack" | "original soundtrack" | "va"
    ) {
        return true;
    }
    let q = normalize_for_match(query_album);
    q.chars().count() >= 10 || q.split_whitespace().count() >= 2
}

fn pick_best_itunes_artwork(
    results: &[Value],
    query: &ArtworkQuery,
    entity: ItunesEntity,
) -> Option<String> {
    let cleaned_album = clean_album_title(&query.album);
    let has_artist = query.has_searchable_artist();
    let artist_targets = [query.artist.as_str(), primary_artist(&query.artist)];
    let album_targets = [query.album.as_str(), cleaned_album.as_str()];
    let title_targets = [query.title.as_str()];

    let mut best: Option<(i32, String)> = None;

    for (idx, item) in results.iter().enumerate() {
        let Some(raw_art) = item.get("artworkUrl100").and_then(Value::as_str) else {
            continue;
        };
        let Some(art_url) = upgrade_itunes_artwork_url(raw_art) else {
            continue;
        };

        let cand_artist = item.get("artistName").and_then(Value::as_str).unwrap_or("");
        let cand_album = item
            .get("collectionName")
            .and_then(Value::as_str)
            .unwrap_or("");
        let cand_track = item.get("trackName").and_then(Value::as_str).unwrap_or("");

        let (album_pts, album_exact) = if query.use_album {
            string_similarity_score(cand_album, &album_targets, 70, 45)
        } else {
            (0, false)
        };

        let (primary_match_pts, exact_primary, allow_cjk_fallback, matched_raw) = match entity {
            ItunesEntity::Album => (album_pts, album_exact, contains_cjk(&query.album), cand_album),
            ItunesEntity::Song => {
                let (track_pts, track_exact) =
                    string_similarity_score(cand_track, &title_targets, 65, 40);
                let bonus = if query.use_album && album_pts > 0 {
                    15
                } else {
                    0
                };
                (
                    track_pts + bonus,
                    track_exact,
                    contains_cjk(&query.title),
                    cand_track,
                )
            }
        };

        let artist_score = if has_artist {
            let (pts, _) = string_similarity_score(cand_artist, &artist_targets, 60, 35);
            if pts > 0 {
                pts
            } else {
                let cross_script = contains_cjk(&query.artist) != contains_cjk(cand_artist);
                let norm_matched = normalize_for_match(matched_raw);
                let min_len = if contains_cjk(&norm_matched) { 2 } else { 4 };
                if cross_script
                    && exact_primary
                    && norm_matched.chars().count() >= min_len
                {
                    -15
                } else {
                    continue;
                }
            }
        } else {
            if !album_exact || !is_album_only_candidate_safe(cand_artist, &query.album) {
                continue;
            }
            0
        };

        if primary_match_pts == 0 && !(allow_cjk_fallback && artist_score > 0 && idx < 2) {
            continue;
        }

        let cjk_rank_bonus = if query.has_cjk() && artist_score > 0 {
            match idx {
                0 => 20,
                1 => 10,
                _ => 0,
            }
        } else {
            0
        };

        let total = artist_score + primary_match_pts + cjk_rank_bonus;
        if total >= 50 && best.as_ref().is_none_or(|(s, _)| total > *s) {
            best = Some((total, art_url));
        }
    }

    best.map(|(_, url)| url)
}

/// Online metadata-based album artwork resolver (iTunes Search API -> MusicBrainz + CAA).
pub struct OnlineArtworkResolver {
    agent: ureq::Agent,
    mb_last_call: Mutex<Option<Instant>>,
}

impl Default for OnlineArtworkResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl OnlineArtworkResolver {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(HTTP_TIMEOUT))
            .https_only(true)
            .max_redirects(3)
            .http_status_as_error(false)
            .user_agent(USER_AGENT)
            .build();
        Self {
            agent: config.new_agent(),
            mb_last_call: Mutex::new(None),
        }
    }

    fn search_itunes(
        &self,
        term: &str,
        entity: ItunesEntity,
        country: Option<&str>,
        query: &ArtworkQuery,
        cancel: &AtomicBool,
        had_transient_error: &mut bool,
    ) -> Option<String> {
        if cancel.load(Ordering::Relaxed) {
            *had_transient_error = true;
            return None;
        }
        let mut req = self
            .agent
            .get("https://itunes.apple.com/search")
            .query("term", term)
            .query("entity", entity.as_str())
            .query("limit", "8");
        if let Some(c) = country {
            req = req.query("country", c);
        }
        let mut resp = match req.call() {
            Ok(r) => r,
            Err(err) => {
                debug!(error = %err, "iTunes artwork lookup transport error");
                *had_transient_error = true;
                return None;
            }
        };
        let status = resp.status().as_u16();
        if status == 403 || status == 429 || status >= 500 {
            debug!(status, "iTunes artwork lookup transient status");
            *had_transient_error = true;
            return None;
        }
        if status != 200 {
            return None;
        }
        let Ok(bytes) = resp
            .body_mut()
            .with_config()
            .limit(MAX_HTTP_BODY_BYTES)
            .read_to_vec()
        else {
            *had_transient_error = true;
            return None;
        };
        let Ok(val) = serde_json::from_slice::<Value>(&bytes) else {
            return None;
        };
        let results = val.get("results").and_then(Value::as_array)?;
        pick_best_itunes_artwork(results, query, entity)
    }

    fn try_itunes(
        &self,
        query: &ArtworkQuery,
        cancel: &AtomicBool,
        had_transient_error: &mut bool,
    ) -> Option<String> {
        let artist_q = if query.has_searchable_artist() {
            Some(primary_artist(&query.artist))
        } else {
            None
        };

        if query.use_album {
            let term = match artist_q {
                Some(a) => format!("{a} {}", query.album),
                None => query.album.clone(),
            };
            if let Some(url) = self.search_itunes(
                &term,
                ItunesEntity::Album,
                None,
                query,
                cancel,
                had_transient_error,
            ) {
                return Some(url);
            }
            if *had_transient_error {
                return None;
            }

            let cleaned = clean_album_title(&query.album);
            if cleaned != query.album {
                let cleaned_term = match artist_q {
                    Some(a) => format!("{a} {cleaned}"),
                    None => cleaned,
                };
                if let Some(url) = self.search_itunes(
                    &cleaned_term,
                    ItunesEntity::Album,
                    None,
                    query,
                    cancel,
                    had_transient_error,
                ) {
                    return Some(url);
                }
                if *had_transient_error {
                    return None;
                }
            }

            if query.has_cjk()
                && let Some(url) = self.search_itunes(
                    &term,
                    ItunesEntity::Album,
                    Some("jp"),
                    query,
                    cancel,
                    had_transient_error,
                )
            {
                return Some(url);
            }
            if *had_transient_error {
                return None;
            }
        }

        if !query.title.is_empty() && query.has_searchable_artist() {
            let song_term = match artist_q {
                Some(a) => format!("{a} {}", query.title),
                None => query.title.clone(),
            };
            if let Some(url) = self.search_itunes(
                &song_term,
                ItunesEntity::Song,
                None,
                query,
                cancel,
                had_transient_error,
            ) {
                return Some(url);
            }
            if !*had_transient_error
                && query.has_cjk()
                && let Some(url) = self.search_itunes(
                    &song_term,
                    ItunesEntity::Song,
                    Some("jp"),
                    query,
                    cancel,
                    had_transient_error,
                )
            {
                return Some(url);
            }
        }

        None
    }

    fn wait_musicbrainz_rate_limit(&self, cancel: &AtomicBool) -> bool {
        let mut guard = self
            .mb_last_call
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(prev) = *guard {
            while prev.elapsed() < MUSICBRAINZ_MIN_GAP {
                if cancel.load(Ordering::Relaxed) {
                    return false;
                }
                let rem = MUSICBRAINZ_MIN_GAP.saturating_sub(prev.elapsed());
                std::thread::sleep(rem.min(Duration::from_millis(50)));
            }
        }
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        *guard = Some(Instant::now());
        true
    }

    fn try_musicbrainz_caa(
        &self,
        query: &ArtworkQuery,
        cancel: &AtomicBool,
        had_transient_error: &mut bool,
    ) -> Option<String> {
        if !query.use_album {
            return None;
        }
        if !self.wait_musicbrainz_rate_limit(cancel) {
            *had_transient_error = true;
            return None;
        }

        let cleaned_album = clean_album_title(&query.album);
        let escaped_album = escape_lucene(&cleaned_album);
        let has_artist = query.has_searchable_artist();
        let lucene_q = if has_artist {
            let escaped_artist = escape_lucene(primary_artist(&query.artist));
            format!("releasegroup:\"{escaped_album}\" AND artist:\"{escaped_artist}\"")
        } else {
            format!("releasegroup:\"{escaped_album}\"")
        };

        let mut resp = match self
            .agent
            .get("https://musicbrainz.org/ws/2/release-group/")
            .query("query", &lucene_q)
            .query("fmt", "json")
            .query("limit", "5")
            .call()
        {
            Ok(r) => r,
            Err(err) => {
                debug!(error = %err, "MusicBrainz lookup transport error");
                *had_transient_error = true;
                return None;
            }
        };

        let status = resp.status().as_u16();
        if matches!(status, 400 | 404) {
            debug!(status, "MusicBrainz lookup non-retryable status");
            return None;
        }
        if status != 200 {
            debug!(status, "MusicBrainz lookup transient status");
            *had_transient_error = true;
            return None;
        }

        let Ok(bytes) = resp
            .body_mut()
            .with_config()
            .limit(MAX_HTTP_BODY_BYTES)
            .read_to_vec()
        else {
            *had_transient_error = true;
            return None;
        };
        let Ok(val) = serde_json::from_slice::<Value>(&bytes) else {
            return None;
        };
        let groups = val.get("release-groups").and_then(Value::as_array)?;

        let min_score = if has_artist { 80 } else { 95 };
        let album_norms = [
            normalize_for_match(&query.album),
            normalize_for_match(&cleaned_album),
        ];

        for item in groups
            .iter()
            .filter(|g| g.get("score").and_then(Value::as_i64).unwrap_or(0) >= min_score)
            .take(3)
        {
            if !has_artist {
                let title_norm = normalize_for_match(
                    item.get("title").and_then(Value::as_str).unwrap_or(""),
                );
                if title_norm.is_empty() || !album_norms.contains(&title_norm) {
                    continue;
                }
            }
            if cancel.load(Ordering::Relaxed) {
                *had_transient_error = true;
                return None;
            }
            let Some(mbid) = item.get("id").and_then(Value::as_str) else {
                continue;
            };
            if !is_valid_mbid(mbid) {
                continue;
            }
            let caa_url = format!("https://coverartarchive.org/release-group/{mbid}/front-500");
            let head_resp = match self
                .agent
                .head(&caa_url)
                .config()
                .max_redirects(0)
                .max_redirects_will_error(false)
                .build()
                .call()
            {
                Ok(r) => r,
                Err(err) => {
                    debug!(error = %err, "Cover Art Archive HEAD error");
                    *had_transient_error = true;
                    break;
                }
            };
            let head_status = head_resp.status().as_u16();
            if matches!(head_status, 301 | 302 | 303 | 307 | 308) {
                if let Some(loc) = head_resp.headers().get("location").and_then(|v| v.to_str().ok())
                {
                    let normalized = if let Some(rest) = loc.strip_prefix("http://") {
                        format!("https://{rest}")
                    } else {
                        loc.to_owned()
                    };
                    if let Some(valid) = validate_artwork_url(&normalized) {
                        return Some(valid);
                    }
                }
                if let Some(valid) = validate_artwork_url(&caa_url) {
                    return Some(valid);
                }
            } else if head_status == 200 {
                if let Some(valid) = validate_artwork_url(&caa_url) {
                    return Some(valid);
                }
            } else if head_status == 404 {
                continue;
            } else {
                *had_transient_error = true;
                break;
            }
        }

        None
    }
}

impl ArtworkResolver for OnlineArtworkResolver {
    fn resolve(&self, query: &ArtworkQuery, cancel: &AtomicBool) -> LookupOutcome {
        if cancel.load(Ordering::Relaxed) {
            return LookupOutcome::TransientError;
        }
        let mut had_transient_error = false;
        if let Some(url) = self.try_itunes(query, cancel, &mut had_transient_error) {
            return LookupOutcome::Found(url);
        }
        if cancel.load(Ordering::Relaxed) {
            return LookupOutcome::TransientError;
        }
        if let Some(url) = self.try_musicbrainz_caa(query, cancel, &mut had_transient_error) {
            return LookupOutcome::Found(url);
        }
        if cancel.load(Ordering::Relaxed) || had_transient_error {
            LookupOutcome::TransientError
        } else {
            LookupOutcome::NotFound
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    entries: HashMap<String, CacheEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    url: Option<String>,
    fetched_at: u64,
}

pub struct ArtworkCache {
    path: Option<PathBuf>,
    entries: HashMap<String, CacheEntry>,
    transient_backoff: HashMap<String, Instant>,
}

impl ArtworkCache {
    pub fn load(path: Option<PathBuf>) -> Self {
        let now = now_unix_u64();
        let mut entries = HashMap::new();
        if let Some(ref p) = path
            && let Ok(text) = fs::read_to_string(p)
            && let Ok(file) = serde_json::from_str::<CacheFile>(&text)
            && file.version == CACHE_VERSION
        {
            for (k, mut entry) in file.entries {
                if let Some(ref raw_url) = entry.url {
                    let Some(valid) = validate_artwork_url(raw_url) else {
                        continue;
                    };
                    entry.url = Some(valid);
                }
                if is_entry_fresh(&entry, now) {
                    entries.insert(k, entry);
                }
            }
        }
        Self {
            path,
            entries,
            transient_backoff: HashMap::new(),
        }
    }

    pub fn get(&self, key: &str, now_unix: u64, now_inst: Instant) -> CacheLookup {
        if let Some(entry) = self.entries.get(key)
            && is_entry_fresh(entry, now_unix)
        {
            return CacheLookup::Hit(entry.url.clone());
        }
        if let Some(failed_at) = self.transient_backoff.get(key)
            && now_inst.saturating_duration_since(*failed_at) < TRANSIENT_BACKOFF
        {
            return CacheLookup::Backoff;
        }
        CacheLookup::Miss
    }

    pub fn record(
        &mut self,
        key: String,
        outcome: &LookupOutcome,
        now_unix: u64,
        now_inst: Instant,
    ) {
        self.transient_backoff.retain(|_, failed_at| {
            now_inst.saturating_duration_since(*failed_at) < TRANSIENT_BACKOFF
        });
        match outcome {
            LookupOutcome::Found(url) => {
                self.transient_backoff.remove(&key);
                let Some(valid) = validate_artwork_url(url) else {
                    return;
                };
                self.entries.insert(
                    key,
                    CacheEntry {
                        url: Some(valid),
                        fetched_at: now_unix,
                    },
                );
                self.prune_and_persist(now_unix);
            }
            LookupOutcome::NotFound => {
                self.transient_backoff.remove(&key);
                self.entries.insert(
                    key,
                    CacheEntry {
                        url: None,
                        fetched_at: now_unix,
                    },
                );
                self.prune_and_persist(now_unix);
            }
            LookupOutcome::TransientError => {
                self.transient_backoff.insert(key, now_inst);
            }
        }
    }

    fn prune_and_persist(&mut self, now_unix: u64) {
        self.entries
            .retain(|_, entry| is_entry_fresh(entry, now_unix));
        while self.entries.len() > MAX_CACHE_ENTRIES {
            let Some(oldest_key) = self
                .entries
                .iter()
                .min_by(|(k1, e1), (k2, e2)| {
                    e1.fetched_at.cmp(&e2.fetched_at).then_with(|| k1.cmp(k2))
                })
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            self.entries.remove(&oldest_key);
        }

        let Some(ref path) = self.path else {
            return;
        };
        if let Some(parent) = path.parent()
            && fs::create_dir_all(parent).is_err()
        {
            return;
        }
        let file = CacheFile {
            version: CACHE_VERSION,
            entries: self.entries.clone(),
        };
        let seq = CACHE_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = path.with_extension(format!("json.{}.{seq}.tmp", std::process::id()));
        if let Ok(json) = serde_json::to_string(&file)
            && fs::write(&tmp, json).is_ok()
        {
            let _ = fs::rename(&tmp, path);
        }
    }
}

fn is_entry_fresh(entry: &CacheEntry, now_unix: u64) -> bool {
    let ttl = if entry.url.is_some() {
        HIT_TTL_SECS
    } else {
        MISS_TTL_SECS
    };
    now_unix.saturating_sub(entry.fetched_at) < ttl
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_cache_path(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "rg-art-cache-{}-{tag}-{}.json",
            std::process::id(),
            now_unix_u64()
        ));
        let _ = fs::remove_file(&p);
        p
    }

    #[test]
    fn detects_generic_tags_and_builds_expected_cache_keys() {
        assert!(is_generic_album(""));
        assert!(is_generic_album("Unknown Album"));
        assert!(is_generic_album("Unknown Album (Ado)"));
        assert!(is_generic_album("  SINGLES "));
        assert!(!is_generic_album("Kyougen"));

        assert!(is_generic_artist("Various Artists"));
        assert!(is_generic_artist("Unknown Artist"));
        assert!(!is_generic_artist("Ado"));

        let q_album = ArtworkQuery::from_metadata("Ado", "  Kyougen ", "Usseewa").unwrap();
        assert!(q_album.use_album);
        assert_eq!(q_album.cache_key, "album::ado::kyougen");

        let q_track =
            ArtworkQuery::from_metadata("Ado", "Unknown Album (Ado)", "Usseewa").unwrap();
        assert!(!q_track.use_album);
        assert_eq!(q_track.cache_key, "track::ado::usseewa");

        let q_va = ArtworkQuery::from_metadata("Various Artists", "Arcane OST", "Enemy").unwrap();
        assert!(q_va.use_album);
        assert_eq!(q_va.cache_key, "album_only::arcane ost");

        assert!(ArtworkQuery::from_metadata("Unknown Artist", "Unknown Album", "").is_none());
        assert!(ArtworkQuery::from_metadata("Unknown Artist", "Unknown Album", "Intro").is_none());
        assert!(ArtworkQuery::from_metadata("Various Artists", "Greatest Hits", "Track 1").is_none());
    }

    #[test]
    fn normalizes_primary_artist_and_cleans_album_edition_tags() {
        assert_eq!(primary_artist("Ado; B'z"), "Ado");
        assert_eq!(primary_artist("Robert del Naja feat. Someone"), "Robert del Naja");
        assert_eq!(primary_artist("Simon & Garfunkel"), "Simon & Garfunkel");
        assert_eq!(primary_artist("SawanoHiroyuki[nZk]"), "SawanoHiroyuki[nZk]");

        assert_eq!(clean_album_title("Kyougen (Deluxe Edition) [FLAC]"), "Kyougen");
        assert_eq!(clean_album_title("Hibana Disc 1"), "Hibana");
        assert_eq!(clean_album_title("Lone Trail (Part 2)"), "Lone Trail (Part 2)");
        assert_eq!(clean_album_title("X (Orbital)"), "X (Orbital)");
        assert_eq!(clean_album_title("Daft Punk (Discovery)"), "Daft Punk (Discovery)");
        assert_eq!(clean_album_title("Test (ABCD)"), "Test (ABCD)");
    }

    #[test]
    fn validates_artwork_urls_and_upgrades_itunes_size() {
        let upgraded = upgrade_itunes_artwork_url(
            "https://is1-ssl.mzstatic.com/image/thumb/Music126/v4/24/e9/c2/21UM1IM54074.rgb.jpg/100x100bb.jpg",
        )
        .unwrap();
        assert!(upgraded.ends_with("/512x512bb.jpg"));

        assert!(validate_artwork_url(
            "https://archive.org/download/mbid-123/thumb500.jpg"
        ).is_some());
        assert!(validate_artwork_url(
            "http://is1-ssl.mzstatic.com/image/thumb/x/512x512bb.jpg"
        ).is_none());
        assert!(validate_artwork_url("https://evil.example.com/cover.jpg").is_none());
        assert!(validate_artwork_url("https://user@is1-ssl.mzstatic.com/a.jpg").is_none());
        assert!(validate_artwork_url("https://evil.com\\.mzstatic.com/x.jpg").is_none());
        assert!(validate_artwork_url("https://evil.com%2e.mzstatic.com/x.jpg").is_none());
        assert!(validate_artwork_url(
            &format!("https://is1-ssl.mzstatic.com/{}.jpg", "a".repeat(240))
        ).is_none());

        assert!(is_valid_mbid("aa997ea0-2936-40bd-884d-3af8a0e064dc"));
        assert!(!is_valid_mbid("not-a-uuid"));
        assert_eq!(
            escape_lucene("Ado: (Live!) [2025]"),
            "Ado\\: \\(Live\\!\\) \\[2025\\]"
        );
    }

    #[test]
    fn itunes_scoring_picks_exact_handles_cjk_and_rejects_wrong_artist() {
        let q_exact = ArtworkQuery::from_metadata("Ado", "Kyougen", "Usseewa").unwrap();
        let results = vec![
            json!({
                "artistName": "Other Artist",
                "collectionName": "Random",
                "artworkUrl100": "https://is1-ssl.mzstatic.com/a/100x100bb.jpg",
            }),
            json!({
                "artistName": "ADO",
                "collectionName": "Kyougen",
                "artworkUrl100": "https://is1-ssl.mzstatic.com/b/100x100bb.jpg",
            }),
        ];
        assert_eq!(
            pick_best_itunes_artwork(&results, &q_exact, ItunesEntity::Album).as_deref(),
            Some("https://is1-ssl.mzstatic.com/b/512x512bb.jpg")
        );

        let q_cjk = ArtworkQuery::from_metadata("Ado", "狂言", "うっせぇわ").unwrap();
        let cjk_results = vec![json!({
            "artistName": "ADO",
            "collectionName": "Kyougen",
            "artworkUrl100": "https://is1-ssl.mzstatic.com/c/100x100bb.jpg",
        })];
        assert_eq!(
            pick_best_itunes_artwork(&cjk_results, &q_cjk, ItunesEntity::Album).as_deref(),
            Some("https://is1-ssl.mzstatic.com/c/512x512bb.jpg")
        );

        // Short artist name must NOT match inside unrelated Latin words (e.g. "Ado" in "Shadow").
        let q_short = ArtworkQuery::from_metadata("Ado", "Shadow", "Shadow").unwrap();
        let substring_trap = vec![json!({
            "artistName": "Shadow",
            "collectionName": "Shadow",
            "artworkUrl100": "https://is1-ssl.mzstatic.com/trap/100x100bb.jpg",
        })];
        assert!(
            pick_best_itunes_artwork(&substring_trap, &q_short, ItunesEntity::Album).is_none()
        );

        // Whole-token feature match ("Ado" in "Ado feat. Miku") still succeeds.
        let token_feat = vec![json!({
            "artistName": "Ado feat. Miku",
            "collectionName": "Kyougen",
            "artworkUrl100": "https://is1-ssl.mzstatic.com/feat/100x100bb.jpg",
        })];
        assert_eq!(
            pick_best_itunes_artwork(&token_feat, &q_exact, ItunesEntity::Album).as_deref(),
            Some("https://is1-ssl.mzstatic.com/feat/512x512bb.jpg")
        );

        // Cross-script artist (kana/kanji vs romanized) with exact non-trivial album succeeds.
        let q_cross = ArtworkQuery::from_metadata("ヨルシカ", "Elma", "Rain with Cappuccino").unwrap();
        let cross_hit = vec![json!({
            "artistName": "Yorushika",
            "collectionName": "Elma",
            "artworkUrl100": "https://is1-ssl.mzstatic.com/elma/100x100bb.jpg",
        })];
        assert_eq!(
            pick_best_itunes_artwork(&cross_hit, &q_cross, ItunesEntity::Album).as_deref(),
            Some("https://is1-ssl.mzstatic.com/elma/512x512bb.jpg")
        );

        // Same-script wrong artist with exact same album title is strictly rejected.
        let q_same_script = ArtworkQuery::from_metadata("Taylor Swift", "Red", "Red").unwrap();
        let wrong_artist_exact_album = vec![json!({
            "artistName": "King Crimson",
            "collectionName": "Red",
            "artworkUrl100": "https://is1-ssl.mzstatic.com/kc/100x100bb.jpg",
        })];
        assert!(
            pick_best_itunes_artwork(&wrong_artist_exact_album, &q_same_script, ItunesEntity::Album)
                .is_none()
        );
    }

    #[test]
    fn cache_persists_hits_misses_respects_ttl_and_skips_transient_errors() {
        let path = temp_cache_path("roundtrip");
        let now_unix = now_unix_u64();
        let now_inst = Instant::now();
        let url = "https://is1-ssl.mzstatic.com/image/thumb/a/512x512bb.jpg".to_owned();

        let mut cache = ArtworkCache::load(Some(path.clone()));
        cache.record(
            "album::ado::kyougen".to_owned(),
            &LookupOutcome::Found(url.clone()),
            now_unix,
            now_inst,
        );
        cache.record(
            "album::x::missing".to_owned(),
            &LookupOutcome::NotFound,
            now_unix,
            now_inst,
        );
        cache.record(
            "album::x::offline".to_owned(),
            &LookupOutcome::TransientError,
            now_unix,
            now_inst,
        );

        assert_eq!(
            cache.get("album::x::offline", now_unix, now_inst),
            CacheLookup::Backoff
        );
        assert_eq!(
            cache.get(
                "album::x::offline",
                now_unix,
                now_inst + Duration::from_secs(61)
            ),
            CacheLookup::Miss
        );

        let reloaded = ArtworkCache::load(Some(path.clone()));
        assert_eq!(
            reloaded.get("album::ado::kyougen", now_unix, now_inst),
            CacheLookup::Hit(Some(url))
        );
        assert_eq!(
            reloaded.get("album::x::missing", now_unix, now_inst),
            CacheLookup::Hit(None)
        );
        // Transient errors are not persisted across reloads.
        assert_eq!(
            reloaded.get("album::x::offline", now_unix, now_inst),
            CacheLookup::Miss
        );
        // Negative hit expires after 24h while positive hit remains valid up to 30d.
        assert_eq!(
            reloaded.get("album::x::missing", now_unix + MISS_TTL_SECS + 1, now_inst),
            CacheLookup::Miss
        );
        assert!(matches!(
            reloaded.get("album::ado::kyougen", now_unix + MISS_TTL_SECS + 1, now_inst),
            CacheLookup::Hit(Some(_))
        ));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn cache_evicts_oldest_entries_past_limit() {
        let path = temp_cache_path("evict");
        let base_unix = now_unix_u64();
        let now_inst = Instant::now();
        let mut cache = ArtworkCache::load(Some(path.clone()));

        for i in 0..1005u64 {
            cache.entries.insert(
                format!("k::{i}"),
                CacheEntry {
                    url: Some(format!(
                        "https://is1-ssl.mzstatic.com/image/thumb/{i}/512x512bb.jpg"
                    )),
                    fetched_at: base_unix.saturating_sub(1005 - i),
                },
            );
        }
        cache.record(
            "k::1005".to_owned(),
            &LookupOutcome::Found(
                "https://is1-ssl.mzstatic.com/image/thumb/1005/512x512bb.jpg".to_owned(),
            ),
            base_unix,
            now_inst,
        );

        assert_eq!(cache.entries.len(), MAX_CACHE_ENTRIES);
        assert!(!cache.entries.contains_key("k::0"));
        assert!(!cache.entries.contains_key("k::5"));
        assert!(cache.entries.contains_key("k::6"));
        assert!(cache.entries.contains_key("k::1005"));

        let _ = fs::remove_file(&path);
    }
}
