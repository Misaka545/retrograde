//! One-time seed of liked songs and playlists from the Electron build's localStorage.
//!
//! Port of the C# `LevelDbReader` + `ElectronImport`: a minimal read-only LevelDB reader
//! (log + table files, Snappy blocks) that returns the newest live value of every key, and the
//! decoding of Chromium's localStorage entries `my_playlists` / `liked_songs`.
//! Malformed bytes never panic: a damaged file only loses its own entries.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::collections::{CollectionsData, PlaylistData};

/// `$XDG_CONFIG_HOME/{retrograde,Retrograde}/Local Storage/leveldb` (.NET's `ApplicationData`
/// on Linux, which is where the C# build looked), falling back to `~/.config`.
pub fn find_local_storage_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    ["retrograde", "Retrograde"]
        .iter()
        .map(|name| base.join(name).join("Local Storage").join("leveldb"))
        .find(|dir| dir.is_dir())
}

/// Reads the Electron playlists / liked songs from the default localStorage location.
pub fn try_read() -> Option<CollectionsData> {
    try_read_from(&find_local_storage_dir()?)
}

/// Reads `my_playlists` and `liked_songs` from the LevelDB in `dir`. `None` when neither key is
/// there or their JSON does not parse (the C# build gave up on the whole import then too).
pub fn try_read_from(dir: &Path) -> Option<CollectionsData> {
    let mut playlists_json = None;
    let mut liked_json = None;
    for (key, raw) in read_leveldb(dir) {
        // Chromium key: "_<origin>\0\x01<name>"; value: 0x00 + UTF-16LE or 0x01 + Latin-1.
        if !key.starts_with(b"_") {
            continue;
        }
        if key.ends_with(b"\x01my_playlists") {
            playlists_json = Some(decode_value(&raw));
        } else if key.ends_with(b"\x01liked_songs") {
            liked_json = Some(decode_value(&raw));
        }
    }
    if playlists_json.is_none() && liked_json.is_none() {
        return None;
    }

    let mut data = CollectionsData {
        imported_from: Some(dir.to_string_lossy().into_owned()),
        ..CollectionsData::default()
    };
    if let Some(json) = liked_json {
        let doc: Value = serde_json::from_str(&json).ok()?;
        for t in doc.as_array()? {
            if let Some(id) = track_id(t)
                && !data.liked.contains(&id)
            {
                data.liked.push(id);
            }
        }
    }
    if let Some(json) = playlists_json {
        let doc: Value = serde_json::from_str(&json).ok()?;
        for pl in doc.as_array()? {
            let mut p = PlaylistData {
                id: pl.get("id").and_then(Value::as_i64).unwrap_or_else(now_ms),
                name: pl
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                cover_art: pl
                    .get("coverArt")
                    .and_then(Value::as_str)
                    .filter(|c| c.get(..5).is_some_and(|s| s.eq_ignore_ascii_case("file:")))
                    .map(str::to_owned),
                ..PlaylistData::default()
            };
            if let Some(tracks) = pl.get("tracks").and_then(Value::as_array) {
                for t in tracks {
                    if let Some(id) = track_id(t)
                        && !p.track_ids.contains(&id)
                    {
                        p.track_ids.push(id);
                    }
                }
            }
            data.playlists.push(p);
        }
    }
    Some(data)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `filePath` first: some Electron entries carry ids with a timestamp suffix.
fn track_id(t: &Value) -> Option<String> {
    let obj = t.as_object()?;
    obj.get("filePath")
        .and_then(Value::as_str)
        .or_else(|| obj.get("id").and_then(Value::as_str))
        .map(str::to_owned)
}

fn decode_value(raw: &[u8]) -> String {
    match raw.split_first() {
        None => String::new(),
        Some((0, rest)) => {
            let units: Vec<u16> = rest
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&c| u16::from_le_bytes(c))
                .collect();
            String::from_utf16_lossy(&units)
        }
        Some((_, rest)) => rest.iter().map(|&b| b as char).collect(),
    }
}

// ---------------------------------------------------------------------------------------------
// LevelDB
// ---------------------------------------------------------------------------------------------

type Newest = HashMap<Vec<u8>, (u64, Option<Vec<u8>>)>;

fn put(newest: &mut Newest, key: Vec<u8>, seq: u64, value: Option<Vec<u8>>) {
    match newest.get(&key) {
        Some((cur, _)) if seq < *cur => {}
        _ => {
            newest.insert(key, (seq, value));
        }
    }
}

/// Newest live value of every key in the LevelDB at `dir` (deleted keys are left out).
pub fn read_leveldb(dir: &Path) -> HashMap<Vec<u8>, Vec<u8>> {
    let mut newest = Newest::new();
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    files.sort();
    for f in files {
        let ext = f
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        let is_table = ext == "ldb" || ext == "sst";
        if !is_table && ext != "log" {
            continue;
        }
        let Ok(bytes) = fs::read(&f) else { continue };
        if is_table {
            read_table(&bytes, &mut newest);
        } else {
            read_log(&bytes, &mut newest);
        }
    }
    newest
        .into_iter()
        .filter_map(|(k, (_, v))| v.map(|v| (k, v)))
        .collect()
}

/// Log files: 32 KB blocks of records (7-byte header) carrying WriteBatches.
fn read_log(data: &[u8], newest: &mut Newest) {
    const BLOCK: usize = 32768;
    let mut pending: Vec<u8> = Vec::new();
    let mut pos = 0usize;
    while pos + 7 <= data.len() {
        let left = BLOCK - pos % BLOCK;
        if left < 7 {
            pos += left;
            continue;
        }
        let len = data[pos + 4] as usize | (data[pos + 5] as usize) << 8;
        let kind = data[pos + 6];
        let start = pos + 7;
        if kind == 0 && len == 0 {
            pos += left; // zero padding
            continue;
        }
        let Some(payload) = data.get(start..start + len) else {
            break;
        };
        match kind {
            1 => {
                apply_batch(payload, newest);
                pending.clear();
            }
            2 => {
                pending.clear();
                pending.extend_from_slice(payload);
            }
            3 => pending.extend_from_slice(payload),
            4 => {
                pending.extend_from_slice(payload);
                apply_batch(&pending, newest);
                pending.clear();
            }
            _ => {}
        }
        pos = start + len;
    }
}

fn apply_batch(b: &[u8], newest: &mut Newest) {
    let (Some(seq), Some(count)) = (b.get(..8), b.get(8..12)) else {
        return;
    };
    let mut seq = u64::from_le_bytes(seq.try_into().unwrap_or_default());
    let count = u32::from_le_bytes(count.try_into().unwrap_or_default());
    let mut p = 12usize;
    for _ in 0..count {
        let Some(&kind) = b.get(p) else { return };
        p += 1;
        let Some(key) = read_slice(b, &mut p) else {
            return;
        };
        if kind == 1 {
            let Some(value) = read_slice(b, &mut p) else {
                return;
            };
            put(newest, key.to_vec(), seq, Some(value.to_vec()));
        } else {
            put(newest, key.to_vec(), seq, None);
        }
        seq = seq.wrapping_add(1);
    }
}

/// Table files: footer -> index block -> data blocks of internal keys (user key + 8-byte trailer).
fn read_table(data: &[u8], newest: &mut Newest) {
    let Some(footer) = data.len().checked_sub(48).and_then(|s| data.get(s..)) else {
        return;
    };
    let mut p = 0usize;
    // metaindex handle, then index handle
    let (Some(_), Some(_), Some(idx_off), Some(idx_size)) = (
        read_varint(footer, &mut p),
        read_varint(footer, &mut p),
        read_varint(footer, &mut p),
        read_varint(footer, &mut p),
    ) else {
        return;
    };
    let Some(index) = read_block(data, idx_off, idx_size) else {
        return;
    };
    for (_, handle) in block_entries(&index) {
        let mut hp = 0usize;
        let (Some(off), Some(size)) = (read_varint(&handle, &mut hp), read_varint(&handle, &mut hp))
        else {
            continue;
        };
        let Some(block) = read_block(data, off, size) else {
            continue;
        };
        for (ikey, value) in block_entries(&block) {
            let Some(split) = ikey.len().checked_sub(8) else {
                continue;
            };
            let trailer = u64::from_le_bytes(ikey[split..].try_into().unwrap_or_default());
            let value = (trailer & 0xFF == 1).then_some(value);
            put(newest, ikey[..split].to_vec(), trailer >> 8, value);
        }
    }
}

fn read_block(data: &[u8], off: u64, size: u64) -> Option<Vec<u8>> {
    let off = usize::try_from(off).ok()?;
    let end = off.checked_add(usize::try_from(size).ok()?)?;
    let raw = data.get(off..end)?;
    match *data.get(end)? {
        1 => snappy_decompress(raw),
        _ => Some(raw.to_vec()),
    }
}

/// Every (key, value) of a block, prefix-compressed keys expanded.
fn block_entries(block: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut out = Vec::new();
    let Some(n) = block.len().checked_sub(4) else {
        return out;
    };
    let restarts = u32::from_le_bytes(block[n..].try_into().unwrap_or_default()) as usize;
    let Some(end) = restarts.checked_mul(4).and_then(|r| n.checked_sub(r)) else {
        return out;
    };
    let mut p = 0usize;
    let mut last: Vec<u8> = Vec::new();
    while p < end {
        let (Some(shared), Some(non_shared), Some(value_len)) = (
            read_varint(block, &mut p),
            read_varint(block, &mut p),
            read_varint(block, &mut p),
        ) else {
            break;
        };
        let (shared, non_shared, value_len) =
            (shared as usize, non_shared as usize, value_len as usize);
        let (Some(prefix), Some(rest)) = (
            last.get(..shared),
            p.checked_add(non_shared).and_then(|e| block.get(p..e)),
        ) else {
            break;
        };
        let mut key = prefix.to_vec();
        key.extend_from_slice(rest);
        p += non_shared;
        let Some(value) = p.checked_add(value_len).and_then(|e| block.get(p..e)) else {
            break;
        };
        p += value_len;
        last.clone_from(&key);
        out.push((key, value.to_vec()));
    }
    out
}

fn read_slice<'a>(b: &'a [u8], p: &mut usize) -> Option<&'a [u8]> {
    let len = usize::try_from(read_varint(b, p)?).ok()?;
    let s = b.get(*p..p.checked_add(len)?)?;
    *p += len;
    Some(s)
}

fn read_varint(b: &[u8], p: &mut usize) -> Option<u64> {
    let mut result = 0u64;
    let mut shift = 0u32;
    while shift < 64 {
        let x = *b.get(*p)?;
        *p += 1;
        result |= u64::from(x & 0x7F) << shift;
        if x & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
    }
    Some(result)
}

/// Raw Snappy block format (no framing), as LevelDB stores compressed blocks.
fn snappy_decompress(src: &[u8]) -> Option<Vec<u8>> {
    let mut p = 0usize;
    let len = usize::try_from(read_varint(src, &mut p)?).ok()?;
    // A corrupt header must not make us allocate gigabytes up front.
    let mut dst: Vec<u8> = Vec::with_capacity(len.min(src.len().saturating_mul(32)));
    while p < src.len() && dst.len() < len {
        let tag = src[p];
        p += 1;
        let (n, off) = match tag & 3 {
            0 => {
                let mut n = (tag >> 2) as usize;
                if n >= 60 {
                    let extra = n - 59;
                    let bytes = src.get(p..p + extra)?;
                    p += extra;
                    n = bytes
                        .iter()
                        .enumerate()
                        .fold(0usize, |acc, (i, &b)| acc | (b as usize) << (8 * i));
                }
                let n = n.checked_add(1)?;
                dst.extend_from_slice(src.get(p..p.checked_add(n)?)?);
                p += n;
                continue;
            }
            1 => {
                let n = ((tag >> 2) & 7) as usize + 4;
                let off = ((tag >> 5) as usize) << 8 | *src.get(p)? as usize;
                p += 1;
                (n, off)
            }
            2 => {
                let n = (tag >> 2) as usize + 1;
                let b = src.get(p..p + 2)?;
                p += 2;
                (n, u16::from_le_bytes([b[0], b[1]]) as usize)
            }
            _ => {
                let n = (tag >> 2) as usize + 1;
                let b = src.get(p..p + 4)?;
                p += 4;
                (n, u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
            }
        };
        if off == 0 || off > dst.len() {
            return None;
        }
        // Overlapping copies are allowed (run-length style), so copy byte by byte.
        let s = dst.len() - off;
        for i in 0..n {
            let byte = dst[s + i];
            dst.push(byte);
        }
    }
    Some(dst)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn varint(mut v: u64, out: &mut Vec<u8>) {
        while v >= 0x80 {
            out.push((v as u8) | 0x80);
            v >>= 7;
        }
        out.push(v as u8);
    }

    fn slice(s: &[u8], out: &mut Vec<u8>) {
        varint(s.len() as u64, out);
        out.extend_from_slice(s);
    }

    /// A log file with one full record holding one WriteBatch. `None` value = delete.
    fn log_file(seq: u64, ops: &[(&[u8], Option<&[u8]>)]) -> Vec<u8> {
        let mut batch = Vec::new();
        batch.extend_from_slice(&seq.to_le_bytes());
        batch.extend_from_slice(&(ops.len() as u32).to_le_bytes());
        for (k, v) in ops {
            batch.push(u8::from(v.is_some()));
            slice(k, &mut batch);
            if let Some(v) = v {
                slice(v, &mut batch);
            }
        }
        let mut out = vec![0, 0, 0, 0]; // CRC, not checked
        out.extend_from_slice(&(batch.len() as u16).to_le_bytes());
        out.push(1); // full record
        out.extend_from_slice(&batch);
        out
    }

    /// Snappy with literals only (valid, just not compressed).
    fn snappy_literal(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        varint(data.len() as u64, &mut out);
        for chunk in data.chunks(60) {
            out.push(((chunk.len() - 1) as u8) << 2);
            out.extend_from_slice(chunk);
        }
        out
    }

    fn block(entries: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (k, v) in entries {
            varint(0, &mut out);
            varint(k.len() as u64, &mut out);
            varint(v.len() as u64, &mut out);
            out.extend_from_slice(k);
            out.extend_from_slice(v);
        }
        out.extend_from_slice(&0u32.to_le_bytes()); // restart[0]
        out.extend_from_slice(&1u32.to_le_bytes()); // restart count
        out
    }

    /// (user key, sequence, value); `None` = deletion.
    type Entry<'a> = (&'a [u8], u64, Option<&'a [u8]>);

    /// A table with one Snappy data block of entries.
    fn table_file(entries: &[Entry]) -> Vec<u8> {
        let ikeys: Vec<(Vec<u8>, Vec<u8>)> = entries
            .iter()
            .map(|(k, seq, v)| {
                let mut ik = k.to_vec();
                let kind = u64::from(v.is_some());
                ik.extend_from_slice(&((seq << 8) | kind).to_le_bytes());
                (ik, v.unwrap_or_default().to_vec())
            })
            .collect();
        let data_block = snappy_literal(&block(&ikeys));
        let mut file = data_block.clone();
        file.push(1); // snappy
        file.extend_from_slice(&[0; 4]); // CRC
        let mut handle = Vec::new();
        varint(0, &mut handle);
        varint(data_block.len() as u64, &mut handle);
        let index = block(&[(b"zzzz".to_vec(), handle)]);
        let index_off = file.len();
        file.extend_from_slice(&index);
        file.push(0); // uncompressed
        file.extend_from_slice(&[0; 4]);
        let mut footer = Vec::new();
        varint(0, &mut footer);
        varint(0, &mut footer);
        varint(index_off as u64, &mut footer);
        varint(index.len() as u64, &mut footer);
        footer.resize(40, 0);
        footer.extend_from_slice(&0xdb47_7524_8b80_fb57u64.to_le_bytes());
        file.extend_from_slice(&footer);
        file
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "retro-leveldb-test-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn snappy_overlapping_copy() {
        // len 7, literal "ab", copy-1 len 5 offset 2 -> "abababa"
        let src = [7, 0x04, b'a', b'b', (1 << 2) | 1, 2];
        assert_eq!(snappy_decompress(&src).unwrap(), b"abababa");
        // copy before any output is rejected, not a panic
        assert_eq!(snappy_decompress(&[4, 0x01, 1]), None);
    }

    #[test]
    fn log_overrides_table_and_deletes() {
        let dir = temp_dir("merge");
        fs::write(
            dir.join("000005.ldb"),
            table_file(&[
                (b"a", 1, Some(b"old")),
                (b"b", 2, Some(b"kept")),
                (b"c", 3, Some(b"gone")),
            ]),
        )
        .unwrap();
        fs::write(
            dir.join("000006.log"),
            log_file(10, &[(b"a", Some(b"new")), (b"c", None), (b"d", Some(b"log"))]),
        )
        .unwrap();
        fs::write(dir.join("LOCK"), b"").unwrap();
        let all = read_leveldb(&dir);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(all.get(b"a".as_slice()).map(Vec::as_slice), Some(b"new".as_slice()));
        assert_eq!(all.get(b"b".as_slice()).map(Vec::as_slice), Some(b"kept".as_slice()));
        assert_eq!(all.get(b"d".as_slice()).map(Vec::as_slice), Some(b"log".as_slice()));
        assert!(!all.contains_key(b"c".as_slice()));
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn damaged_files_do_not_panic() {
        let dir = temp_dir("damaged");
        fs::write(dir.join("000001.ldb"), [0xff; 60]).unwrap();
        fs::write(dir.join("000002.log"), [0, 0, 0, 0, 0xff, 0xff, 1, 3]).unwrap();
        let mut log = log_file(1, &[(b"x", Some(b"y"))]);
        log.truncate(log.len() - 1);
        fs::write(dir.join("000003.log"), log).unwrap();
        let all = read_leveldb(&dir);
        let _ = fs::remove_dir_all(&dir);
        assert!(all.is_empty());
    }

    #[test]
    fn imports_chromium_local_storage() {
        let dir = temp_dir("import");
        let playlists = r#"[{"id": 42, "name": "Road", "coverArt": "FILE:///c.jpg",
            "tracks": [{"filePath": "/m/1.flac", "id": "/m/1.flac123"}, {"id": "/m/2.flac"},
                       {"filePath": "/m/1.flac"}, 5]},
            {"name": "No id", "coverArt": "https://x/y.jpg"}]"#;
        // Latin-1 value (prefix 0x01)
        let mut pl_value = vec![1u8];
        pl_value.extend_from_slice(playlists.as_bytes());
        // UTF-16LE value (prefix 0x00) with a non-ASCII path
        let liked = r#"[{"filePath": "/m/Café.flac"}, {"id": "/m/3.flac"}, {"filePath": "/m/Café.flac"}]"#;
        let mut liked_value = vec![0u8];
        for u in liked.encode_utf16() {
            liked_value.extend_from_slice(&u.to_le_bytes());
        }
        fs::write(
            dir.join("000003.log"),
            log_file(
                5,
                &[
                    (b"_file://\x00\x01my_playlists", Some(&pl_value)),
                    (b"_file://\x00\x01liked_songs", Some(&liked_value)),
                    (b"META:file://", Some(b"\x08\x01")),
                ],
            ),
        )
        .unwrap();
        let data = try_read_from(&dir).expect("import");
        let empty = temp_dir("import-empty");
        let none = try_read_from(&empty);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty);

        assert!(none.is_none());
        assert_eq!(data.imported_from.as_deref(), Some(dir.to_string_lossy().as_ref()));
        assert_eq!(data.liked, vec!["/m/Café.flac", "/m/3.flac"]);
        assert_eq!(data.playlists.len(), 2);
        let p = &data.playlists[0];
        assert_eq!((p.id, p.name.as_str()), (42, "Road"));
        assert_eq!(p.cover_art.as_deref(), Some("FILE:///c.jpg"));
        assert_eq!(p.track_ids, vec!["/m/1.flac", "/m/2.flac"]);
        let q = &data.playlists[1];
        assert!(q.id > 0 && q.cover_art.is_none() && q.track_ids.is_empty());
        assert!(data.smart.is_none());
    }
}
