use std::ffi::{CStr, CString, c_char, c_int, c_long, c_uint, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use libloading::Library;

const CANDIDATES: &[&str] = &[
    "Master",
    "PCM",
    "Speaker",
    "Headphone",
    "Digital",
    "Line Out",
    "Front",
];

/// The `0.0..=1.0` knob maps linearly in dB over the top 60 dB (`6000` centi-dB) of the hardware control.
pub const MAX_SPAN_CENTI_DB: i64 = 6000;

struct AlsaVtable {
    _lib: Library,
    mixer_open: unsafe extern "C" fn(*mut *mut c_void, c_int) -> c_int,
    mixer_close: unsafe extern "C" fn(*mut c_void) -> c_int,
    mixer_attach: unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int,
    mixer_selem_register: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> c_int,
    mixer_load: unsafe extern "C" fn(*mut c_void) -> c_int,
    selem_id_malloc: unsafe extern "C" fn(*mut *mut c_void) -> c_int,
    selem_id_free: unsafe extern "C" fn(*mut c_void),
    selem_id_set_index: unsafe extern "C" fn(*mut c_void, c_uint),
    selem_id_set_name: unsafe extern "C" fn(*mut c_void, *const c_char),
    find_selem: unsafe extern "C" fn(*mut c_void, *const c_void) -> *mut c_void,
    first_elem: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    elem_next: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    selem_get_name: unsafe extern "C" fn(*mut c_void) -> *const c_char,
    has_playback_volume: unsafe extern "C" fn(*mut c_void) -> c_int,
    has_playback_switch: unsafe extern "C" fn(*mut c_void) -> c_int,
    get_playback_switch: unsafe extern "C" fn(*mut c_void, c_int, *mut c_int) -> c_int,
    set_playback_switch_all: unsafe extern "C" fn(*mut c_void, c_int) -> c_int,
    get_playback_db_range: unsafe extern "C" fn(*mut c_void, *mut c_long, *mut c_long) -> c_int,
    get_playback_db: unsafe extern "C" fn(*mut c_void, c_int, *mut c_long) -> c_int,
    set_playback_db_all: unsafe extern "C" fn(*mut c_void, c_long, c_int) -> c_int,
    get_playback_vol_range: unsafe extern "C" fn(*mut c_void, *mut c_long, *mut c_long) -> c_int,
    get_playback_vol: unsafe extern "C" fn(*mut c_void, c_int, *mut c_long) -> c_int,
    set_playback_vol_all: unsafe extern "C" fn(*mut c_void, c_long) -> c_int,
}

unsafe impl Send for AlsaVtable {}
unsafe impl Sync for AlsaVtable {}

static ALSA_VT: OnceLock<Option<AlsaVtable>> = OnceLock::new();

fn vtable() -> Option<&'static AlsaVtable> {
    ALSA_VT
        .get_or_init(|| unsafe {
            let lib = Library::new("libasound.so.2")
                .or_else(|_| Library::new("libasound.so"))
                .ok()?;
            macro_rules! sym {
                ($n:literal) => {
                    *lib.get($n).ok()?
                };
            }
            Some(AlsaVtable {
                mixer_open: sym!(b"snd_mixer_open\0"),
                mixer_close: sym!(b"snd_mixer_close\0"),
                mixer_attach: sym!(b"snd_mixer_attach\0"),
                mixer_selem_register: sym!(b"snd_mixer_selem_register\0"),
                mixer_load: sym!(b"snd_mixer_load\0"),
                selem_id_malloc: sym!(b"snd_mixer_selem_id_malloc\0"),
                selem_id_free: sym!(b"snd_mixer_selem_id_free\0"),
                selem_id_set_index: sym!(b"snd_mixer_selem_id_set_index\0"),
                selem_id_set_name: sym!(b"snd_mixer_selem_id_set_name\0"),
                find_selem: sym!(b"snd_mixer_find_selem\0"),
                first_elem: sym!(b"snd_mixer_first_elem\0"),
                elem_next: sym!(b"snd_mixer_elem_next\0"),
                selem_get_name: sym!(b"snd_mixer_selem_get_name\0"),
                has_playback_volume: sym!(b"snd_mixer_selem_has_playback_volume\0"),
                has_playback_switch: sym!(b"snd_mixer_selem_has_playback_switch\0"),
                get_playback_switch: sym!(b"snd_mixer_selem_get_playback_switch\0"),
                set_playback_switch_all: sym!(b"snd_mixer_selem_set_playback_switch_all\0"),
                get_playback_db_range: sym!(b"snd_mixer_selem_get_playback_dB_range\0"),
                get_playback_db: sym!(b"snd_mixer_selem_get_playback_dB\0"),
                set_playback_db_all: sym!(b"snd_mixer_selem_set_playback_dB_all\0"),
                get_playback_vol_range: sym!(b"snd_mixer_selem_get_playback_volume_range\0"),
                get_playback_vol: sym!(b"snd_mixer_selem_get_playback_volume\0"),
                set_playback_vol_all: sym!(b"snd_mixer_selem_set_playback_volume_all\0"),
                _lib: lib,
            })
        })
        .as_ref()
}

pub struct AlsaMixer;

impl AlsaMixer {
    pub fn find_control(card: i32) -> Option<String> {
        for &name in CANDIDATES {
            if Self::with_elem(card, name, |_, _| true) {
                return Some(name.to_owned());
            }
        }
        Self::find_first_playback_volume_control(card)
    }

    fn find_first_playback_volume_control(card: i32) -> Option<String> {
        let vt = vtable()?;
        let hw_name = CString::new(format!("hw:{card}")).ok()?;
        unsafe {
            struct MixerGuard<'a> {
                vt: &'a AlsaVtable,
                mixer: *mut c_void,
            }
            impl Drop for MixerGuard<'_> {
                fn drop(&mut self) {
                    if !self.mixer.is_null() {
                        unsafe { (self.vt.mixer_close)(self.mixer) };
                    }
                }
            }

            let mut mixer: *mut c_void = ptr::null_mut();
            if (vt.mixer_open)(&raw mut mixer, 0) < 0 || mixer.is_null() {
                return None;
            }
            let guard = MixerGuard { vt, mixer };
            if (vt.mixer_attach)(guard.mixer, hw_name.as_ptr()) < 0
                || (vt.mixer_selem_register)(guard.mixer, ptr::null_mut(), ptr::null_mut()) < 0
                || (vt.mixer_load)(guard.mixer) < 0
            {
                return None;
            }
            let mut elem = (vt.first_elem)(guard.mixer);
            while !elem.is_null() {
                if (vt.has_playback_volume)(elem) != 0 {
                    let name_ptr = (vt.selem_get_name)(elem);
                    if !name_ptr.is_null() {
                        let s = CStr::from_ptr(name_ptr).to_string_lossy().into_owned();
                        if !s.is_empty() {
                            return Some(s);
                        }
                    }
                }
                elem = (vt.elem_next)(elem);
            }
            None
        }
    }

    // c_long is i64 on x86_64/aarch64 but i32 on 32-bit Linux targets.
    #[allow(clippy::unnecessary_cast)]
    pub fn read_level(card: i32, control: &str) -> Option<f64> {
        let mut out = None;
        Self::with_elem(card, control, |vt, elem| unsafe {
            let (mut min, mut max, mut db): (c_long, c_long, c_long) = (0, 0, 0);
            if (vt.get_playback_db_range)(elem, &raw mut min, &raw mut max) == 0
                && max > min
                && (vt.get_playback_db)(elem, 0, &raw mut db) == 0
            {
                out = Some(db_to_level(min as i64, max as i64, db as i64));
                return true;
            }
            let (mut rmin, mut rmax, mut raw): (c_long, c_long, c_long) = (0, 0, 0);
            if (vt.get_playback_vol_range)(elem, &raw mut rmin, &raw mut rmax) == 0
                && rmax > rmin
                && (vt.get_playback_vol)(elem, 0, &raw mut raw) == 0
            {
                out = Some(raw_to_level(rmin as i64, rmax as i64, raw as i64));
                return true;
            }
            false
        });
        out
    }

    pub fn read_muted(card: i32, control: &str) -> Option<bool> {
        let mut out = None;
        Self::with_elem(card, control, |vt, elem| unsafe {
            if (vt.has_playback_switch)(elem) != 0 {
                let mut val: c_int = 1;
                if (vt.get_playback_switch)(elem, 0, &raw mut val) == 0 {
                    out = Some(val == 0);
                    return true;
                }
            }
            false
        });
        out
    }

    #[allow(clippy::unnecessary_cast)]
    pub fn write_level(card: i32, control: &str, level: f64) -> bool {
        let level = level.clamp(0.0, 1.0);
        Self::with_elem(card, control, |vt, elem| unsafe {
            let (mut min, mut max): (c_long, c_long) = (0, 0);
            if (vt.get_playback_db_range)(elem, &raw mut min, &raw mut max) == 0 && max > min {
                let db = level_to_db(min as i64, max as i64, level);
                return (vt.set_playback_db_all)(elem, db as c_long, 0) >= 0;
            }
            let (mut rmin, mut rmax): (c_long, c_long) = (0, 0);
            if (vt.get_playback_vol_range)(elem, &raw mut rmin, &raw mut rmax) == 0 && rmax > rmin {
                let raw = level_to_raw(rmin as i64, rmax as i64, level);
                return (vt.set_playback_vol_all)(elem, raw as c_long) >= 0;
            }
            false
        })
    }

    #[allow(clippy::unnecessary_cast)]
    pub fn set_mute(card: i32, control: &str, muted: bool, restore_level: f64) -> bool {
        Self::with_elem(card, control, |vt, elem| unsafe {
            if (vt.has_playback_switch)(elem) != 0 {
                return (vt.set_playback_switch_all)(elem, if muted { 0 } else { 1 }) >= 0;
            }
            if muted {
                let (mut min, mut max): (c_long, c_long) = (0, 0);
                if (vt.get_playback_db_range)(elem, &raw mut min, &raw mut max) == 0 && max > min {
                    return (vt.set_playback_db_all)(elem, min, 0) >= 0;
                }
                let (mut rmin, mut rmax): (c_long, c_long) = (0, 0);
                if (vt.get_playback_vol_range)(elem, &raw mut rmin, &raw mut rmax) == 0
                    && rmax > rmin
                {
                    return (vt.set_playback_vol_all)(elem, rmin) >= 0;
                }
                false
            } else {
                let level = restore_level.clamp(0.0, 1.0);
                let (mut min, mut max): (c_long, c_long) = (0, 0);
                if (vt.get_playback_db_range)(elem, &raw mut min, &raw mut max) == 0 && max > min {
                    let db = level_to_db(min as i64, max as i64, level);
                    return (vt.set_playback_db_all)(elem, db as c_long, 0) >= 0;
                }
                let (mut rmin, mut rmax): (c_long, c_long) = (0, 0);
                if (vt.get_playback_vol_range)(elem, &raw mut rmin, &raw mut rmax) == 0
                    && rmax > rmin
                {
                    let raw = level_to_raw(rmin as i64, rmax as i64, level);
                    return (vt.set_playback_vol_all)(elem, raw as c_long) >= 0;
                }
                false
            }
        })
    }

    fn with_elem<F>(card: i32, control: &str, mut f: F) -> bool
    where
        F: FnMut(&AlsaVtable, *mut c_void) -> bool,
    {
        let Some(vt) = vtable() else {
            return false;
        };
        let Ok(hw_name) = CString::new(format!("hw:{card}")) else {
            return false;
        };
        let Ok(ctrl_name) = CString::new(control) else {
            return false;
        };

        unsafe {
            struct Guard<'a> {
                vt: &'a AlsaVtable,
                mixer: *mut c_void,
                id: *mut c_void,
            }
            impl Drop for Guard<'_> {
                fn drop(&mut self) {
                    unsafe {
                        if !self.id.is_null() {
                            (self.vt.selem_id_free)(self.id);
                        }
                        if !self.mixer.is_null() {
                            (self.vt.mixer_close)(self.mixer);
                        }
                    }
                }
            }

            let mut guard = Guard {
                vt,
                mixer: ptr::null_mut(),
                id: ptr::null_mut(),
            };

            let mut mixer: *mut c_void = ptr::null_mut();
            if (vt.mixer_open)(&raw mut mixer, 0) < 0 || mixer.is_null() {
                return false;
            }
            guard.mixer = mixer;

            if (vt.mixer_attach)(guard.mixer, hw_name.as_ptr()) < 0 {
                return false;
            }
            if (vt.mixer_selem_register)(guard.mixer, ptr::null_mut(), ptr::null_mut()) < 0 {
                return false;
            }
            if (vt.mixer_load)(guard.mixer) < 0 {
                return false;
            }

            let mut id: *mut c_void = ptr::null_mut();
            if (vt.selem_id_malloc)(&raw mut id) < 0 || id.is_null() {
                return false;
            }
            guard.id = id;

            (vt.selem_id_set_index)(guard.id, 0);
            (vt.selem_id_set_name)(guard.id, ctrl_name.as_ptr());
            let elem = (vt.find_selem)(guard.mixer, guard.id);
            if elem.is_null() || (vt.has_playback_volume)(elem) == 0 {
                return false;
            }
            f(vt, elem)
        }
    }
}

#[derive(Default)]
struct HwWritePending {
    target: Option<(i32, String)>,
    pending_level: Option<f64>,
    pending_mute: Option<(bool, f64)>,
    last_written_at: Option<Instant>,
    shutdown: bool,
}

/// Asynchronous hardware mixer writer that coalesces rapid volume/mute updates
/// so dragging UI sliders or synced PipeWire sliders never stalls on `snd_mixer_*`.
pub struct CoalescedHwWriter {
    state: Arc<(Mutex<HwWritePending>, Condvar)>,
    has_pending: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl CoalescedHwWriter {
    pub fn new() -> Self {
        Self::with_sink(Arc::new(AlsaHardwareSink))
    }

    fn with_sink<S: HwMixerSink + Send + Sync + 'static>(sink: Arc<S>) -> Self {
        let state = Arc::new((Mutex::new(HwWritePending::default()), Condvar::new()));
        let has_pending = Arc::new(AtomicBool::new(false));
        let worker_state = Arc::clone(&state);
        let worker_pending = Arc::clone(&has_pending);

        let worker = thread::Builder::new()
            .name("retrograde-alsa-hw-writer".to_owned())
            .spawn(move || {
                let (lock, cvar) = &*worker_state;
                loop {
                    let (target, level, mute) = {
                        let mut guard = match lock.lock() {
                            Ok(g) => g,
                            Err(poisoned) => poisoned.into_inner(),
                        };
                        while !guard.shutdown
                            && guard.pending_level.is_none()
                            && guard.pending_mute.is_none()
                        {
                            worker_pending.store(false, Ordering::Release);
                            guard = match cvar.wait(guard) {
                                Ok(g) => g,
                                Err(poisoned) => poisoned.into_inner(),
                            };
                        }
                        if guard.shutdown
                            && guard.pending_level.is_none()
                            && guard.pending_mute.is_none()
                        {
                            break;
                        }
                        let target = guard.target.clone();
                        let level = guard.pending_level.take();
                        let mute = guard.pending_mute.take();
                        (target, level, mute)
                    };

                    if let Some((card, control)) = target {
                        let mut wrote = false;
                        if let Some(level) = level {
                            wrote |= sink.write_level(card, &control, level);
                        }
                        if let Some((muted, restore_level)) = mute {
                            wrote |= sink.set_mute(card, &control, muted, restore_level);
                        }
                        if wrote {
                            let mut guard = match lock.lock() {
                                Ok(g) => g,
                                Err(poisoned) => poisoned.into_inner(),
                            };
                            guard.last_written_at = Some(Instant::now());
                        }
                    }
                }
            })
            .ok();

        Self {
            state,
            has_pending,
            worker,
        }
    }

    pub fn push_level(&self, card: i32, control: impl Into<String>, level: f64) {
        if self.worker.is_none() {
            return;
        }
        let (lock, cvar) = &*self.state;
        let mut guard = match lock.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.target = Some((card, control.into()));
        guard.pending_level = Some(level.clamp(0.0, 1.0));
        self.has_pending.store(true, Ordering::Release);
        cvar.notify_one();
    }

    pub fn push_mute(
        &self,
        card: i32,
        control: impl Into<String>,
        muted: bool,
        restore_level: f64,
    ) {
        if self.worker.is_none() {
            return;
        }
        let (lock, cvar) = &*self.state;
        let mut guard = match lock.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.target = Some((card, control.into()));
        guard.pending_mute = Some((muted, restore_level.clamp(0.0, 1.0)));
        self.has_pending.store(true, Ordering::Release);
        cvar.notify_one();
    }

    /// True if a write happened within `window` or a write is currently queued.
    pub fn recently_wrote(&self, window: Duration) -> bool {
        if self.has_pending.load(Ordering::Acquire) {
            return true;
        }
        let (lock, _) = &*self.state;
        let guard = match lock.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard
            .last_written_at
            .is_some_and(|t| t.elapsed() < window)
    }
}

impl Default for CoalescedHwWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for CoalescedHwWriter {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.state;
        {
            let mut guard = match lock.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.shutdown = true;
            cvar.notify_one();
        }
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

trait HwMixerSink {
    fn write_level(&self, card: i32, control: &str, level: f64) -> bool;
    fn set_mute(&self, card: i32, control: &str, muted: bool, restore_level: f64) -> bool;
}

struct AlsaHardwareSink;

impl HwMixerSink for AlsaHardwareSink {
    fn write_level(&self, card: i32, control: &str, level: f64) -> bool {
        AlsaMixer::write_level(card, control, level)
    }

    fn set_mute(&self, card: i32, control: &str, muted: bool, restore_level: f64) -> bool {
        AlsaMixer::set_mute(card, control, muted, restore_level)
    }
}

pub fn db_to_level(min: i64, max: i64, db: i64) -> f64 {
    let lo = min.max(max - MAX_SPAN_CENTI_DB);
    if max <= lo {
        return 1.0;
    }
    ((db - lo) as f64 / (max - lo) as f64).clamp(0.0, 1.0)
}

pub fn level_to_db(min: i64, max: i64, level: f64) -> i64 {
    let level = level.clamp(0.0, 1.0);
    if level <= 0.0 {
        return min;
    }
    let lo = min.max(max - MAX_SPAN_CENTI_DB);
    lo + ((max - lo) as f64 * level).round() as i64
}

pub fn raw_to_level(min: i64, max: i64, raw: i64) -> f64 {
    if max <= min {
        return 1.0;
    }
    ((raw - min) as f64 / (max - min) as f64).clamp(0.0, 1.0)
}

pub fn level_to_raw(min: i64, max: i64, level: f64) -> i64 {
    min + ((max - min) as f64 * level.clamp(0.0, 1.0)).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_mapping_handles_30db_dac_and_wide_card_ranges() {
        // 30 dB DAC (-3000..0 centi-dB): full span is smaller than 60 dB so whole range maps 0..1.
        assert!((db_to_level(-3000, 0, -1800) - 0.4).abs() < 1e-6);
        assert_eq!(level_to_db(-3000, 0, 0.7), -900);
        assert_eq!(level_to_db(-3000, 0, 0.0), -3000);

        // Wide 90 dB card (-9000..0): only top 60 dB maps across 0..1, level 0 drops to min (-9000).
        assert!((db_to_level(-9000, 0, -3000) - 0.5).abs() < 1e-6);
        assert_eq!(level_to_db(-9000, 0, 0.5), -3000);
        assert_eq!(level_to_db(-9000, 0, 0.0), -9000);
    }

    #[test]
    fn coalesced_hw_writer_delivers_latest_level() {
        use std::sync::Barrier;

        struct MockSink {
            started: Barrier,
            release: Barrier,
            writes: Mutex<Vec<(i32, String, f64)>>,
        }
        impl HwMixerSink for MockSink {
            fn write_level(&self, card: i32, control: &str, level: f64) -> bool {
                if self.writes.lock().unwrap().is_empty() {
                    self.started.wait();
                    self.release.wait();
                }
                self.writes
                    .lock()
                    .unwrap()
                    .push((card, control.to_owned(), level));
                true
            }
            fn set_mute(&self, _: i32, _: &str, _: bool, _: f64) -> bool {
                true
            }
        }

        let sink = Arc::new(MockSink {
            started: Barrier::new(2),
            release: Barrier::new(2),
            writes: Mutex::new(Vec::new()),
        });
        let writer = CoalescedHwWriter::with_sink(Arc::clone(&sink));

        writer.push_level(0, "PCM", 0.10);
        sink.started.wait();
        // Rapid burst while first write is in progress; intermediates collapse to 0.72.
        writer.push_level(0, "PCM", 0.25);
        writer.push_level(0, "PCM", 0.45);
        writer.push_level(0, "PCM", 0.72);
        sink.release.wait();

        drop(writer);
        let writes = sink.writes.lock().unwrap();
        assert_eq!(writes.len(), 2);
        assert!((writes[0].2 - 0.10).abs() < 1e-6);
        assert!((writes[1].2 - 0.72).abs() < 1e-6);
    }
}
