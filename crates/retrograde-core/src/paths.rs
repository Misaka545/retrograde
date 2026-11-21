use std::path::{Path, PathBuf};

/// Cover cache naming and app directory resolution shared with the Electron & C# builds
/// (`~/.retrograde/covers/{hash}_thumb.jpg`).
pub struct CoverPaths;

impl CoverPaths {
    /// `~/.retrograde`, or `RETRO_APPDIR` when set and non-empty (tests run against a scratch copy).
    pub fn app_dir() -> PathBuf {
        if let Ok(dir) = std::env::var("RETRO_APPDIR")
            && !dir.is_empty()
        {
            return PathBuf::from(dir);
        }
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        home.join(".retrograde")
    }

    pub fn cover_dir() -> PathBuf {
        Self::app_dir().join("covers")
    }

    /// Bit-exact port of the JS `hashName()` / C# `CoverPaths.HashName()`:
    /// iterates UTF-16 code units (`encode_utf16`), updating
    /// `hash = ((hash << 5) - hash + unit)` in wrapping 32-bit signed arithmetic,
    /// then formats `abs(hash as i64)` in lowercase base-36.
    pub fn hash_name(s: &str) -> String {
        let mut hash: i32 = 0;
        for unit in s.encode_utf16() {
            hash = hash
                .wrapping_shl(5)
                .wrapping_sub(hash)
                .wrapping_add(unit as i32);
        }
        let abs = (hash as i64).abs();
        to_base36(abs)
    }

    pub fn thumb_path(album_name: &str) -> PathBuf {
        Self::cover_dir().join(format!("{}_thumb.jpg", Self::hash_name(album_name)))
    }

    pub fn full_path(album_name: &str) -> PathBuf {
        Self::cover_dir().join(format!("{}_full.jpg", Self::hash_name(album_name)))
    }

    /// Turns `"file:///path/to/x.jpg?t=..."` or a plain path into a local `PathBuf`.
    pub fn to_local_path(url_or_path: Option<&str>) -> Option<PathBuf> {
        let raw = url_or_path?.trim();
        if raw.is_empty() {
            return None;
        }
        if raw.len() >= 7 && raw[..7].eq_ignore_ascii_case("file://") {
            let stripped = match raw.find('?') {
                Some(idx) => &raw[..idx],
                None => raw,
            };
            let mut p = &stripped[7..];
            // Legacy Windows file:///C:/... safety just in case a shared library.json contains it.
            let bytes = p.as_bytes();
            if bytes.len() > 2 && bytes[0] == b'/' && bytes[2] == b':' {
                p = &p[1..];
            }
            let decoded = urlencoding::decode(p)
                .map(|c| c.into_owned())
                .unwrap_or_else(|_| p.to_owned());
            return Some(PathBuf::from(decoded));
        }
        Some(PathBuf::from(raw))
    }

    pub fn to_file_url(path: &Path) -> String {
        let normalized = path.to_string_lossy().replace('\\', "/");
        let encoded = normalized
            .split('/')
            .map(|seg| urlencoding::encode(seg).into_owned())
            .collect::<Vec<_>>()
            .join("/");
        format!("file://{encoded}")
    }
}

fn to_base36(mut value: i64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if value == 0 {
        return "0".to_owned();
    }
    let mut buf = [0u8; 16];
    let mut i = buf.len();
    while value > 0 {
        i -= 1;
        buf[i] = DIGITS[(value % 36) as usize];
        value /= 36;
    }
    String::from_utf8(buf[i..].to_vec()).expect("base36 digits are ASCII")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_name_matches_real_library_and_utf16_surrogates() {
        // Verified directly against ~/.retrograde/library.json:
        assert_eq!(
            CoverPaths::hash_name("Ado 1st Live Kigeki (Live At Zepp DiverCity, 2022)"),
            "oy1a16"
        );
        assert_eq!(CoverPaths::hash_name(""), "0");
        // Emoji requires UTF-16 surrogate pair handling rather than Unicode scalar codepoints.
        assert_eq!(CoverPaths::hash_name("🎵"), "120ah");
        // Japanese / CJK title
        assert_eq!(CoverPaths::hash_name("残夢"), "is53");
    }

    #[test]
    fn to_local_path_handles_file_urls_and_queries() {
        let p = CoverPaths::to_local_path(Some("file:///home/okarin/.retrograde/covers/a%20b.jpg?t=123"))
            .unwrap();
        assert_eq!(p, PathBuf::from("/home/okarin/.retrograde/covers/a b.jpg"));
        // Plain Linux filenames with `?` must not be truncated.
        assert_eq!(
            CoverPaths::to_local_path(Some("/music/What?/01.flac")),
            Some(PathBuf::from("/music/What?/01.flac"))
        );
        let url = CoverPaths::to_file_url(Path::new("/home/okarin/My Covers/a #1?.jpg"));
        assert_eq!(
            url,
            "file:///home/okarin/My%20Covers/a%20%231%3F.jpg"
        );
        assert_eq!(
            CoverPaths::to_local_path(Some(&url)),
            Some(PathBuf::from("/home/okarin/My Covers/a #1?.jpg"))
        );
        assert_eq!(CoverPaths::to_local_path(Some("   ")), None);
        assert_eq!(CoverPaths::to_local_path(None), None);
    }
}
