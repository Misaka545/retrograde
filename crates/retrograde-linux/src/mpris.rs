use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, RwLock};

use tokio::sync::mpsc;
use tracing::{debug, warn};
use zbus::object_server::SignalEmitter;
use zbus::{ObjectPath, OwnedObjectPath, OwnedVariant, Variant};

#[derive(Debug, Clone, PartialEq)]
pub struct MprisState {
    pub playback_status: String,
    pub track_id: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub art_url: Option<String>,
    pub length_us: i64,
    pub position_us: i64,
    pub volume: f64,
    pub can_go_next: bool,
    pub can_go_previous: bool,
    pub can_play: bool,
    pub can_pause: bool,
    pub can_seek: bool,
    pub loop_status: String,
    pub shuffle: bool,
}

impl Default for MprisState {
    fn default() -> Self {
        Self {
            playback_status: "Stopped".to_owned(),
            track_id: None,
            title: None,
            artist: None,
            album: None,
            art_url: None,
            length_us: 0,
            position_us: 0,
            volume: 0.5,
            can_go_next: true,
            can_go_previous: true,
            can_play: true,
            can_pause: true,
            can_seek: true,
            loop_status: "None".to_owned(),
            shuffle: false,
        }
    }
}

/// A valid, unique MPRIS track id for a track id (its file path): a hash under the app's own
/// namespace, since the spec reserves `/org/mpris` (C# used `/org/retrograde/track/...` too).
pub fn track_object_path(track_id: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in track_id.as_bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("/org/retrograde/track/t{hash:016x}")
}

/// Positions further apart than this are a jump (seek, restart) and get a `Seeked` signal.
const SEEK_JUMP_US: i64 = 2_000_000;

impl MprisState {
    fn metadata_map(&self) -> HashMap<String, OwnedVariant> {
        let mut map = HashMap::new();
        let Some(id) = self.track_id.as_deref() else {
            return map;
        };
        let obj_str = track_object_path(id);
        if let Ok(path) = ObjectPath::try_from(obj_str.as_str())
            && let Ok(val) = OwnedVariant::try_from(Variant::from(path))
        {
            map.insert("mpris:trackid".to_owned(), val);
        }
        if let Some(title) = self.title.as_deref()
            && !title.is_empty()
            && let Ok(val) = OwnedVariant::try_from(Variant::from(title))
        {
            map.insert("xesam:title".to_owned(), val);
        }
        if let Some(artist) = self.artist.as_deref()
            && !artist.is_empty()
            && let Ok(val) = OwnedVariant::try_from(Variant::from(vec![artist.to_owned()]))
        {
            map.insert("xesam:artist".to_owned(), val);
        }
        if let Some(album) = self.album.as_deref()
            && !album.is_empty()
            && let Ok(val) = OwnedVariant::try_from(Variant::from(album))
        {
            map.insert("xesam:album".to_owned(), val);
        }
        if let Some(art_url) = self.art_url.as_deref()
            && !art_url.is_empty()
            && let Ok(val) = OwnedVariant::try_from(Variant::from(art_url))
        {
            map.insert("mpris:artUrl".to_owned(), val);
        }
        if self.length_us > 0
            && let Ok(val) = OwnedVariant::try_from(Variant::from(self.length_us))
        {
            map.insert("mpris:length".to_owned(), val);
        }
        map
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MprisCommand {
    Play,
    Pause,
    PlayPause,
    Stop,
    Next,
    Previous,
    SeekOffsetUs(i64),
    SetPositionUs(i64),
    SetVolume(f64),
    SetLoopStatus(String),
    SetShuffle(bool),
    Raise,
    Quit,
}

struct MediaPlayer2Root {
    cmd_tx: mpsc::UnboundedSender<MprisCommand>,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl MediaPlayer2Root {
    #[zbus(property)]
    async fn can_quit(&self) -> bool {
        true
    }

    #[zbus(property)]
    async fn can_raise(&self) -> bool {
        true
    }

    #[zbus(property)]
    async fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn identity(&self) -> &str {
        "Retrograde"
    }

    #[zbus(property)]
    async fn desktop_entry(&self) -> &str {
        "retrograde"
    }

    #[zbus(property)]
    async fn supported_uri_schemes(&self) -> Vec<String> {
        vec!["file".to_owned()]
    }

    #[zbus(property)]
    async fn supported_mime_types(&self) -> Vec<String> {
        vec![
            "audio/flac".to_owned(),
            "audio/mpeg".to_owned(),
            "audio/ogg".to_owned(),
            "audio/mp4".to_owned(),
            "audio/wav".to_owned(),
            "audio/x-wav".to_owned(),
            "audio/opus".to_owned(),
        ]
    }

    async fn raise(&self) {
        let _ = self.cmd_tx.send(MprisCommand::Raise);
    }

    async fn quit(&self) {
        let _ = self.cmd_tx.send(MprisCommand::Quit);
    }
}

struct MediaPlayer2Player {
    state: Arc<RwLock<MprisState>>,
    /// Live playback position, updated on every poll without a PropertiesChanged signal
    /// (clients read Position on demand, as the spec intends).
    position_us: Arc<AtomicI64>,
    cmd_tx: mpsc::UnboundedSender<MprisCommand>,
}

impl MediaPlayer2Player {
    fn snapshot(&self) -> MprisState {
        match self.state.read() {
            Ok(g) => g.clone(),
            Err(p) => p.into_inner().clone(),
        }
    }
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl MediaPlayer2Player {
    #[zbus(property)]
    async fn playback_status(&self) -> String {
        self.snapshot().playback_status
    }

    #[zbus(property)]
    async fn loop_status(&self) -> String {
        self.snapshot().loop_status
    }

    #[zbus(property)]
    async fn set_loop_status(&self, status: String) {
        let _ = self.cmd_tx.send(MprisCommand::SetLoopStatus(status));
    }

    #[zbus(property)]
    async fn rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    async fn set_rate(&self, _rate: f64) {}

    #[zbus(property)]
    async fn shuffle(&self) -> bool {
        self.snapshot().shuffle
    }

    #[zbus(property)]
    async fn set_shuffle(&self, shuffle: bool) {
        let _ = self.cmd_tx.send(MprisCommand::SetShuffle(shuffle));
    }

    #[zbus(property)]
    async fn metadata(&self) -> HashMap<String, OwnedVariant> {
        self.snapshot().metadata_map()
    }

    #[zbus(property)]
    async fn volume(&self) -> f64 {
        self.snapshot().volume
    }

    #[zbus(property)]
    async fn set_volume(&self, volume: f64) {
        let _ = self
            .cmd_tx
            .send(MprisCommand::SetVolume(volume.clamp(0.0, 1.0)));
    }

    #[zbus(property)]
    async fn position(&self) -> i64 {
        self.position_us.load(Ordering::Relaxed)
    }

    #[zbus(property)]
    async fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    async fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    async fn can_go_next(&self) -> bool {
        self.snapshot().can_go_next
    }

    #[zbus(property)]
    async fn can_go_previous(&self) -> bool {
        self.snapshot().can_go_previous
    }

    #[zbus(property)]
    async fn can_play(&self) -> bool {
        self.snapshot().can_play
    }

    #[zbus(property)]
    async fn can_pause(&self) -> bool {
        self.snapshot().can_pause
    }

    #[zbus(property)]
    async fn can_seek(&self) -> bool {
        self.snapshot().can_seek
    }

    #[zbus(property)]
    async fn can_control(&self) -> bool {
        true
    }

    async fn next(&self) {
        let _ = self.cmd_tx.send(MprisCommand::Next);
    }

    async fn previous(&self) {
        let _ = self.cmd_tx.send(MprisCommand::Previous);
    }

    async fn pause(&self) {
        let _ = self.cmd_tx.send(MprisCommand::Pause);
    }

    async fn play_pause(&self) {
        let _ = self.cmd_tx.send(MprisCommand::PlayPause);
    }

    async fn stop(&self) {
        let _ = self.cmd_tx.send(MprisCommand::Stop);
    }

    async fn play(&self) {
        let _ = self.cmd_tx.send(MprisCommand::Play);
    }

    async fn seek(&self, offset: i64) {
        let _ = self.cmd_tx.send(MprisCommand::SeekOffsetUs(offset));
    }

    async fn set_position(&self, _track_id: OwnedObjectPath, position: i64) {
        let _ = self.cmd_tx.send(MprisCommand::SetPositionUs(position));
    }

    async fn open_uri(&self, _uri: String) {}

    #[zbus(signal)]
    async fn seeked(emitter: &SignalEmitter<'_>, position: i64) -> zbus::Result<()>;
}

const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const MPRIS_BUS_NAME: &str = "org.mpris.MediaPlayer2.retrograde";

pub struct MprisService {
    conn: zbus::Connection,
    state: Arc<RwLock<MprisState>>,
    position_us: Arc<AtomicI64>,
}

impl MprisService {
    pub async fn start() -> Result<(Self, mpsc::UnboundedReceiver<MprisCommand>), String> {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let state = Arc::new(RwLock::new(MprisState::default()));
        let position_us = Arc::new(AtomicI64::new(0));

        let root = MediaPlayer2Root {
            cmd_tx: cmd_tx.clone(),
        };
        let player = MediaPlayer2Player {
            state: Arc::clone(&state),
            position_us: Arc::clone(&position_us),
            cmd_tx,
        };

        let conn = zbus::connection::Builder::session()
            .name(MPRIS_BUS_NAME)
            .serve_at(MPRIS_PATH, root)
            .serve_at(MPRIS_PATH, player)
            .build()
            .await
            .map_err(|e| format!("MPRIS D-Bus connection failed: {e}"))?;

        debug!(bus_name = MPRIS_BUS_NAME, "MPRIS service started");
        Ok((
            Self {
                conn,
                state,
                position_us,
            },
            cmd_rx,
        ))
    }

    /// Updates the live position. Emits `Seeked` only when it jumped by more than 2 s
    /// (a seek or restart); ordinary playback progress is silent.
    pub async fn update_position(&self, position_us: i64) {
        let prev = self.position_us.swap(position_us, Ordering::Relaxed);
        if (position_us - prev).abs() > SEEK_JUMP_US {
            self.emit_seeked(position_us).await;
        }
    }

    /// An explicit seek: stores the position and always emits `Seeked`.
    pub async fn seeked_to(&self, position_us: i64) {
        self.position_us.store(position_us, Ordering::Relaxed);
        self.emit_seeked(position_us).await;
    }

    pub async fn update_state(&self, next: MprisState) {
        self.position_us.store(next.position_us, Ordering::Relaxed);
        let prev = {
            let mut guard = match self.state.write() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            let prev = guard.clone();
            *guard = next.clone();
            prev
        };

        let Ok(iface) = self
            .conn
            .object_server()
            .interface::<_, MediaPlayer2Player>(MPRIS_PATH)
            .await
        else {
            return;
        };
        let emitter = iface.signal_emitter();
        let player = iface.get().await;

        if prev.playback_status != next.playback_status
            && let Err(e) = player.playback_status_changed(emitter).await
        {
            warn!(error = %e, "Failed emitting MPRIS PlaybackStatus change");
        }
        if prev.track_id != next.track_id
            || prev.title != next.title
            || prev.artist != next.artist
            || prev.album != next.album
            || prev.art_url != next.art_url
            || prev.length_us != next.length_us
        {
            let _ = player.metadata_changed(emitter).await;
        }
        if (prev.volume - next.volume).abs() > 0.002 {
            let _ = player.volume_changed(emitter).await;
        }
        if prev.loop_status != next.loop_status {
            let _ = player.loop_status_changed(emitter).await;
        }
        if prev.shuffle != next.shuffle {
            let _ = player.shuffle_changed(emitter).await;
        }
        if prev.can_go_next != next.can_go_next {
            let _ = player.can_go_next_changed(emitter).await;
        }
        if prev.can_go_previous != next.can_go_previous {
            let _ = player.can_go_previous_changed(emitter).await;
        }
    }

    pub async fn emit_seeked(&self, position_us: i64) {
        if let Ok(iface) = self
            .conn
            .object_server()
            .interface::<_, MediaPlayer2Player>(MPRIS_PATH)
            .await
        {
            let _ = MediaPlayer2Player::seeked(iface.signal_emitter(), position_us).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trackid_of(id: &str) -> String {
        let state = MprisState {
            track_id: Some(id.to_owned()),
            ..MprisState::default()
        };
        let map = state.metadata_map();
        let val = map.get("mpris:trackid").expect("trackid present");
        let path = ObjectPath::try_from(val.clone()).expect("trackid is an object path");
        path.as_str().to_owned()
    }

    #[test]
    fn trackid_is_valid_outside_mpris_namespace_and_unique() {
        let ids = [
            "/music/Ado/01 唱.flac",
            "/music/Ado/01 踊.flac",
            "/music/Ado/01 __.flac",
            "/music/Ado/01 ..flac",
            "",
            "/music/a b/c#d?.flac",
        ];
        let mut seen = std::collections::HashSet::new();
        for id in ids {
            let path = trackid_of(id);
            assert!(path.starts_with("/org/retrograde/track/"), "{path}");
            assert!(!path.starts_with("/org/mpris"));
            assert!(ObjectPath::try_from(path.as_str()).is_ok());
            assert_eq!(path, track_object_path(id), "stable across calls");
            assert!(seen.insert(path), "duplicate trackid for {id:?}");
        }
        assert!(MprisState::default().metadata_map().is_empty());
    }
}
