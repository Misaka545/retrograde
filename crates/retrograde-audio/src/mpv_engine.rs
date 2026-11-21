use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use crate::alsa_direct::{AlsaDirect, AudioFormat};
use crate::libmpv::{LibMpvClient, LibMpvError, MpvEventMsg, MpvValue};

#[derive(Debug, Clone)]
pub struct AudioPath {
    pub input: Option<AudioFormat>,
    pub output: Option<AudioFormat>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDevice {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub enum EngineEvent {
    TimePos(f64),
    Duration(f64),
    Pause(bool),
    EndOfFile,
    PlaybackFailed { path: String, reason: String },
    Exited,
    AdvancedToNext(String),
    AudioPathChanged(AudioPath),
}

pub struct EqualizerConfig;

impl EqualizerConfig {
    pub const BAND_HZ: [i32; 10] = [32, 64, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
    pub const MAX_GAIN: f64 = 12.0;
    pub const PRESETS: [(&'static str, [f64; 10]); 6] = [
        ("FLAT", [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        ("BASS", [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        ("TREBLE", [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 4.0, 5.0, 6.0]),
        ("VOCAL", [-2.0, -2.0, -1.0, 1.0, 3.0, 4.0, 3.0, 1.0, 0.0, -1.0]),
        ("V-SHAPE", [5.0, 4.0, 2.0, 0.0, -2.0, -2.0, 0.0, 2.0, 4.0, 5.0]),
        ("WARM", [3.0, 3.0, 2.0, 1.0, 0.0, -1.0, -1.0, -2.0, -2.0, -3.0]),
    ];

    pub fn preamp_db(gains: &[f64]) -> f64 {
        let max_boost = gains
            .iter()
            .copied()
            .fold(0.0_f64, |acc, g| acc.max(g.clamp(-Self::MAX_GAIN, Self::MAX_GAIN)));
        -max_boost
    }

    pub fn build_filters(enabled: bool, bit_perfect: bool, gains: &[f64]) -> String {
        if !enabled || bit_perfect {
            return String::new();
        }
        let preamp = Self::preamp_db(gains);
        let mut parts = Vec::with_capacity(11);
        parts.push(format!("@eqpre:lavfi=[volume={}dB]", fmt_num(preamp)));
        for (i, hz) in Self::BAND_HZ.iter().enumerate() {
            let g = gains
                .get(i)
                .copied()
                .unwrap_or(0.0)
                .clamp(-Self::MAX_GAIN, Self::MAX_GAIN);
            parts.push(format!(
                "@eq{i}:lavfi=[equalizer=f={hz}:t=o:w=1:g={}]",
                fmt_num(g)
            ));
        }
        parts.join(",")
    }
}

fn fmt_num(v: f64) -> String {
    let s = format!("{v:.1}");
    s.strip_suffix(".0").unwrap_or(&s).to_owned()
}

struct SharedState {
    next_request_id: AtomicU64,
    load_request_id: AtomicU64,
    current_entry_id: AtomicI64,
    /// Playing = a file is loaded and mpv is not paused. Drives the time-pos poll.
    playing: AtomicBool,
    /// mpv's last reported `pause`.
    paused: AtomicBool,
    /// A file is loaded (`FileLoaded` seen, no `EndFile`/stop since). An idle mpv reports
    /// `pause=no` too; that is not playback.
    file_loaded: AtomicBool,
    current_path: Mutex<Option<String>>,
    pending_seek: Mutex<Option<f64>>,
    append_requests: Mutex<HashMap<u64, String>>,
    queued: Mutex<HashMap<i64, String>>,
    pending: Mutex<HashMap<u64, oneshot::Sender<MpvValue>>>,
    path_in: Mutex<Option<AudioFormat>>,
    path_out: Mutex<Option<AudioFormat>>,
    /// Bumped to cancel an end-of-file that is waiting to be reported (see `EOF_SETTLE`).
    eof_gen: AtomicU64,
}

impl SharedState {
    fn new() -> Self {
        Self {
            next_request_id: AtomicU64::new(2000),
            load_request_id: AtomicU64::new(0),
            current_entry_id: AtomicI64::new(-1),
            playing: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            file_loaded: AtomicBool::new(false),
            current_path: Mutex::new(None),
            pending_seek: Mutex::new(None),
            append_requests: Mutex::new(HashMap::new()),
            queued: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
            path_in: Mutex::new(None),
            path_out: Mutex::new(None),
            eof_gen: AtomicU64::new(0),
        }
    }
}

const TIME_POS_REQUEST_ID: u64 = 1000;

/// With `keep-open`, a file whose audio output fails to open reports `eof-reached` right before its
/// `end-file` error. The end of file waits this long so a failure is not taken for the track ending
/// (which would step the queue on and lose the position).
const EOF_SETTLE: Duration = Duration::from_millis(80);

/// mpv's text for a failed audio output (`MPV_ERROR_AO_INIT_FAILED`): the device, not the file.
pub const AO_INIT_FAILED: &str = "audio output initialization failed";

pub struct MpvEngine {
    lib: Option<Arc<LibMpvClient>>,
    shared: Arc<SharedState>,
    audio_device: String,
    replay_gain: String,
    audio_filters: String,
    poll_interval: Arc<Mutex<Duration>>,
    shutdown_poll: Option<tokio::sync::watch::Sender<bool>>,
    event_tx: mpsc::UnboundedSender<EngineEvent>,
}

impl MpvEngine {
    pub fn new(event_tx: mpsc::UnboundedSender<EngineEvent>) -> Self {
        Self {
            lib: None,
            shared: Arc::new(SharedState::new()),
            audio_device: "auto".to_owned(),
            replay_gain: "no".to_owned(),
            audio_filters: String::new(),
            poll_interval: Arc::new(Mutex::new(Duration::from_millis(500))),
            shutdown_poll: None,
            event_tx,
        }
    }

    pub fn is_running(&self) -> bool {
        self.lib.as_ref().is_some_and(|l| l.is_alive())
    }

    pub fn ao_for(device: &str) -> &'static str {
        if device == "null" {
            "null"
        } else if AlsaDirect::is_direct(device) {
            "alsa"
        } else {
            // Works across PipeWire, PulseAudio, and pure-ALSA Linux setups.
            "pipewire,pulse,alsa"
        }
    }

    pub fn normalize_replay_gain(mode: &str) -> &'static str {
        match mode {
            "track" => "track",
            "album" => "album",
            _ => "no",
        }
    }

    pub fn set_poll_interval(&self, interval: Duration) {
        if let Ok(mut g) = self.poll_interval.lock() {
            *g = interval.max(Duration::from_millis(100));
        }
    }

    pub fn start(
        &mut self,
        audio_device: Option<&str>,
        replay_gain: Option<&str>,
    ) -> Result<(), LibMpvError> {
        if self.is_running() {
            return Ok(());
        }
        self.stop_internal();

        self.audio_device = audio_device
            .filter(|s| !s.is_empty())
            .unwrap_or("auto")
            .to_owned();
        self.replay_gain = Self::normalize_replay_gain(replay_gain.unwrap_or("no")).to_owned();

        let ao = Self::ao_for(&self.audio_device);
        // RETRO_MPV_LOG=<file>: mpv's own verbose log, for output problems.
        let log_file = std::env::var("RETRO_MPV_LOG").ok().filter(|s| !s.is_empty());
        let mut options: Vec<(&str, &str)> = vec![
            ("config", "no"),
            ("load-scripts", "no"),
            ("ytdl", "no"),
            ("osc", "no"),
            ("load-stats-overlay", "no"),
            ("load-auto-profiles", "no"),
            ("idle", "yes"),
            ("ao", ao),
            ("audio-device", &self.audio_device),
            ("audio-client-name", "Retrograde"),
            ("input-media-keys", "no"),
            ("media-controls", "no"),
            ("video", "no"),
            ("vid", "no"),
            ("audio-display", "no"),
            ("force-window", "no"),
            ("keep-open", "yes"),
            ("gapless-audio", "weak"),
            ("prefetch-playlist", "yes"),
            ("replaygain", &self.replay_gain),
            ("af", &self.audio_filters),
            ("terminal", "no"),
            ("msg-level", "all=no"),
        ];
        if let Some(path) = log_file.as_deref() {
            options.push(("log-file", path));
            options.push(("msg-level", "all=v"));
        }

        let (raw_tx, mut raw_rx) = mpsc::unbounded_channel();
        let client = Arc::new(LibMpvClient::start(&options, raw_tx)?);
        client.observe(2, "pause");
        client.observe(3, "eof-reached");
        client.observe(4, "duration");
        client.observe(5, "audio-params");
        client.observe(6, "audio-out-params");

        self.lib = Some(Arc::clone(&client));

        // Event dispatcher task holds a Weak reference to break the Arc<LibMpvClient> cycle.
        let shared = Arc::clone(&self.shared);
        let out_tx = self.event_tx.clone();
        let lib_for_seek = Arc::downgrade(&client);
        tokio::spawn(async move {
            while let Some(msg) = raw_rx.recv().await {
                handle_raw_message(msg, &shared, &out_tx, &lib_for_seek);
            }
        });

        // Position polling task
        let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
        self.shutdown_poll = Some(stop_tx);
        let shared_poll = Arc::clone(&self.shared);
        let poll_interval = Arc::clone(&self.poll_interval);
        let lib_poll = Arc::clone(&client);
        tokio::spawn(async move {
            loop {
                let wait = poll_interval
                    .lock()
                    .map(|g| *g)
                    .unwrap_or(Duration::from_millis(500));
                tokio::select! {
                    _ = tokio::time::sleep(wait) => {
                        if shared_poll.playing.load(Ordering::Acquire) {
                            lib_poll.get_property(TIME_POS_REQUEST_ID, "time-pos");
                        }
                    }
                    _ = stop_rx.changed() => {
                        if *stop_rx.borrow() {
                            break;
                        }
                    }
                }
            }
        });

        Ok(())
    }

    pub fn play(&self, file_path: &str, start_seconds: f64) {
        self.shared.current_entry_id.store(-1, Ordering::Release);
        // An end of file still settling belongs to the file being replaced.
        self.shared.eof_gen.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut cp) = self.shared.current_path.lock() {
            *cp = Some(file_path.to_owned());
        }
        if !Path::new(file_path).is_absolute() {
            let _ = self.event_tx.send(EngineEvent::PlaybackFailed {
                path: file_path.to_owned(),
                reason: "not a local file".to_owned(),
            });
            return;
        }
        if let Ok(mut ar) = self.shared.append_requests.lock() {
            ar.clear();
        }
        if let Ok(mut q) = self.shared.queued.lock() {
            q.clear();
        }
        if let Ok(mut ps) = self.shared.pending_seek.lock() {
            *ps = if start_seconds > 0.0 {
                Some(start_seconds)
            } else {
                None
            };
        }
        let id = self.shared.next_request_id.fetch_add(1, Ordering::Relaxed);
        self.shared.load_request_id.store(id, Ordering::Release);
        if let Some(lib) = &self.lib {
            lib.command(id, &["loadfile", file_path, "replace"]);
            lib.set_property("pause", "no");
        }
    }

    pub fn set_next(&self, path: Option<&str>) {
        if let Ok(mut ar) = self.shared.append_requests.lock() {
            ar.clear();
        }
        if let Some(lib) = &self.lib {
            lib.command(0, &["playlist-clear"]);
            let Some(p) = path else {
                return;
            };
            if !Path::new(p).is_absolute() {
                return;
            }
            if let Ok(mut q) = self.shared.queued.lock()
                && q.len() > 64
            {
                q.clear();
            }
            let id = self.shared.next_request_id.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut ar) = self.shared.append_requests.lock() {
                ar.insert(id, p.to_owned());
            }
            lib.command(id, &["loadfile", p, "append"]);
        }
    }

    pub fn set_replay_gain(&mut self, mode: &str) {
        self.replay_gain = Self::normalize_replay_gain(mode).to_owned();
        if let Some(lib) = &self.lib {
            lib.set_property("replaygain", &self.replay_gain);
        }
    }

    pub fn set_paused(&self, paused: bool) {
        if let Some(lib) = &self.lib {
            lib.set_property("pause", if paused { "yes" } else { "no" });
        }
    }

    pub fn toggle_pause(&self) {
        if let Some(lib) = &self.lib {
            lib.command(0, &["cycle", "pause"]);
        }
    }

    pub fn seek(&self, seconds: f64) {
        if let Some(lib) = &self.lib {
            let s = seconds.max(0.0).to_string();
            lib.command(0, &["seek", &s, "absolute"]);
            lib.get_property(TIME_POS_REQUEST_ID, "time-pos");
        }
    }

    pub fn set_volume(&self, volume01: f64) {
        if let Some(lib) = &self.lib {
            let v = (volume01.clamp(0.0, 1.0) * 100.0).to_string();
            lib.set_property("volume", &v);
        }
    }

    pub fn set_mute(&self, muted: bool) {
        if let Some(lib) = &self.lib {
            lib.set_property("mute", if muted { "yes" } else { "no" });
        }
    }

    pub fn stop_playback(&self) {
        self.shared.current_entry_id.store(-1, Ordering::Release);
        self.shared.file_loaded.store(false, Ordering::Release);
        self.shared.playing.store(false, Ordering::Release);
        if let Some(lib) = &self.lib {
            lib.command(0, &["stop"]);
        }
    }

    pub fn set_audio_filters(&mut self, filters: &str) {
        if self.audio_filters == filters {
            return;
        }
        self.audio_filters = filters.to_owned();
        if let Some(lib) = &self.lib {
            lib.set_property("af", filters);
        }
    }

    pub fn filter_command(&self, label: &str, target: &str, command: &str, argument: &str) {
        if let Some(lib) = &self.lib {
            lib.command(0, &["af-command", label, command, argument, target]);
        }
    }

    pub fn update_eq_band_live(&mut self, band: usize, gains: &[f64]) {
        let next_filters = EqualizerConfig::build_filters(true, false, gains);
        if self.audio_filters.is_empty() {
            self.set_audio_filters(&next_filters);
            return;
        }
        let preamp = EqualizerConfig::preamp_db(gains);
        self.filter_command("eqpre", "volume", "volume", &format!("{}dB", fmt_num(preamp)));
        let g = gains
            .get(band)
            .copied()
            .unwrap_or(0.0)
            .clamp(-EqualizerConfig::MAX_GAIN, EqualizerConfig::MAX_GAIN);
        self.filter_command(&format!("eq{band}"), "equalizer", "g", &fmt_num(g));
        self.audio_filters = next_filters;
    }

    pub fn update_eq_all_live(&mut self, gains: &[f64]) {
        let next_filters = EqualizerConfig::build_filters(true, false, gains);
        if self.audio_filters.is_empty() {
            self.set_audio_filters(&next_filters);
            return;
        }
        let preamp = EqualizerConfig::preamp_db(gains);
        self.filter_command("eqpre", "volume", "volume", &format!("{}dB", fmt_num(preamp)));
        for i in 0..EqualizerConfig::BAND_HZ.len() {
            let g = gains
                .get(i)
                .copied()
                .unwrap_or(0.0)
                .clamp(-EqualizerConfig::MAX_GAIN, EqualizerConfig::MAX_GAIN);
            self.filter_command(&format!("eq{i}"), "equalizer", "g", &fmt_num(g));
        }
        self.audio_filters = next_filters;
    }

    pub fn set_audio_device(&mut self, name: &str) {
        if self.audio_device == name {
            return;
        }
        self.audio_device = name.to_owned();
        if let Ok(mut out) = self.shared.path_out.lock() {
            *out = None;
        }
        if let Some(lib) = &self.lib {
            lib.set_property("ao", Self::ao_for(name));
            lib.set_property("audio-device", name);
            lib.command(0, &["ao-reload"]);
        }
    }

    pub fn reload_audio(&self) {
        if let Ok(mut out) = self.shared.path_out.lock() {
            *out = None;
        }
        if let Some(lib) = &self.lib {
            lib.command(0, &["ao-reload"]);
        }
    }

    async fn get_property_async(&self, name: &str) -> Option<MpvValue> {
        let lib = self.lib.as_ref()?;
        let id = self.shared.next_request_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        if let Ok(mut p) = self.shared.pending.lock() {
            p.insert(id, tx);
        }
        lib.get_property(id, name);
        match tokio::time::timeout(Duration::from_secs(2), rx).await {
            Ok(Ok(val)) => Some(val),
            _ => {
                if let Ok(mut p) = self.shared.pending.lock() {
                    p.remove(&id);
                }
                None
            }
        }
    }

    /// Output devices reported by mpv for PipeWire / PulseAudio (`"auto"` included).
    pub async fn get_audio_devices(&self) -> Vec<AudioDevice> {
        let mut list = Vec::new();
        let Some(MpvValue::Array(arr)) = self.get_property_async("audio-device-list").await else {
            return list;
        };
        for item in arr {
            let Some(map) = item.as_map() else {
                continue;
            };
            let Some(name) = map.get("name").and_then(MpvValue::as_str) else {
                continue;
            };
            if name != "auto"
                && !name.starts_with("pipewire/")
                && !name.starts_with("pulse/")
            {
                continue;
            }
            let desc = map
                .get("description")
                .and_then(MpvValue::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or(name);
            list.push(AudioDevice {
                name: name.to_owned(),
                description: desc.to_owned(),
            });
        }
        list
    }

    /// Confirms that nothing inside mpv alters samples (`af`, `volume == 100`, `replaygain == no`, `speed == 1`).
    pub async fn get_sample_alteration(&self) -> Option<String> {
        let Some(MpvValue::Array(af)) = self.get_property_async("af").await else {
            return Some("MPV DID NOT CONFIRM ITS SETTINGS".to_owned());
        };
        if !af.is_empty() {
            return Some("AN AUDIO FILTER IS ON".to_owned());
        }
        let Some(vol) = self
            .get_property_async("volume")
            .await
            .and_then(|v| v.as_f64())
        else {
            return Some("MPV DID NOT CONFIRM ITS SETTINGS".to_owned());
        };
        if (vol - 100.0).abs() > 0.01 {
            return Some("SOFTWARE VOLUME IS ON".to_owned());
        }
        match self.get_property_async("replaygain").await {
            Some(MpvValue::Bool(false)) => {}
            Some(MpvValue::String(s)) if s == "no" => {}
            Some(MpvValue::String(_)) => return Some("REPLAYGAIN IS ON".to_owned()),
            _ => return Some("MPV DID NOT CONFIRM ITS SETTINGS".to_owned()),
        }
        let Some(speed) = self
            .get_property_async("speed")
            .await
            .and_then(|v| v.as_f64())
        else {
            return Some("MPV DID NOT CONFIRM ITS SETTINGS".to_owned());
        };
        if (speed - 1.0).abs() > 0.0001 {
            return Some("PLAYBACK SPEED IS CHANGED".to_owned());
        }
        None
    }

    fn stop_internal(&mut self) {
        if let Some(tx) = self.shutdown_poll.take() {
            let _ = tx.send(true);
        }
        self.shared.playing.store(false, Ordering::Release);
        self.shared.file_loaded.store(false, Ordering::Release);
        self.shared.paused.store(false, Ordering::Release);
        self.shared.current_entry_id.store(-1, Ordering::Release);
        if let Ok(mut p) = self.shared.pending.lock() {
            p.clear();
        }
        if let Some(lib) = self.lib.take() {
            lib.shutdown();
        }
    }

    pub fn shutdown(&mut self) {
        self.stop_internal();
    }

    pub async fn shutdown_async(&mut self) {
        if let Some(tx) = self.shutdown_poll.take() {
            let _ = tx.send(true);
        }
        self.shared.playing.store(false, Ordering::Release);
        self.shared.current_entry_id.store(-1, Ordering::Release);
        if let Ok(mut p) = self.shared.pending.lock() {
            p.clear();
        }
        if let Some(lib) = self.lib.take() {
            let _ = tokio::task::spawn_blocking(move || lib.shutdown()).await;
        }
    }
}

impl Drop for MpvEngine {
    fn drop(&mut self) {
        self.stop_internal();
    }
}

fn handle_raw_message(
    msg: MpvEventMsg,
    shared: &Arc<SharedState>,
    out_tx: &mpsc::UnboundedSender<EngineEvent>,
    lib: &Weak<LibMpvClient>,
) {
    match msg {
        MpvEventMsg::CommandReply {
            request_id, data, ..
        } => {
            if request_id == shared.load_request_id.load(Ordering::Acquire) {
                if let Some(entry) = data
                    .as_map()
                    .and_then(|m| m.get("playlist_entry_id"))
                    .and_then(MpvValue::as_i64)
                {
                    shared.current_entry_id.store(entry, Ordering::Release);
                }
                return;
            }
            if let Ok(mut ar) = shared.append_requests.lock()
                && let Some(appended) = ar.remove(&request_id)
            {
                if let Some(entry) = data
                    .as_map()
                    .and_then(|m| m.get("playlist_entry_id"))
                    .and_then(MpvValue::as_i64)
                    && let Ok(mut q) = shared.queued.lock()
                {
                    q.insert(entry, appended);
                }
                return;
            }
            if let Ok(mut p) = shared.pending.lock()
                && let Some(tx) = p.remove(&request_id)
            {
                let _ = tx.send(data);
            }
        }
        MpvEventMsg::GetPropertyReply {
            request_id, data, ..
        } => {
            if request_id == TIME_POS_REQUEST_ID {
                if let Some(pos) = data.as_f64() {
                    let _ = out_tx.send(EngineEvent::TimePos(pos));
                }
                return;
            }
            if let Ok(mut p) = shared.pending.lock()
                && let Some(tx) = p.remove(&request_id)
            {
                let _ = tx.send(data);
            }
        }
        MpvEventMsg::FileLoaded => {
            shared.file_loaded.store(true, Ordering::Release);
            let paused = shared.paused.load(Ordering::Acquire);
            shared.playing.store(!paused, Ordering::Release);
            // mpv reports `pause` only when it changes; a file starting under an unchanged
            // pause=no would otherwise never be announced as playing.
            let _ = out_tx.send(EngineEvent::Pause(paused));
            let seek = shared
                .pending_seek
                .lock()
                .ok()
                .and_then(|mut g| g.take());
            if let Some(sec) = seek
                && let Some(client) = lib.upgrade()
            {
                let s = sec.to_string();
                client.command(0, &["seek", &s, "absolute"]);
                client.get_property(TIME_POS_REQUEST_ID, "time-pos");
            }
        }
        MpvEventMsg::StartFile { playlist_entry_id } => {
            let next_path = shared
                .queued
                .lock()
                .ok()
                .and_then(|mut q| q.remove(&playlist_entry_id));
            if let Some(path) = next_path {
                shared
                    .current_entry_id
                    .store(playlist_entry_id, Ordering::Release);
                if let Ok(mut cp) = shared.current_path.lock() {
                    *cp = Some(path.clone());
                }
                let _ = out_tx.send(EngineEvent::AdvancedToNext(path));
            }
        }
        MpvEventMsg::EndFile {
            reason,
            playlist_entry_id,
            file_error,
        } => {
            shared.file_loaded.store(false, Ordering::Release);
            shared.playing.store(false, Ordering::Release);
            if reason == "error" {
                // The `eof-reached` just before this was the failure, not the end of the track.
                shared.eof_gen.fetch_add(1, Ordering::AcqRel);
            }
            if reason == "error"
                && playlist_entry_id == shared.current_entry_id.load(Ordering::Acquire)
            {
                shared.current_entry_id.store(-1, Ordering::Release);
                let path = shared
                    .current_path
                    .lock()
                    .ok()
                    .and_then(|g| g.clone())
                    .unwrap_or_default();
                let _ = out_tx.send(EngineEvent::PlaybackFailed {
                    path,
                    reason: file_error.unwrap_or_else(|| "error".to_owned()),
                });
            }
        }
        MpvEventMsg::PropertyChange { name, data } => {
            if matches!(data, MpvValue::None) {
                return;
            }
            match name.as_str() {
                "audio-params" => {
                    let fmt = parse_audio_format(&data);
                    let out = shared.path_out.lock().ok().and_then(|g| g.clone());
                    if let Ok(mut g) = shared.path_in.lock() {
                        *g = fmt.clone();
                    }
                    let _ = out_tx.send(EngineEvent::AudioPathChanged(AudioPath {
                        input: fmt,
                        output: out,
                    }));
                }
                "audio-out-params" => {
                    let fmt = parse_audio_format(&data);
                    let input = shared.path_in.lock().ok().and_then(|g| g.clone());
                    if let Ok(mut g) = shared.path_out.lock() {
                        *g = fmt.clone();
                    }
                    let _ = out_tx.send(EngineEvent::AudioPathChanged(AudioPath {
                        input,
                        output: fmt,
                    }));
                }
                "duration" => {
                    if let Some(d) = data.as_f64() {
                        let _ = out_tx.send(EngineEvent::Duration(d));
                    }
                }
                "pause" => {
                    if let Some(paused) = data.as_bool() {
                        shared.paused.store(paused, Ordering::Release);
                        let loaded = shared.file_loaded.load(Ordering::Acquire);
                        shared.playing.store(!paused && loaded, Ordering::Release);
                        let _ = out_tx.send(EngineEvent::Pause(paused));
                    }
                }
                "eof-reached" if data.as_bool() == Some(true) => {
                    let generation = shared.eof_gen.fetch_add(1, Ordering::AcqRel) + 1;
                    let shared = Arc::clone(shared);
                    let tx = out_tx.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(EOF_SETTLE).await;
                        if shared.eof_gen.load(Ordering::Acquire) == generation {
                            let _ = tx.send(EngineEvent::EndOfFile);
                        }
                    });
                }
                _ => {}
            }
        }
        MpvEventMsg::Shutdown { unexpected } => {
            if unexpected {
                shared.playing.store(false, Ordering::Release);
                shared.file_loaded.store(false, Ordering::Release);
                shared.current_entry_id.store(-1, Ordering::Release);
                let _ = out_tx.send(EngineEvent::Exited);
            }
        }
    }
}

fn parse_audio_format(val: &MpvValue) -> Option<AudioFormat> {
    let map = val.as_map()?;
    let format = map.get("format").and_then(MpvValue::as_str)?.to_owned();
    let rate = map.get("samplerate").and_then(MpvValue::as_i64)? as i32;
    let channels = map
        .get("channel-count")
        .and_then(MpvValue::as_i64)
        .unwrap_or(0) as i32;
    if rate <= 0 {
        None
    } else {
        Some(AudioFormat {
            format,
            rate,
            channels,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equalizer_builds_expected_lavfi_chain() {
        assert_eq!(EqualizerConfig::build_filters(false, false, &[5.0]), "");
        assert_eq!(EqualizerConfig::build_filters(true, true, &[5.0]), "");
        let chain = EqualizerConfig::build_filters(true, false, &[6.0, 0.0, -3.5]);
        assert!(chain.starts_with("@eqpre:lavfi=[volume=-6dB],@eq0:lavfi=[equalizer=f=32:t=o:w=1:g=6]"));
        assert!(chain.contains("@eq2:lavfi=[equalizer=f=125:t=o:w=1:g=-3.5]"));
    }

    #[test]
    fn ao_selection_matches_device_type() {
        assert_eq!(MpvEngine::ao_for("null"), "null");
        assert_eq!(MpvEngine::ao_for("alsa/hw:CARD=Pro,DEV=0"), "alsa");
        assert_eq!(MpvEngine::ao_for("auto"), "pipewire,pulse,alsa");
        assert_eq!(MpvEngine::ao_for("pipewire/foo"), "pipewire,pulse,alsa");
    }

    #[test]
    #[ignore = "Requires libmpv and ffmpeg installed on host"]
    fn live_silent_flac_has_no_sample_alteration() {
        use std::process::Command;

        let path = "/tmp/retrograde-silent-test.flac";
        let ffmpeg_status = Command::new("ffmpeg")
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=44100:cl=stereo",
                "-t",
                "1",
                "-c:a",
                "flac",
                path,
            ])
            .status()
            .expect("run ffmpeg");
        assert!(ffmpeg_status.success(), "ffmpeg failed generating silent FLAC");

        struct Cleanup<'a>(&'a str);
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(self.0);
            }
        }
        let _cleanup = Cleanup(path);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let (tx, mut rx) = mpsc::unbounded_channel();
            let mut engine = MpvEngine::new(tx);
            engine
                .start(Some("null"), Some("no"))
                .expect("start libmpv with ao=null");
            engine.set_volume(1.0);
            engine.play(path, 0.0);

            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            let mut input_fmt: Option<AudioFormat> = None;
            while tokio::time::Instant::now() < deadline {
                if let Ok(Some(EngineEvent::AudioPathChanged(ap))) =
                    tokio::time::timeout(Duration::from_millis(250), rx.recv()).await
                    && let Some(inp) = ap.input
                {
                    input_fmt = Some(inp);
                    break;
                }
            }
            let fmt = input_fmt.expect("expected AudioPathChanged input format for silent FLAC");
            assert_eq!(fmt.rate, 44100);
            assert_eq!(fmt.channels, 2);

            let altered = engine.get_sample_alteration().await;
            assert_eq!(altered, None, "unexpected mpv sample alteration: {altered:?}");
            engine.shutdown();
        });
    }
}
