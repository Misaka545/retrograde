use std::time::{Duration, Instant};

use retrograde_audio::{AudioPath, EngineEvent, MpvEngine};
use retrograde_core::Track;
use tokio::sync::mpsc;
use tracing::warn;

use crate::playback::CrossfadeMath;

struct ActiveFade {
    started_at: Instant,
    duration: Duration,
    target_track: Track,
    target_index: usize,
}

pub enum FadeStep {
    Inactive,
    InProgress,
    Completed,
}

/// Manages an auxiliary `MpvEngine` for smooth equal-power crossfades on shared outputs.
/// Never active during direct bit-perfect `alsa/hw:` playback.
pub struct CrossfadeManager {
    spare: Option<MpvEngine>,
    pub spare_rx: Option<mpsc::UnboundedReceiver<EngineEvent>>,
    spare_audio_path: Option<AudioPath>,
    spare_pos_s: Option<f64>,
    configured_device: String,
    configured_replay_gain: String,
    configured_filters: String,
    active: Option<ActiveFade>,
}

impl CrossfadeManager {
    pub fn new(device: String, replay_gain: String, filters: String) -> Self {
        Self {
            spare: None,
            spare_rx: None,
            spare_audio_path: None,
            spare_pos_s: None,
            configured_device: device,
            configured_replay_gain: replay_gain,
            configured_filters: filters,
            active: None,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    pub fn active_target(&self) -> Option<(&str, usize)> {
        self.active
            .as_ref()
            .map(|a| (a.target_track.id.as_str(), a.target_index))
    }

    pub fn active_target_path(&self) -> Option<&str> {
        self.active.as_ref().map(|a| a.target_track.file_path.as_str())
    }

    pub fn set_spare_audio_path(&mut self, ap: AudioPath) {
        self.spare_audio_path = Some(ap);
    }

    pub fn set_spare_position(&mut self, pos_s: f64) {
        self.spare_pos_s = Some(pos_s.max(0.0));
    }

    pub fn set_mute(&self, muted: bool) {
        if let Some(spare) = &self.spare {
            spare.set_mute(muted);
        }
    }

    pub fn update_eq_band_live(&mut self, band: usize, gains: &[f64]) {
        self.configured_filters = retrograde_audio::EqualizerConfig::build_filters(true, false, gains);
        if let Some(spare) = &mut self.spare {
            spare.update_eq_band_live(band, gains);
        }
    }

    pub fn update_eq_all_live(&mut self, gains: &[f64]) {
        self.configured_filters = retrograde_audio::EqualizerConfig::build_filters(true, false, gains);
        if let Some(spare) = &mut self.spare {
            spare.update_eq_all_live(gains);
        }
    }

    pub fn drop_dead_spare(&mut self) {
        self.spare = None;
        self.spare_rx = None;
        self.spare_audio_path = None;
        self.spare_pos_s = None;
    }

    pub fn sync_config(&mut self, device: &str, replay_gain: &str, filters: &str) {
        self.configured_device = device.to_owned();
        self.configured_replay_gain = replay_gain.to_owned();
        self.configured_filters = filters.to_owned();

        if let Some(spare) = &mut self.spare {
            spare.set_audio_device(device);
            spare.set_replay_gain(replay_gain);
            spare.set_audio_filters(filters);
        }
    }

    pub async fn begin_async(
        &mut self,
        next_track: Track,
        target_index: usize,
        duration: Duration,
        poll_interval: Duration,
        muted: bool,
    ) -> Result<(), String> {
        if duration.is_zero() {
            return Err("Crossfade disabled".to_owned());
        }
        let start_wall = Instant::now();
        if self.spare.is_none() {
            let dev = self.configured_device.clone();
            let rg = self.configured_replay_gain.clone();
            let filters = self.configured_filters.clone();
            let (eng, rx) = tokio::task::spawn_blocking(
                move || -> Result<(MpvEngine, mpsc::UnboundedReceiver<EngineEvent>), String> {
                    let (tx, rx) = mpsc::unbounded_channel();
                    let mut eng = MpvEngine::new(tx);
                    eng.set_poll_interval(poll_interval);
                    eng.start(Some(&dev), Some(&rg))
                        .map_err(|e| format!("Failed starting spare crossfade mpv: {e}"))?;
                    eng.set_audio_filters(&filters);
                    eng.set_volume(0.0);
                    eng.set_mute(muted);
                    Ok((eng, rx))
                },
            )
            .await
            .map_err(|e| format!("Spare crossfade init task failed: {e}"))??;
            self.spare = Some(eng);
            self.spare_rx = Some(rx);
        }

        let Some(spare) = &self.spare else {
            return Err("Spare engine unavailable".to_owned());
        };
        self.spare_audio_path = None;
        self.spare_pos_s = None;
        spare.set_poll_interval(poll_interval);
        spare.set_volume(0.0);
        spare.set_mute(muted);
        spare.play(&next_track.file_path, 0.0);

        let adjusted = duration
            .saturating_sub(start_wall.elapsed())
            .max(Duration::from_millis(200));
        self.active = Some(ActiveFade {
            started_at: Instant::now(),
            duration: adjusted,
            target_track: next_track,
            target_index,
        });
        Ok(())
    }

    pub fn step(&self, main: &MpvEngine, base_vol: f64) -> FadeStep {
        let Some(fade) = &self.active else {
            return FadeStep::Inactive;
        };
        let t = (fade.started_at.elapsed().as_secs_f64() / fade.duration.as_secs_f64())
            .clamp(0.0, 1.0);
        let (out_gain, in_gain) = CrossfadeMath::equal_power_gains(t, base_vol);
        main.set_volume(out_gain);
        if let Some(spare) = &self.spare {
            spare.set_volume(in_gain);
        }
        if t >= 1.0 {
            FadeStep::Completed
        } else {
            FadeStep::InProgress
        }
    }

    /// Promotes `spare` to `main` once the fade curve reaches 1.0 and stops the old main stream.
    pub fn complete_swap(
        &mut self,
        main: &mut MpvEngine,
        main_rx: &mut Option<mpsc::UnboundedReceiver<EngineEvent>>,
        base_vol: f64,
        muted: bool,
    ) -> Option<(Track, usize, Option<AudioPath>, Option<f64>)> {
        let fade = self.active.take()?;
        let mut spare = self.spare.take()?;
        let spare_rx = self.spare_rx.take();
        let spare_ap = self.spare_audio_path.take();
        let spare_pos = self.spare_pos_s.take();

        main.stop_playback();
        main.set_volume(0.0);
        std::mem::swap(main, &mut spare);

        let old_main_rx = std::mem::replace(main_rx, spare_rx);
        self.spare = Some(spare);
        self.spare_rx = old_main_rx;

        main.set_volume(base_vol.clamp(0.0, 1.0));
        main.set_mute(muted);

        Some((fade.target_track, fade.target_index, spare_ap, spare_pos))
    }

    pub fn cancel(&mut self, main: &MpvEngine, restore_vol: f64, muted: bool) {
        if self.active.take().is_some() {
            self.spare_audio_path = None;
            self.spare_pos_s = None;
            if let Some(spare) = &self.spare {
                spare.stop_playback();
                spare.set_volume(0.0);
            }
            main.set_volume(restore_vol.clamp(0.0, 1.0));
            main.set_mute(muted);
        }
    }

    pub fn set_poll_interval(&self, interval: Duration) {
        if let Some(spare) = &self.spare {
            spare.set_poll_interval(interval);
        }
    }

    pub async fn shutdown_async(&mut self) {
        self.active = None;
        self.spare_rx = None;
        self.spare_audio_path = None;
        self.spare_pos_s = None;
        if let Some(mut spare) = self.spare.take() {
            spare.shutdown_async().await;
            warn!(target: "retrograde_app::crossfade", "Shut down spare crossfade engine");
        }
    }
}
