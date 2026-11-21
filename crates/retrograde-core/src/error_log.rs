use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::paths::CoverPaths;

const MAX_BYTES: u64 = 512 * 1024;
static GATE: Mutex<()> = Mutex::new(());

/// Unexpected errors go to `~/.retrograde/native-error.log` (or `$RETRO_APPDIR/native-error.log`).
/// Best-effort: logging never panics.
pub fn file_path() -> PathBuf {
    CoverPaths::app_dir().join("native-error.log")
}

pub fn write(context: &str, message: &str) {
    let Ok(_guard) = GATE.lock() else {
        return;
    };
    let dir = CoverPaths::app_dir();
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = file_path();
    if let Ok(meta) = fs::metadata(&path)
        && meta.len() > MAX_BYTES
    {
        let mut old = path.as_os_str().to_os_string();
        old.push(".old");
        let _ = fs::rename(&path, old);
    }
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let stamp = format_utc_now();
    let _ = writeln!(f, "{stamp} [{context}] {message}\n");
}

fn format_utc_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // Civil-from-days (Howard Hinnant algorithm) so we don't need chrono for error timestamps.
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let sec = rem % 60;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02} {hour:02}:{min:02}:{sec:02} UTC")
}
