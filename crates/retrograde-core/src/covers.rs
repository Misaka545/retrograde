use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use fast_image_resize::images::Image as FirImage;
use fast_image_resize::{FilterType, IntoImageView, ResizeAlg, ResizeOptions, Resizer};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::metadata::Orientation;
use image::{ColorType, DynamicImage, ImageDecoder, ImageEncoder, ImageReader};
use thiserror::Error;

use crate::library::try_delete;
use crate::paths::CoverPaths;

pub const MAX_COVER_FILE_BYTES: u64 = 30 * 1024 * 1024;
pub const MAX_PIXELS: u64 = 64_000_000;
pub const MAX_DIMENSION: u32 = 16_384;

#[derive(Debug, Error)]
pub enum CoverError {
    #[error("Unsupported image format. Please use JPG, PNG, or WEBP.")]
    UnsupportedFormat,
    #[error("Image could not be read, or is too large.")]
    InsaneImage,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Image decode/encode error: {0}")]
    Image(String),
}

pub struct ImageLimits;

impl ImageLimits {
    pub fn is_sane_bytes(data: &[u8]) -> bool {
        if data.len() as u64 > MAX_COVER_FILE_BYTES {
            return false;
        }
        let Ok(reader) = ImageReader::new(Cursor::new(data)).with_guessed_format() else {
            return false;
        };
        Self::fits(reader.into_dimensions().ok())
    }

    pub fn is_sane_path(path: &Path) -> bool {
        let Ok(meta) = fs::metadata(path) else {
            return false;
        };
        if meta.len() > MAX_COVER_FILE_BYTES {
            return false;
        }
        let Ok(reader) = ImageReader::open(path).and_then(|r| r.with_guessed_format()) else {
            return false;
        };
        Self::fits(reader.into_dimensions().ok())
    }

    fn fits(dims: Option<(u32, u32)>) -> bool {
        let Some((w, h)) = dims else {
            return false;
        };
        w > 0
            && h > 0
            && w <= MAX_DIMENSION
            && h <= MAX_DIMENSION
            && (w as u64) * (h as u64) <= MAX_PIXELS
    }
}

/// Longest edge allowed for stored full-size covers. iced uploads images above ~2 MB
/// RGBA into a dedicated texture (a 4096px cover cost ~117 MB of GPU memory). 720px keeps
/// every aspect ratio under that 2 MB (724x724) limit, so full covers share the image atlas
/// with the thumbnails; it still covers the 510px full-screen cover at scale 1.
pub const MAX_FULL_COVER_EDGE: u32 = 720;

/// Decodes an image and applies its EXIF orientation, so a re-encoded copy (which carries no
/// EXIF) still shows upright. A missing or unreadable orientation tag leaves the image as is.
pub(crate) fn decode_oriented<R: std::io::BufRead + std::io::Seek>(
    reader: ImageReader<R>,
) -> Result<DynamicImage, CoverError> {
    let mut decoder = reader
        .with_guessed_format()
        .map_err(|e| CoverError::Image(e.to_string()))?
        .into_decoder()
        .map_err(|e| CoverError::Image(e.to_string()))?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| CoverError::Image(e.to_string()))?;
    img.apply_orientation(orientation);
    Ok(img)
}

/// Size of `w` x `h` fitted within `max_edge` x `max_edge`, preserving aspect ratio.
fn capped_dims(w: u32, h: u32, max_edge: u32) -> (u32, u32) {
    if max_edge == 0 || w.max(h) <= max_edge {
        return (w, h);
    }
    let scale = |side: u32, long: u32| -> u32 {
        (((side as u64) * (max_edge as u64) + (long as u64) / 2) / (long as u64)).max(1) as u32
    };
    if h > w {
        (scale(w, h), max_edge)
    } else {
        (max_edge, scale(h, w))
    }
}

/// Encodes a full-size cover fitted within `max_edge`. Opaque images become JPEG; images with
/// an alpha channel become PNG so transparency survives (the cover loaders sniff the format
/// from the bytes, so PNG data in a `_full.jpg` file still loads).
pub(crate) fn encode_capped_full(
    img: &DynamicImage,
    max_edge: u32,
    quality: u8,
) -> Result<Vec<u8>, CoverError> {
    if img.color().has_alpha() {
        encode_capped_png(img, max_edge)
    } else {
        encode_capped_jpeg(img, max_edge, quality)
    }
}

/// Encode `img` as JPEG fitted within `max_edge` x `max_edge`, preserving aspect ratio.
/// Images already within the cap are encoded without resizing.
pub(crate) fn encode_capped_jpeg(
    img: &DynamicImage,
    max_edge: u32,
    quality: u8,
) -> Result<Vec<u8>, CoverError> {
    let (dw, dh) = capped_dims(img.width(), img.height(), max_edge);
    encode_to_size(img, dw, dh, quality)
}

fn encode_capped_png(img: &DynamicImage, max_edge: u32) -> Result<Vec<u8>, CoverError> {
    let rgba = img.to_rgba8();
    let (dw, dh) = capped_dims(rgba.width(), rgba.height(), max_edge);
    let pixels = if (dw, dh) != (rgba.width(), rgba.height()) {
        let pixel_type = rgba
            .pixel_type()
            .ok_or_else(|| CoverError::Image("Unsupported pixel type".to_owned()))?;
        let mut dst = FirImage::new(dw, dh, pixel_type);
        let opts = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Mitchell));
        Resizer::new()
            .resize(&rgba, &mut dst, &opts)
            .map_err(|e| CoverError::Image(e.to_string()))?;
        dst.into_vec()
    } else {
        rgba.into_raw()
    };
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(&pixels, dw, dh, ColorType::Rgba8.into())
        .map_err(|e| CoverError::Image(e.to_string()))?;
    Ok(out)
}

pub(crate) fn encode_resized_jpeg(
    img: &DynamicImage,
    max_width: u32,
    quality: u8,
) -> Result<Vec<u8>, CoverError> {
    let (src_w, src_h) = (img.width(), img.height());
    if src_w > max_width && max_width > 0 {
        let dst_h = ((src_h as f64) * (max_width as f64 / src_w as f64))
            .round()
            .max(1.0) as u32;
        encode_to_size(img, max_width, dst_h, quality)
    } else {
        encode_to_size(img, src_w, src_h, quality)
    }
}

fn encode_to_size(
    img: &DynamicImage,
    dst_w: u32,
    dst_h: u32,
    quality: u8,
) -> Result<Vec<u8>, CoverError> {
    let rgb = img.to_rgb8();
    let pixels = if (dst_w, dst_h) != (rgb.width(), rgb.height()) {
        let pixel_type = rgb
            .pixel_type()
            .ok_or_else(|| CoverError::Image("Unsupported pixel type".to_owned()))?;
        let mut dst = FirImage::new(dst_w, dst_h, pixel_type);
        let mut resizer = Resizer::new();
        let opts = ResizeOptions::new()
            .resize_alg(ResizeAlg::Convolution(FilterType::Mitchell));
        resizer
            .resize(&rgb, &mut dst, &opts)
            .map_err(|e| CoverError::Image(e.to_string()))?;
        dst.into_vec()
    } else {
        rgb.into_raw()
    };

    let mut out = Vec::new();
    let mut enc = JpegEncoder::new_with_quality(&mut out, quality);
    enc.encode(&pixels, dst_w, dst_h, ColorType::Rgb8.into())
        .map_err(|e| CoverError::Image(e.to_string()))?;
    Ok(out)
}

/// Shrinks stored `*_full.jpg` covers in `dir` whose longest side exceeds `max_edge` (covers
/// saved before import was capped, or by the C# build). Only image headers are read for covers
/// already within the cap, so this is cheap to run on every start. Each file is replaced
/// atomically (temp file + rename); a cover that cannot be read or re-encoded is left alone.
/// Returns the paths that were rewritten.
pub fn shrink_oversized_full_covers(dir: &Path, max_edge: u32) -> Vec<std::path::PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut changed = Vec::new();
    for path in entries.flatten().map(|e| e.path()) {
        let is_full = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.to_ascii_lowercase().ends_with("_full.jpg"));
        if !is_full || !path.is_file() {
            continue;
        }
        let dims = ImageReader::open(&path)
            .and_then(|r| r.with_guessed_format())
            .ok()
            .and_then(|r| r.into_dimensions().ok());
        let Some((w, h)) = dims else { continue };
        if w.max(h) <= max_edge || !ImageLimits::is_sane_path(&path) {
            continue;
        }
        match shrink_one(&path, max_edge) {
            Ok(()) => changed.push(path),
            Err(e) => tracing::warn!(path = %path.display(), error = %e, "Could not shrink cover"),
        }
    }
    changed
}

fn shrink_one(path: &Path, max_edge: u32) -> Result<(), CoverError> {
    let img = decode_oriented(ImageReader::open(path)?)?;
    let bytes = encode_capped_full(&img, max_edge, 90)?;
    let tmp = path.with_extension("jpg.shrink-tmp");
    fs::write(&tmp, bytes)?;
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

pub struct PlaylistCovers;

impl PlaylistCovers {
    pub const EXTENSIONS: &'static [&'static str] = &["jpg", "jpeg", "png", "webp"];
    const PREFIX: &'static str = "native_pl_";

    pub fn is_supported_ext(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| Self::EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
    }

    pub fn save(
        playlist_id: i64,
        source_path: &Path,
        previous_cover_art: Option<&str>,
    ) -> Result<String, CoverError> {
        if !Self::is_supported_ext(source_path) {
            return Err(CoverError::UnsupportedFormat);
        }
        if !ImageLimits::is_sane_path(source_path) {
            return Err(CoverError::InsaneImage);
        }
        let img = decode_oriented(ImageReader::open(source_path)?)?;

        fs::create_dir_all(CoverPaths::cover_dir())?;
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let stem = CoverPaths::cover_dir().join(format!("{}{}_{}", Self::PREFIX, playlist_id, ms));
        let full = Path::new(&format!("{}_full.jpg", stem.display())).to_path_buf();
        let thumb = Path::new(&format!("{}_thumb.jpg", stem.display())).to_path_buf();

        let full_bytes = encode_capped_full(&img, MAX_FULL_COVER_EDGE, 92)?;
        let thumb_bytes = encode_resized_jpeg(&img, 300, 80)?;
        fs::write(&full, full_bytes)?;
        fs::write(&thumb, thumb_bytes)?;

        Self::delete(previous_cover_art);
        Ok(CoverPaths::to_file_url(&full))
    }

    pub fn delete(cover_art: Option<&str>) {
        let Some(full) = CoverPaths::to_local_path(cover_art) else {
            return;
        };
        let Some(name) = full.file_name().and_then(|n| n.to_str()) else {
            return;
        };
        if !name.starts_with(Self::PREFIX) {
            return;
        }
        try_delete(&full);
        try_delete(&Self::thumb_for(&full));
    }

    pub fn thumb_for(full_path: &Path) -> std::path::PathBuf {
        let s = full_path.to_string_lossy();
        if s.len() >= "_full.jpg".len()
            && s[s.len() - "_full.jpg".len()..].eq_ignore_ascii_case("_full.jpg")
        {
            std::path::PathBuf::from(format!("{}_thumb.jpg", &s[..s.len() - "_full.jpg".len()]))
        } else {
            full_path.to_path_buf()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoverPalette {
    /// Brightened accent color readable on `#121212` panels (Orbit Orange `#FF6B35` default).
    pub accent: [u8; 3],
    /// Rich ambient backdrop glow color.
    pub glow: [u8; 3],
    /// True when a dominant vivid hue was found (false for grayscale/fallback).
    pub vivid: bool,
}

impl CoverPalette {
    pub const ORBIT_DEFAULT: Self = Self {
        accent: [0xFF, 0x6B, 0x35],
        glow: [0xD9, 0x56, 0x26],
        vivid: true,
    };
}

impl Default for CoverPalette {
    fn default() -> Self {
        Self::ORBIT_DEFAULT
    }
}

pub fn extract_cover_palette(path: &Path) -> Option<CoverPalette> {
    if !ImageLimits::is_sane_path(path) {
        return None;
    }
    let img = ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    let small = img.thumbnail(48, 48).to_rgb8();
    Some(extract_cover_palette_from_rgb(
        small.as_raw(),
        small.width(),
        small.height(),
    ))
}

pub fn extract_cover_palette_from_rgb(pixels: &[u8], width: u32, height: u32) -> CoverPalette {
    const BINS: usize = 24;
    if width == 0 || height == 0 || pixels.len() < 3 {
        return CoverPalette::ORBIT_DEFAULT;
    }

    let step_x = (width / 48).max(1);
    let step_y = (height / 48).max(1);
    let mut weight = [0.0_f64; BINS];
    let mut bin_r = [0.0_f64; BINS];
    let mut bin_g = [0.0_f64; BINS];
    let mut bin_b = [0.0_f64; BINS];
    let mut avg_r = 0.0_f64;
    let mut avg_g = 0.0_f64;
    let mut avg_b = 0.0_f64;
    let mut count = 0usize;

    let mut y = 0u32;
    while y < height {
        let mut x = 0u32;
        while x < width {
            let idx = ((y as usize) * (width as usize) + (x as usize)) * 3;
            if idx + 2 >= pixels.len() {
                break;
            }
            let r = pixels[idx] as f64;
            let g = pixels[idx + 1] as f64;
            let b = pixels[idx + 2] as f64;
            avg_r += r;
            avg_g += g;
            avg_b += b;
            count += 1;

            let (h, s, v) = rgb_to_hsv(r / 255.0, g / 255.0, b / 255.0);
            if v >= 0.12 && s >= 0.15 {
                let w = s * s * v;
                let bin = ((h / 360.0) * (BINS as f64)).floor() as usize % BINS;
                weight[bin] += w;
                bin_r[bin] += r * w;
                bin_g[bin] += g * w;
                bin_b[bin] += b * w;
            }
            x += step_x;
        }
        y += step_y;
    }

    if count == 0 {
        return CoverPalette::ORBIT_DEFAULT;
    }

    let mut best = 0usize;
    for i in 1..BINS {
        if weight[i] > weight[best] {
            best = i;
        }
    }

    let vivid = weight[best] > (count as f64) * 0.002;
    let base = if vivid {
        [
            bin_r[best] / weight[best],
            bin_g[best] / weight[best],
            bin_b[best] / weight[best],
        ]
    } else {
        let n = count as f64;
        [avg_r / n, avg_g / n, avg_b / n]
    };

    let (h, s, v) = rgb_to_hsv(base[0] / 255.0, base[1] / 255.0, base[2] / 255.0);
    let (mut acc_s, mut acc_v) = if vivid {
        (s.clamp(0.45, 0.85), v.clamp(0.88, 1.0))
    } else {
        (s.clamp(0.0, 0.18), v.clamp(0.80, 0.88))
    };
    let mut accent = hsv_to_rgb(h, acc_s, acc_v);
    // Keep the accent readable (about 6:1) against the `#121212` panel: lift value,
    // then shed saturation, until relative luminance reaches the floor.
    let mut guard = 0;
    while relative_luminance(accent) < MIN_ACCENT_LUMINANCE && guard < 64 {
        if acc_v < 1.0 {
            acc_v = (acc_v + 0.04).min(1.0);
        } else {
            acc_s = (acc_s - 0.02).max(0.0);
        }
        accent = hsv_to_rgb(h, acc_s, acc_v);
        guard += 1;
    }
    let glow = if vivid {
        hsv_to_rgb(h, s.clamp(0.45, 0.85), v.clamp(0.60, 0.85))
    } else {
        hsv_to_rgb(h, s.clamp(0.0, 0.20), v.clamp(0.35, 0.55))
    };

    CoverPalette {
        accent,
        glow,
        vivid,
    }
}

/// Minimum WCAG relative luminance for an accent on the `#121212` panel (about 6:1 contrast).
pub const MIN_ACCENT_LUMINANCE: f64 = 0.30;

/// WCAG 2.x relative luminance of an sRGB color (0.0 = black, 1.0 = white).
pub fn relative_luminance(rgb: [u8; 3]) -> f64 {
    fn lin(c: u8) -> f64 {
        let c = c as f64 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * lin(rgb[0]) + 0.7152 * lin(rgb[1]) + 0.0722 * lin(rgb[2])
}

fn rgb_to_hsv(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let h = if delta <= 1e-6 {
        0.0
    } else if (max - r).abs() <= 1e-6 {
        60.0 * (((g - b) / delta).rem_euclid(6.0))
    } else if (max - g).abs() <= 1e-6 {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    let s = if max <= 1e-6 { 0.0 } else { delta / max };
    (h.rem_euclid(360.0), s, max)
}

fn hsv_to_rgb(h: f64, s: f64, v: f64) -> [u8; 3] {
    let h = h.rem_euclid(360.0);
    let c = v * s;
    let x = c * (1.0 - (((h / 60.0) % 2.0) - 1.0).abs());
    let m = v - c;
    let (r1, g1, b1) = match (h / 60.0).floor() as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [
        ((r1 + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((g1 + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((b1 + m) * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_cover_with_small_red_accent_extracts_red() {
        let mut pixels = vec![10u8; 48 * 48 * 3];
        // Make ~3% of pixels vivid red.
        for i in 0..70 {
            let base = i * 3;
            pixels[base] = 220;
            pixels[base + 1] = 28;
            pixels[base + 2] = 28;
        }
        let pal = extract_cover_palette_from_rgb(&pixels, 48, 48);
        assert!(pal.vivid);
        assert!(pal.accent[0] > pal.accent[1] + 50);
        assert!(pal.accent[0] > pal.accent[2] + 50);
    }

    #[test]
    fn pure_blue_cover_accent_meets_contrast_floor() {
        let mut pixels = Vec::with_capacity(32 * 32 * 3);
        for _ in 0..(32 * 32) {
            pixels.extend_from_slice(&[0, 0, 255]);
        }
        let pal = extract_cover_palette_from_rgb(&pixels, 32, 32);
        assert!(pal.vivid);
        assert!(
            relative_luminance(pal.accent) >= MIN_ACCENT_LUMINANCE,
            "accent {:?} luminance {}",
            pal.accent,
            relative_luminance(pal.accent)
        );
        // Still recognisably blue.
        assert!(pal.accent[2] > pal.accent[0] && pal.accent[2] > pal.accent[1]);
    }

    #[test]
    fn large_dimensions_do_not_overflow_index_math() {
        // 70_000 * 70_000 overflows u32; a short buffer must simply stop early.
        let pixels = vec![200u8; 3 * 64];
        let pal = extract_cover_palette_from_rgb(&pixels, 70_000, 70_000);
        assert!(relative_luminance(pal.accent) >= MIN_ACCENT_LUMINANCE);
    }

    #[test]
    fn grayscale_cover_falls_back_to_silver() {
        let mut pixels = Vec::with_capacity(24 * 24 * 3);
        for _ in 0..(24 * 24) {
            pixels.extend_from_slice(&[90, 90, 90]);
        }
        let pal = extract_cover_palette_from_rgb(&pixels, 24, 24);
        assert!(!pal.vivid);
        assert!((pal.accent[0] as i16 - pal.accent[1] as i16).abs() <= 2);
        assert!((pal.accent[1] as i16 - pal.accent[2] as i16).abs() <= 2);
        assert!(pal.accent[0] >= 200 && pal.accent[0] <= 230);
    }

    fn dims_of(bytes: &[u8]) -> (u32, u32) {
        let img = ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap();
        (img.width(), img.height())
    }

    fn blank(w: u32, h: u32) -> DynamicImage {
        DynamicImage::ImageRgb8(image::RgbImage::new(w, h))
    }

    #[test]
    fn capped_jpeg_landscape() {
        let out = encode_capped_jpeg(&blank(3000, 2000), MAX_FULL_COVER_EDGE, 90).unwrap();
        assert_eq!(dims_of(&out), (720, 480));
    }

    #[test]
    fn capped_jpeg_portrait() {
        let out = encode_capped_jpeg(&blank(2000, 3000), MAX_FULL_COVER_EDGE, 90).unwrap();
        assert_eq!(dims_of(&out), (480, 720));
    }

    #[test]
    fn capped_full_keeps_alpha_as_png() {
        let mut rgba = image::RgbaImage::new(2048, 1024);
        rgba.put_pixel(0, 0, image::Rgba([255, 0, 0, 0]));
        let out = encode_capped_full(&DynamicImage::ImageRgba8(rgba), MAX_FULL_COVER_EDGE, 90).unwrap();
        assert!(out.starts_with(&[0x89, b'P', b'N', b'G']));
        let img = ImageReader::new(Cursor::new(&out)).with_guessed_format().unwrap().decode().unwrap();
        assert_eq!((img.width(), img.height()), (720, 360));
        assert!(img.color().has_alpha());
    }

    #[test]
    fn capped_full_opaque_is_jpeg() {
        let out = encode_capped_full(&blank(3000, 3000), MAX_FULL_COVER_EDGE, 90).unwrap();
        assert!(out.starts_with(&[0xFF, 0xD8]));
        assert_eq!(dims_of(&out), (720, 720));
    }

    #[test]
    fn shrink_oversized_full_covers_rewrites_only_big_fulls() {
        let dir = std::env::temp_dir().join(format!("retro-shrink-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let big = dir.join("big_full.jpg");
        let small = dir.join("small_full.jpg");
        let big_thumb = dir.join("big_thumb.jpg");
        fs::write(&big, encode_capped_jpeg(&blank(2000, 1500), u32::MAX, 90).unwrap()).unwrap();
        fs::write(&small, encode_capped_jpeg(&blank(600, 600), u32::MAX, 90).unwrap()).unwrap();
        fs::write(&big_thumb, encode_capped_jpeg(&blank(2000, 2000), u32::MAX, 90).unwrap()).unwrap();
        let small_before = fs::read(&small).unwrap();

        let changed = shrink_oversized_full_covers(&dir, MAX_FULL_COVER_EDGE);

        assert_eq!(changed, vec![big.clone()]);
        assert_eq!(dims_of(&fs::read(&big).unwrap()), (720, 540));
        assert_eq!(fs::read(&small).unwrap(), small_before);
        assert_eq!(dims_of(&fs::read(&big_thumb).unwrap()), (2000, 2000));
        assert!(shrink_oversized_full_covers(&dir, MAX_FULL_COVER_EDGE).is_empty());
        let leftovers = fs::read_dir(&dir).unwrap().count();
        assert_eq!(leftovers, 3);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn capped_jpeg_small_unchanged() {
        let out = encode_capped_jpeg(&blank(700, 500), MAX_FULL_COVER_EDGE, 90).unwrap();
        assert_eq!(dims_of(&out), (700, 500));
        let out = encode_capped_jpeg(&blank(500, 700), MAX_FULL_COVER_EDGE, 90).unwrap();
        assert_eq!(dims_of(&out), (500, 700));
    }
}
