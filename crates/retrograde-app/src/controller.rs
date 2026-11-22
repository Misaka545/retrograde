use std::collections::HashSet;
use std::fmt;
use std::future::pending;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use retrograde_audio::{
    AO_INIT_FAILED, AlsaDirect, AlsaHwParams, AlsaMixer, AudioDevice, AudioPath, BRIDGE_NODE_NAME,
    BridgeEvent, CoalescedHwWriter, DirectDevice, EngineEvent, EqualizerConfig, MpvEngine,
    PipeWireBridge, evaluate_bit_perfect,
};
use retrograde_core::library::build_track_index;
use retrograde_core::scanner::ScanOptions;
use retrograde_core::{
    Album, AppSettings, CollectionsData, CollectionsStore, CoverPalette, CoverPaths,
    DirectVolumeBridge, FolderWatcher, LibraryStore, LyricsDoc, LyricsLoader, PlayStats,
    PlaylistCovers, ScanResult, Scanner, SessionState, SessionStore, SmartPlaylistData, TagEdit,
    TagWriter, Track, extract_cover_palette,
};
use retrograde_linux::{
    DeviceReservation, DiscordPlaybackStatus, DiscordPresence, DiscordRpcService,
    DiscordRpcStatus, MprisCommand, MprisService, MprisState, PowerProfile, PowerProfileMonitor,
    RenderTier, ReservationError, SingleInstanceCommand, TrayCommand, TrayService, TrayToggles,
    resolve_client_id,
};
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

use crate::crossfade::{CrossfadeManager, FadeStep};
use crate::library_view::ensure_default_smart_playlists;
use crate::playback::{MpvAdvance, PlayTracker, PlaybackQueue, RepeatMode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceOption {
    pub id: String,
    pub label: String,
    pub is_direct: bool,
}

impl fmt::Display for DeviceOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowAction {
    Raise,
    Toggle,
    /// Tray "Mini player": switch between the main window and the mini player.
    ToggleMiniPlayer,
    Quit,
}

/// How a [`ControllerEvent::Notice`] is shown (C# `ToastKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    Info,
    Success,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub enum ControllerCommand {
    PlayTrackList {
        tracks: Vec<Track>,
        start_index: usize,
    },
    JumpToQueueIndex(usize),
    PlayNext(Track),
    PlayNextTracks(Vec<Track>),
    EnqueueTracks(Vec<Track>),
    RemoveQueueIndex(usize),
    ClearQueue,
    TogglePlayPause,
    Next,
    Previous,
    ToggleShuffle,
    CycleRepeat,
    SeekSeconds(f64),
    SetVolume(f64),
    ToggleMute,
    SelectAudioDevice(String),
    RefreshDevices,
    UpdateSettings(AppSettings),
    SetEqEnabled(bool),
    SelectEqPreset(String),
    SetEqBand {
        band: usize,
        gain: f64,
    },
    SetReplayGain(String),
    SetCrossfadeSeconds(i32),
    ToggleLike(String),
    ToggleLikeTracks(Vec<String>),
    CreatePlaylist(String),
    DeletePlaylist(i64),
    RenamePlaylist {
        id: i64,
        name: String,
    },
    AddTracksToPlaylist {
        playlist_id: i64,
        track_ids: Vec<String>,
    },
    TogglePlaylistTracks {
        playlist_id: i64,
        track_ids: Vec<String>,
    },
    RemoveTrackFromPlaylist {
        playlist_id: i64,
        track_id: String,
    },
    SetPlaylistCover {
        playlist_id: i64,
        image_path: PathBuf,
    },
    RemovePlaylistCover(i64),
    SaveSmartPlaylist(SmartPlaylistData),
    DeleteSmartPlaylist(i64),
    DeleteAlbums(Vec<String>),
    ScanFolders(Vec<PathBuf>),
    ScanFiles(Vec<PathBuf>),
    ImportPaths(Vec<PathBuf>),
    RemoveLibraryFolder(String),
    /// F5 / tray: rescans every library root without dropping anything; tracks whose files are
    /// gone are reported with [`ControllerEvent::MissingTracks`].
    RescanLibrary,
    /// The user confirmed [`ControllerEvent::MissingTracks`]: scan `roots` again and drop the
    /// tracks whose files are gone (and their orphaned covers).
    PruneMissing {
        roots: Vec<PathBuf>,
    },
    SaveTags {
        paths: Vec<PathBuf>,
        album_key: Option<String>,
        edit: TagEdit,
    },
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum ControllerEvent {
    LibraryLoaded(Vec<Album>),
    CollectionsUpdated(CollectionsData),
    StatsUpdated(PlayStats),
    QueueUpdated {
        /// Shared with the controller's queue: a track change sends the same list again for free.
        tracks: Arc<Vec<Track>>,
        index: Option<usize>,
        shuffle: bool,
        repeat: RepeatMode,
        /// How many tracks right after the current one were queued by hand (ADD_TO_QUEUE /
        /// PLAY_NEXT); they play before the rest of the queue.
        hand_len: usize,
    },
    DevicesUpdated(Vec<DeviceOption>),
    TrackChanged(Option<Track>),
    AccentPaletteChanged(Option<CoverPalette>),
    LyricsUpdated {
        track_id: Option<String>,
        lyrics: Option<LyricsDoc>,
    },
    PlaybackState {
        is_playing: bool,
        position_s: f64,
        duration_s: f64,
    },
    VolumeState {
        volume: f64,
        muted: bool,
    },
    DirectStatus {
        audio_device: String,
        /// One line on what reaches the card; empty while a shared output is chosen.
        bit_perfect_status: String,
        /// Playing straight to the card (reserved, mpv on its hw: device): EQ, ReplayGain,
        /// software volume and crossfade are bypassed.
        direct_active: bool,
        /// What the volume knob does in direct output (C# `DirectVolumeHint`); empty otherwise.
        direct_volume_hint: String,
        audio_path: AudioPath,
        pw_bridge_node_id: Option<u32>,
        direct_mixer_control: Option<String>,
    },
    PowerTierChanged {
        profile: PowerProfile,
        tier: RenderTier,
    },
    SettingsSaved(AppSettings),
    /// A message for the user (toast): scan results, playback and engine errors.
    Notice {
        kind: NoticeKind,
        text: String,
    },
    /// Progress of a user-started scan (watcher scans stay quiet). `phase` is the scanner's:
    /// "discovering", "scanning" or "covers".
    ScanProgress {
        phase: &'static str,
        scanned: usize,
        total: usize,
    },
    /// A scan (any kind) has finished or failed.
    ScanFinished,
    /// A scan found `count` library tracks whose files are not on disk (moved folder, offline or
    /// unmounted drive). Nothing was removed; the UI asks, then sends
    /// [`ControllerCommand::PruneMissing`] with `roots`.
    MissingTracks {
        count: usize,
        roots: Vec<PathBuf>,
    },
    TagsSaved {
        first_path: Option<PathBuf>,
        written: usize,
        failed: Vec<String>,
    },
    CoversInvalidated(Vec<PathBuf>),
    AlbumsDeleteFinished,
    TrayAvailable(bool),
    DiscordRpcStatus(DiscordRpcStatus),
    WindowCommand(WindowAction),
    ShutdownComplete,
}

enum PersistMsg {
    Settings(AppSettings),
    Collections(CollectionsData),
    Stats(PlayStats),
    SessionState(SessionState),
    SessionQueue {
        state: SessionState,
        active_ids: Vec<String>,
        original_ids: Vec<String>,
    },
    Flush(oneshot::Sender<()>),
}

fn spawn_persist_worker() -> mpsc::UnboundedSender<PersistMsg> {
    let (tx, mut rx) = mpsc::unbounded_channel::<PersistMsg>();
    tokio::task::spawn_blocking(move || {
        let mut seq: u64 = 0;
        while let Some(first) = rx.blocking_recv() {
            let mut settings: Option<AppSettings> = None;
            let mut collections: Option<CollectionsData> = None;
            let mut stats: Option<PlayStats> = None;
            let mut session_state: Option<(u64, SessionState)> = None;
            let mut session_queue: Option<(u64, SessionState, Vec<String>, Vec<String>)> = None;
            let mut flushes: Vec<oneshot::Sender<()>> = Vec::new();

            let mut push = |msg: PersistMsg| {
                seq += 1;
                match msg {
                    PersistMsg::Settings(s) => settings = Some(s),
                    PersistMsg::Collections(c) => collections = Some(c),
                    PersistMsg::Stats(st) => stats = Some(st),
                    PersistMsg::SessionState(s) => session_state = Some((seq, s)),
                    PersistMsg::SessionQueue {
                        state,
                        active_ids,
                        original_ids,
                    } => session_queue = Some((seq, state, active_ids, original_ids)),
                    PersistMsg::Flush(done) => flushes.push(done),
                }
            };

            push(first);
            while let Ok(more) = rx.try_recv() {
                push(more);
            }

            if let Some(s) = settings {
                s.save();
            }
            if let Some(c) = collections {
                CollectionsStore::save(&c);
            }
            if let Some(st) = stats {
                st.save();
            }
            match (session_queue, session_state) {
                (Some((q_seq, q_state, active_ids, orig_ids)), Some((s_seq, s_state))) => {
                    SessionStore::save_queue(&active_ids);
                    SessionStore::save_original_queue(&orig_ids);
                    if s_seq >= q_seq {
                        SessionStore::save_state(&s_state);
                    } else {
                        SessionStore::save_state(&q_state);
                    }
                }
                (Some((_, q_state, active_ids, orig_ids)), None) => {
                    SessionStore::save_queue(&active_ids);
                    SessionStore::save_original_queue(&orig_ids);
                    SessionStore::save_state(&q_state);
                }
                (None, Some((_, s_state))) => {
                    SessionStore::save_state(&s_state);
                }
                (None, None) => {}
            }
            for tx_done in flushes {
                let _ = tx_done.send(());
            }
        }
    });
    tx
}

pub struct StartupContext {
    pub settings: AppSettings,
    pub single_instance_rx: Option<mpsc::UnboundedReceiver<SingleInstanceCommand>>,
    /// Paths / `file://` URIs given on the primary instance's own command line ("Open with").
    pub open_uris: Vec<String>,
}

/// Collections, play stats, the saved session and the queue from before shuffle, read at start.
type SavedState = (
    CollectionsData,
    PlayStats,
    Option<(SessionState, Vec<String>)>,
    Option<Vec<String>>,
);

struct TagBatchResult {
    first_path: Option<PathBuf>,
    written: usize,
    ok_paths: Vec<PathBuf>,
    failed: Vec<String>,
    changes_cover: bool,
    old_covers: Vec<PathBuf>,
}

struct Coordinator {
    settings: AppSettings,
    mpv: MpvEngine,
    mpv_rx: Option<mpsc::UnboundedReceiver<EngineEvent>>,
    crossfade: CrossfadeManager,
    hw_writer: CoalescedHwWriter,
    reservation: Option<DeviceReservation>,
    pw_bridge: Option<PipeWireBridge>,
    pw_rx: Option<mpsc::UnboundedReceiver<BridgeEvent>>,
    pw_bridge_node_id: Option<u32>,
    direct_card: Option<i32>,
    direct_control: Option<String>,
    /// The output mpv runs on for this session when the saved direct device could not be taken at
    /// start (C# `ChooseOutputAsync` falls back to "auto"); the saved choice stays as it is.
    output_override: Option<String>,
    /// Attempts at reopening the output after mpv failed to open it (`AO_INIT_FAILED`).
    output_retries: u32,
    /// The "no volume control" notice was shown for this direct session.
    volume_lock_told: bool,
    /// The queue slot handed to mpv as its next file (gapless); `None` when nothing was.
    mpv_next: Option<usize>,
    /// What the last `PlaybackState` carried: position ticks are only sent when they change what
    /// the UI shows (another second) or 250 ms have passed.
    last_playback_emit: Option<(Instant, i64, bool, f64)>,
    mpris: Option<MprisService>,
    mpris_rx: Option<mpsc::UnboundedReceiver<MprisCommand>>,
    discord_rpc: Option<DiscordRpcService>,
    discord_status_rx: Option<mpsc::UnboundedReceiver<DiscordRpcStatus>>,
    tray: Option<TrayService>,
    tray_rx: Option<mpsc::UnboundedReceiver<TrayCommand>>,
    watcher: Option<FolderWatcher>,
    watch_rx: Option<mpsc::UnboundedReceiver<Vec<PathBuf>>>,
    pending_watch_paths: Vec<PathBuf>,
    pending_full_rescan: bool,
    /// Roots of a confirmed prune that arrived while another scan was running.
    pending_prune: Option<Vec<PathBuf>>,
    /// What the running scan is, for its progress and result messages.
    scan_job: Option<ScanJob>,
    /// Consecutive tracks that failed to play; the queue stops skipping at [`MAX_FAILED_SKIPS`].
    playback_failures: u32,
    /// Audio files from an "Open with" request, played once a scan has indexed them.
    pending_open_play: Vec<PathBuf>,
    scan_cancel: Option<Arc<AtomicBool>>,
    scan_done_rx: Option<mpsc::UnboundedReceiver<Result<ScanResult, String>>>,
    lyrics_done_rx: Option<mpsc::UnboundedReceiver<(String, Option<LyricsDoc>)>>,
    accent_done_rx: Option<mpsc::UnboundedReceiver<(String, Option<CoverPalette>)>>,
    tag_done_tx: mpsc::UnboundedSender<TagBatchResult>,
    tag_done_rx: Option<mpsc::UnboundedReceiver<TagBatchResult>>,
    persist_tx: mpsc::UnboundedSender<PersistMsg>,
    power_profile: PowerProfile,
    render_tier: RenderTier,
    albums: Vec<Album>,
    collections: CollectionsData,
    stats: PlayStats,
    queue: PlaybackQueue,
    play_tracker: PlayTracker,
    mpv_loaded_track_id: Option<String>,
    pending_restore_seek: Option<f64>,
    crossfade_attempted_track_id: Option<String>,
    is_playing: bool,
    position_s: f64,
    duration_s: f64,
    shared_vol: f64,
    shared_muted: bool,
    current_vol: f64,
    current_muted: bool,
    last_audio_path: AudioPath,
    last_hw_params: Option<AlsaHwParams>,
    last_sample_alteration: Option<String>,
    settings_dirty_since: Option<Instant>,
    session_last_saved: Instant,
    ui_tx: mpsc::UnboundedSender<ControllerEvent>,
}

/// Resolves an "Open with" argument: `file://` URIs are decoded, anything else is
/// treated as a local path. Returns `None` when the target does not exist.
fn resolve_open_uri(raw: &str) -> Option<PathBuf> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let path = if raw.len() >= 7 && raw[..7].eq_ignore_ascii_case("file://") {
        CoverPaths::to_local_path(Some(raw))?
    } else {
        PathBuf::from(raw)
    };
    path.exists()
        .then(|| std::fs::canonicalize(&path).unwrap_or(path))
}

/// Tracks that fail to load one after another are skipped up to this many times; more in a row
/// means it is not about one file (unplugged drive, busy device), so playback stops there.
const MAX_FAILED_SKIPS: u32 = 5;

/// Who started a scan, which decides what the user hears about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScanKind {
    /// F5, tray, ADD MUSIC, drag and drop: progress card, result toast, missing-files prompt.
    Manual,
    /// Folder watcher / tag save: quiet, a toast only when something changed. Never prunes.
    Watch,
}

#[derive(Debug, Clone)]
struct ScanJob {
    kind: ScanKind,
    roots: Vec<PathBuf>,
    prune: bool,
}

/// C# `LibraryRoots()`: the added folders and the folder of every track, a folder inside another
/// listed one being covered by that one.
fn library_roots(folders: &[String], albums: &[Album]) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = folders
        .iter()
        .filter(|f| !f.trim().is_empty())
        .map(PathBuf::from)
        .chain(
            albums
                .iter()
                .flat_map(|a| a.tracks.iter())
                .filter_map(|t| Path::new(&t.file_path).parent())
                .filter(|d| !d.as_os_str().is_empty())
                .map(Path::to_path_buf),
        )
        .collect();
    dirs.sort_by(|a, b| {
        a.as_os_str()
            .len()
            .cmp(&b.as_os_str().len())
            .then_with(|| a.cmp(b))
    });
    dirs.dedup();
    let mut roots: Vec<PathBuf> = Vec::new();
    for d in dirs {
        if !roots.iter().any(|r| d.starts_with(r)) {
            roots.push(d);
        }
    }
    roots
}

/// The C# scan toast parts: "3 NEW, 1 UPDATED, ...".
fn scan_summary(r: &ScanResult) -> String {
    let mut parts = Vec::new();
    if r.added_tracks > 0 {
        parts.push(format!("{} NEW", r.added_tracks));
    }
    if r.updated_tracks > 0 {
        parts.push(format!("{} UPDATED", r.updated_tracks));
    }
    if r.removed_tracks > 0 {
        parts.push(format!("{} REMOVED", r.removed_tracks));
    }
    if r.covers > 0 {
        parts.push(format!("{} COVERS", r.covers));
    }
    if r.merged_albums > 0 {
        parts.push(format!("{} SPLIT ALBUMS JOINED", r.merged_albums));
    }
    parts.join(", ")
}

async fn recv_opt<T>(rx: &mut Option<mpsc::UnboundedReceiver<T>>) -> Option<T> {
    match rx.as_mut() {
        Some(receiver) => receiver.recv().await,
        None => pending().await,
    }
}

/// How long the card may stay open by someone else after it was reserved (PipeWire closing it).
const CARD_FREE_POLL: Duration = Duration::from_millis(50);
const CARD_FREE_POLLS: u32 = 50;
/// Polls in a row the card must read closed before it counts as free.
const CARD_FREE_STABLE: u32 = 3;
/// After the card goes back to PipeWire, its node is set up again a moment later (C# waits 1 s).
const LEAVE_DIRECT_SETTLE: Duration = Duration::from_millis(1000);
/// Reopening a failed output: this many attempts, 150 / 300 / 600 ms apart.
const OUTPUT_RETRIES: u32 = 3;
const OUTPUT_RETRY_BASE: Duration = Duration::from_millis(150);

/// Position ticks closer together than this are merged unless the shown second changes.
const PLAYBACK_TICK_MIN: Duration = Duration::from_millis(250);

/// Whether a position poll should reach the UI (see `Coordinator::emit_playback_tick`).
fn playback_tick_due(
    last: Option<(Instant, i64, bool, f64)>,
    now: Instant,
    position_s: f64,
    is_playing: bool,
    duration_s: f64,
) -> bool {
    let Some((at, second, playing, duration)) = last else {
        return true;
    };
    playing != is_playing
        || (duration - duration_s).abs() > 0.001
        || second != position_s.floor() as i64
        || now.duration_since(at) >= PLAYBACK_TICK_MIN
}

/// What a saved output becomes when it is not offered any more: a direct device on a card that is
/// not a USB DAC, or Retrograde's own volume-bridge sink, go back to the system default.
fn saved_device_replacement(saved: &str) -> Option<String> {
    let bridge = saved
        .split_once('/')
        .is_some_and(|(backend, node)| matches!(backend, "pipewire" | "pulse") && node == BRIDGE_NODE_NAME);
    if bridge {
        return Some("auto".to_owned());
    }
    if AlsaDirect::is_direct(saved)
        && let Some(card) = AlsaDirect::card_of(saved)
        && !AlsaDirect::is_usb_card(card)
    {
        return Some("auto".to_owned());
    }
    None
}

/// The message when the direct card still could not be opened after the retries.
fn direct_failure_message(holders: &[String]) -> String {
    if holders.is_empty() {
        "DIRECT_OUTPUT // THE CARD COULD NOT BE OPENED. PRESS PLAY TO RETRY".to_owned()
    } else {
        format!(
            "DIRECT_OUTPUT // CARD BUSY: IN USE BY {}. PRESS PLAY TO RETRY",
            holders.join(", ").to_uppercase()
        )
    }
}

/// The OUTPUT menu (C# `RefreshAudioDevicesAsync` + `ListDirectDevices`): "auto" as System
/// Default, each shared sink once, then the direct (USB) cards. mpv lists every sink under both
/// `pipewire/` and `pulse/`; one entry is kept per sink, the saved one first, else PipeWire's.
/// The direct-output volume bridge is Retrograde's own sink and is left out.
pub fn build_device_options(
    mpv_devs: &[AudioDevice],
    direct_devs: &[DirectDevice],
    selected: &str,
) -> Vec<DeviceOption> {
    let mut opts = vec![DeviceOption {
        id: "auto".to_owned(),
        label: "System Default".to_owned(),
        is_direct: false,
    }];
    // (node, rank, option): rank 0 = the saved id, 1 = pipewire, 2 = pulse
    let mut shared: Vec<(String, u8, DeviceOption)> = Vec::new();
    for d in mpv_devs {
        let Some((backend, node)) = d.name.split_once('/') else {
            continue;
        };
        if !matches!(backend, "pipewire" | "pulse") || node.is_empty() || node == BRIDGE_NODE_NAME {
            continue;
        }
        let rank = if d.name == selected {
            0
        } else if backend == "pipewire" {
            1
        } else {
            2
        };
        let opt = DeviceOption {
            id: d.name.clone(),
            label: d.description.clone(),
            is_direct: false,
        };
        match shared.iter_mut().find(|(n, _, _)| n == node) {
            Some(entry) if rank < entry.1 => *entry = (node.to_owned(), rank, opt),
            Some(_) => {}
            None => shared.push((node.to_owned(), rank, opt)),
        }
    }
    for (_, _, opt) in shared {
        // Two sinks with the same name (two identical cards): number the later ones.
        let same = opts.iter().filter(|o| o.label == opt.label || o.label.starts_with(&format!("{} #", opt.label))).count();
        let label = if same > 0 { format!("{} #{}", opt.label, same + 1) } else { opt.label };
        opts.push(DeviceOption { label, ..opt });
    }
    for d in direct_devs {
        if !opts.iter().any(|o| o.id == d.name) {
            opts.push(DeviceOption {
                id: d.name.clone(),
                label: d.description.clone(),
                is_direct: true,
            });
        }
    }
    opts
}

pub async fn run_coordinator(
    mut cmd_rx: mpsc::UnboundedReceiver<ControllerCommand>,
    ui_tx: mpsc::UnboundedSender<ControllerEvent>,
    mut startup: StartupContext,
) {
    let cold_open_uris = std::mem::take(&mut startup.open_uris);

    // The library and the saved state load first, alongside the D-Bus services and the audio
    // engine, so the album grid shows as soon as the library is read.
    let library_ui_tx = ui_tx.clone();
    let library_task = tokio::task::spawn_blocking(move || {
        let loaded = LibraryStore::load();
        if let Ok(albums) = &loaded {
            let _ = library_ui_tx.send(ControllerEvent::LibraryLoaded(albums.clone()));
        }
        loaded.map_err(|e| e.to_string())
    });
    let saved_state_task = tokio::task::spawn_blocking(|| {
        (
            // First run (or an unreadable file): seed liked songs and playlists from the old Electron app, as C# does.
            CollectionsStore::load_or_import(),
            PlayStats::load(),
            SessionStore::load(),
            SessionStore::load_original_queue(),
        )
    });

    let (mpv_tx, mpv_rx) = mpsc::unbounded_channel();
    let mpv = MpvEngine::new(mpv_tx);
    let hw_writer = CoalescedHwWriter::new();

    let enable_mpris = startup.settings.enable_mpris;
    let enable_tray = startup.settings.enable_tray;
    let ((power_profile, ppd_rx), mpris_res, tray_res) = tokio::join!(
        PowerProfileMonitor::start(),
        async move {
            if enable_mpris {
                Some(MprisService::start().await)
            } else {
                None
            }
        },
        async move {
            if enable_tray {
                Some(TrayService::start().await)
            } else {
                None
            }
        },
    );
    let mut ppd_rx = Some(ppd_rx);
    let render_tier = RenderTier::resolve(startup.settings.performance_mode, power_profile);
    mpv.set_poll_interval(Duration::from_millis(render_tier.poll_interval_ms));
    let _ = ui_tx.send(ControllerEvent::PowerTierChanged {
        profile: power_profile,
        tier: render_tier,
    });

    let (mpris, mpris_rx) = match mpris_res {
        Some(Ok((svc, rx))) => (Some(svc), Some(rx)),
        Some(Err(err)) => {
            warn!(error = %err, "MPRIS disabled/unavailable");
            (None, None)
        }
        None => (None, None),
    };

    let (tray, tray_rx) = match tray_res {
        Some(Ok((svc, rx))) => (Some(svc), Some(rx)),
        Some(Err(err)) => {
            warn!(error = %err, "System tray unavailable");
            (None, None)
        }
        None => (None, None),
    };

    let (discord_rpc, discord_status_rx) = if startup.settings.enable_discord_rpc {
        let (tx, rx) = mpsc::unbounded_channel();
        let client_id = resolve_client_id(&startup.settings.discord_client_id);
        let svc = DiscordRpcService::start(
            client_id,
            startup.settings.discord_fetch_artwork,
            CoverPaths::app_dir().join("discord-art-cache.json"),
            tx,
        );
        (Some(svc), Some(rx))
    } else {
        (None, None)
    };

    let persist_tx = spawn_persist_worker();
    let (tag_done_tx, tag_done_rx) = mpsc::unbounded_channel();
    let initial_vol = startup.settings.volume.clamp(0.0, 1.0);
    let eq_filters = EqualizerConfig::build_filters(
        startup.settings.eq_enabled,
        false,
        &startup.settings.eq_gains,
    );
    let crossfade = CrossfadeManager::new(
        startup.settings.audio_device.clone(),
        startup.settings.replay_gain.clone(),
        eq_filters,
    );

    if !crate::popups::CROSSFADE_CHOICES.contains(&startup.settings.crossfade_seconds) {
        // Only the C# steps are offered; anything else counts as off (not saved until changed).
        startup.settings.crossfade_seconds = 0;
    }
    let mut coord = Coordinator {
        settings: startup.settings,
        mpv,
        mpv_rx: Some(mpv_rx),
        crossfade,
        hw_writer,
        reservation: None,
        pw_bridge: None,
        pw_rx: None,
        pw_bridge_node_id: None,
        direct_card: None,
        direct_control: None,
        output_override: None,
        output_retries: 0,
        volume_lock_told: false,
        mpv_next: None,
        last_playback_emit: None,
        mpris,
        mpris_rx,
        discord_rpc,
        discord_status_rx,
        tray,
        tray_rx,
        watcher: None,
        watch_rx: None,
        pending_watch_paths: Vec::new(),
        pending_full_rescan: false,
        pending_prune: None,
        scan_job: None,
        playback_failures: 0,
        pending_open_play: Vec::new(),
        scan_cancel: None,
        scan_done_rx: None,
        lyrics_done_rx: None,
        accent_done_rx: None,
        tag_done_tx,
        tag_done_rx: Some(tag_done_rx),
        persist_tx,
        power_profile,
        render_tier,
        albums: Vec::new(),
        collections: CollectionsData::default(),
        stats: PlayStats::default(),
        queue: PlaybackQueue::default(),
        play_tracker: PlayTracker::default(),
        mpv_loaded_track_id: None,
        pending_restore_seek: None,
        crossfade_attempted_track_id: None,
        is_playing: false,
        position_s: 0.0,
        duration_s: 0.0,
        shared_vol: initial_vol,
        shared_muted: false,
        current_vol: initial_vol,
        current_muted: false,
        last_audio_path: AudioPath {
            input: None,
            output: None,
        },
        last_hw_params: None,
        last_sample_alteration: None,
        settings_dirty_since: None,
        session_last_saved: Instant::now(),
        ui_tx,
    };

    let _ = coord
        .ui_tx
        .send(ControllerEvent::TrayAvailable(coord.tray.is_some()));
    if coord.discord_rpc.is_none() {
        let _ = coord
            .ui_tx
            .send(ControllerEvent::DiscordRpcStatus(DiscordRpcStatus::Disabled));
    }
    coord.sync_tray_toggles().await;
    coord.load_initial_state(library_task, saved_state_task).await;
    coord.sync_discord_rpc();
    coord.initialize_audio_output().await;
    coord.refresh_devices().await;
    coord.sync_folder_watcher().await;
    if !cold_open_uris.is_empty() {
        coord.open_uris(cold_open_uris).await;
    }

    let mut hw_poll_tick = tokio::time::interval(Duration::from_secs(1));
    hw_poll_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut fade_interval = tokio::time::interval(Duration::from_millis(40));
    fade_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(ControllerCommand::Shutdown) | None => {
                        coord.shutdown().await;
                        let _ = coord.ui_tx.send(ControllerEvent::ShutdownComplete);
                        break;
                    }
                    Some(c) => coord.handle_command(c).await,
                }
            }
            evt = recv_opt(&mut coord.mpv_rx) => {
                match evt {
                    Some(e) => coord.handle_mpv_event(e).await,
                    None => coord.mpv_rx = None,
                }
            }
            spare_evt = recv_opt(&mut coord.crossfade.spare_rx) => {
                match spare_evt {
                    Some(e) => coord.handle_spare_mpv_event(e).await,
                    None => {
                        coord.crossfade.spare_rx = None;
                        if coord.crossfade.is_active() {
                            coord.crossfade.cancel(&coord.mpv, coord.shared_vol, coord.shared_muted);
                        }
                    }
                }
            }
            _ = fade_interval.tick(), if coord.crossfade.is_active() => {
                coord.step_crossfade().await;
            }
            pw_evt = recv_opt(&mut coord.pw_rx) => {
                match pw_evt {
                    Some(e) => coord.handle_pw_bridge_event(e).await,
                    None => {
                        coord.pw_rx = None;
                        coord.pw_bridge = None;
                        coord.pw_bridge_node_id = None;
                        coord.emit_direct_status();
                    }
                }
            }
            ppd = recv_opt(&mut ppd_rx) => {
                match ppd {
                    Some(profile) => {
                        coord.power_profile = profile;
                        coord.apply_render_tier();
                    }
                    None => ppd_rx = None,
                }
            }
            mpris_cmd = recv_opt(&mut coord.mpris_rx) => {
                match mpris_cmd {
                    Some(mc) => coord.handle_mpris_command(mc).await,
                    None => coord.mpris_rx = None,
                }
            }
            discord_st = recv_opt(&mut coord.discord_status_rx) => {
                match discord_st {
                    Some(st) => {
                        let _ = coord.ui_tx.send(ControllerEvent::DiscordRpcStatus(st));
                    }
                    None => coord.discord_status_rx = None,
                }
            }
            tray_cmd = recv_opt(&mut coord.tray_rx) => {
                match tray_cmd {
                    Some(tc) => coord.handle_tray_command(tc).await,
                    None => {
                        coord.tray_rx = None;
                        coord.tray = None;
                        let _ = coord.ui_tx.send(ControllerEvent::TrayAvailable(false));
                    }
                }
            }
            si_cmd = recv_opt(&mut startup.single_instance_rx) => {
                match si_cmd {
                    Some(SingleInstanceCommand::Raise) => {
                        let _ = coord.ui_tx.send(ControllerEvent::WindowCommand(WindowAction::Raise));
                    }
                    Some(SingleInstanceCommand::OpenUris(uris)) => {
                        coord.open_uris(uris).await;
                        let _ = coord.ui_tx.send(ControllerEvent::WindowCommand(WindowAction::Raise));
                    }
                    None => startup.single_instance_rx = None,
                }
            }
            watch_paths = recv_opt(&mut coord.watch_rx) => {
                match watch_paths {
                    Some(paths) => coord.handle_watched_paths(paths),
                    None => coord.watch_rx = None,
                }
            }
            scan_res = recv_opt(&mut coord.scan_done_rx) => {
                coord.scan_cancel = None;
                coord.scan_done_rx = None;
                let job = coord.scan_job.take();
                let _ = coord.ui_tx.send(ControllerEvent::ScanFinished);
                match scan_res {
                    Some(Ok(summary)) => {
                        if summary.changed() {
                            coord.reload_library().await;
                        }
                        if let Some(job) = job {
                            coord.report_scan(&job, &summary);
                        }
                    }
                    Some(Err(err)) => {
                        coord.notice(NoticeKind::Error, format!("SCAN_FAILED // {err}"));
                    }
                    None => {}
                }
                coord.drain_pending_watch_scan();
                coord.play_pending_open().await;
            }
            lyrics_res = recv_opt(&mut coord.lyrics_done_rx) => {
                match lyrics_res {
                    Some((track_id, doc)) => {
                        coord.lyrics_done_rx = None;
                        if coord.queue.current().is_some_and(|t| t.id == track_id) {
                            let _ = coord.ui_tx.send(ControllerEvent::LyricsUpdated {
                                track_id: Some(track_id),
                                lyrics: doc,
                            });
                        }
                    }
                    None => coord.lyrics_done_rx = None,
                }
            }
            accent_res = recv_opt(&mut coord.accent_done_rx) => {
                match accent_res {
                    Some((track_id, palette)) => {
                        coord.accent_done_rx = None;
                        if coord.queue.current().is_some_and(|t| t.id == track_id) {
                            let _ = coord
                                .ui_tx
                                .send(ControllerEvent::AccentPaletteChanged(palette));
                        }
                    }
                    None => coord.accent_done_rx = None,
                }
            }
            tag_res = recv_opt(&mut coord.tag_done_rx) => {
                match tag_res {
                    Some(res) => {
                        if !res.old_covers.is_empty() {
                            let _ = coord
                                .ui_tx
                                .send(ControllerEvent::CoversInvalidated(res.old_covers.clone()));
                        }
                        if res.written > 0 {
                            coord.reload_library().await;
                            if res.changes_cover {
                                let ok_set: std::collections::HashSet<&PathBuf> =
                                    res.ok_paths.iter().collect();
                                let mut new_covers = res.old_covers;
                                for a in &coord.albums {
                                    if a.tracks.iter().any(|t| ok_set.contains(&PathBuf::from(&t.file_path))) {
                                        if let Some(p) = &a.cover_thumb_path {
                                            new_covers.push(p.clone());
                                        }
                                        if let Some(p) = &a.cover_full_path {
                                            new_covers.push(p.clone());
                                        }
                                    }
                                }
                                if !new_covers.is_empty() {
                                    let _ = coord
                                        .ui_tx
                                        .send(ControllerEvent::CoversInvalidated(new_covers));
                                }
                            }
                            if res.failed.is_empty() {
                                coord.notice(
                                    NoticeKind::Success,
                                    format!("TAGS_SAVED // {} FILE(S)", res.written),
                                );
                            } else {
                                coord.notice(
                                    NoticeKind::Error,
                                    format!(
                                        "TAGS_SAVED // {} OK, {} FAILED",
                                        res.written,
                                        res.failed.len()
                                    ),
                                );
                            }
                        } else if let Some(first) = res.failed.first() {
                            coord.notice(NoticeKind::Error, format!("TAG_WRITE_FAILED // {first}"));
                        }
                        let _ = coord.ui_tx.send(ControllerEvent::TagsSaved {
                            first_path: res.first_path,
                            written: res.written,
                            failed: res.failed,
                        });
                    }
                    None => coord.tag_done_rx = None,
                }
            }
            _ = hw_poll_tick.tick() => {
                coord.poll_external_mixer_and_save().await;
            }
        }
    }
}

impl Coordinator {
    async fn initialize_audio_output(&mut self) {
        let saved = self.settings.audio_device.clone();
        let replacement = tokio::task::spawn_blocking({
            let saved = saved.clone();
            move || saved_device_replacement(&saved)
        })
        .await
        .ok()
        .flatten();
        if let Some(next) = replacement {
            info!(saved, next, "Saved output is not offered any more; using the system default");
            self.settings.audio_device = next;
            self.save_settings_async();
            let _ = self
                .ui_tx
                .send(ControllerEvent::SettingsSaved(self.settings.clone()));
        }
        let target = self.settings.audio_device.clone();
        if AlsaDirect::is_direct(&target) && !self.enter_direct_mode(&target, false).await {
            // C# `ChooseOutputAsync`: the session plays through PipeWire, the choice stays saved.
            self.output_override = Some("auto".to_owned());
        }
        self.start_mpv_engine();
    }

    /// Before a play request: mpv may have exited (crash, killed); start a new one so the request
    /// does not go nowhere. `false` when it cannot be started.
    fn ensure_mpv_running(&mut self) -> bool {
        if self.mpv.is_running() {
            return true;
        }
        info!("libmpv is not running; starting it again");
        self.start_mpv_engine()
    }

    /// The device mpv is told to open: the saved one, or the session's fallback.
    fn mpv_device(&self) -> String {
        self.output_override
            .clone()
            .unwrap_or_else(|| self.settings.audio_device.clone())
    }

    /// Starts libmpv with the current output, ReplayGain, filters and volume.
    fn start_mpv_engine(&mut self) -> bool {
        let target = self.mpv_device();
        let rg = self.settings.replay_gain.clone();
        if let Err(err) = self.mpv.start(Some(&target), Some(&rg)) {
            warn!(error = %err, "Failed to start libmpv");
            self.notice(NoticeKind::Error, format!("AUDIO_ENGINE_FAILED // {err}"));
            return false;
        }
        self.apply_mpv_filters_and_gain();
        if self.direct_card.is_some() {
            // Direct output: mpv never scales the samples; a card without a control can still mute.
            self.mpv.set_volume(1.0);
            self.mpv
                .set_mute(self.direct_control.is_none() && self.current_muted);
        } else {
            self.mpv.set_volume(self.current_vol);
            self.mpv.set_mute(self.current_muted);
        }
        self.emit_volume();
        self.emit_direct_status();
        true
    }

    fn is_direct_output(&self) -> bool {
        self.direct_card.is_some()
            || (self.output_override.is_none() && AlsaDirect::is_direct(&self.settings.audio_device))
    }

    /// EQ and ReplayGain on mpv; both bypassed while the card plays directly (C# `SelectDirectAsync`).
    fn apply_mpv_filters_and_gain(&mut self) {
        let direct = self.is_direct_output();
        let filters = EqualizerConfig::build_filters(
            self.settings.eq_enabled,
            direct,
            &self.settings.eq_gains,
        );
        self.mpv.set_audio_filters(&filters);
        self.mpv
            .set_replay_gain(if direct { "no" } else { &self.settings.replay_gain });
        // The spare (crossfade) engine never runs in direct output; it keeps the shared settings.
        let shared_filters = EqualizerConfig::build_filters(
            self.settings.eq_enabled,
            false,
            &self.settings.eq_gains,
        );
        let spare_device = if direct { "auto".to_owned() } else { self.mpv_device() };
        self.crossfade
            .sync_config(&spare_device, &self.settings.replay_gain, &shared_filters);
    }

    /// Waits for whoever has the card's PCM open (PipeWire closing its node after letting the card
    /// go) to close it for good: mpv opening `hw:` before that fails with "audio output
    /// initialization failed" (EBUSY). PipeWire can close and reopen the PCM once while it tears
    /// the node down (a stream still linked to it wakes it up), so the card has to stay closed for
    /// [`CARD_FREE_STABLE`] polls in a row. `Err` names the programs still holding it at the end.
    async fn wait_for_card_free(device: &str) -> Result<(), Vec<String>> {
        let probe = |dev: String| async move {
            tokio::task::spawn_blocking(move || AlsaDirect::pcm_is_open(&dev))
                .await
                .ok()
                .flatten()
        };
        let holders = |dev: String| async move {
            tokio::task::spawn_blocking(move || AlsaDirect::pcm_holders(&dev))
                .await
                .unwrap_or_default()
        };
        // Callers never have mpv on this card while they wait (re-selecting the active card
        // returns early), so any open substream is someone else's, even when its descriptor is
        // already gone (PipeWire closing: the kernel still reports it open for a moment).
        let mut closed_for = 0;
        for _ in 0..CARD_FREE_POLLS {
            match probe(device.to_owned()).await {
                Some(true) => closed_for = 0,
                // Closed (or no such PCM: mpv will say so itself).
                _ => {
                    closed_for += 1;
                    if closed_for >= CARD_FREE_STABLE {
                        return Ok(());
                    }
                }
            }
            tokio::time::sleep(CARD_FREE_POLL).await;
        }
        let still = holders(device.to_owned()).await;
        if still.is_empty() { Ok(()) } else { Err(still) }
    }

    /// Takes the card behind `device` from PipeWire and moves mpv onto it (C# `SelectDirectAsync`).
    /// `false` (with a message) when the card cannot be had; the output then stays where it was.
    async fn enter_direct_mode(&mut self, device: &str, switch_live_mpv: bool) -> bool {
        let Some(card) = AlsaDirect::card_of(device) else {
            warn!(device, "Could not resolve ALSA card index for direct device");
            self.notice(NoticeKind::Warn, "DIRECT_OUTPUT // DEVICE NOT FOUND");
            self.emit_direct_status();
            return false;
        };
        if self.direct_card == Some(card) {
            // Already playing straight to this card.
            if switch_live_mpv {
                self.mpv.set_audio_device(device);
            }
            self.emit_direct_status();
            return true;
        }

        self.crossfade
            .cancel(&self.mpv, self.shared_vol, self.shared_muted);
        self.crossfade.shutdown_async().await;

        let desc = tokio::task::spawn_blocking({
            let dev = device.to_owned();
            move || {
                AlsaDirect::list_devices()
                    .into_iter()
                    .find(|d| d.name == dev)
                    .map(|d| d.description)
            }
        })
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("ALSA Card {card}"));

        // Unlike C#, playback is not paused around the switch: the PipeWire stream only goes quiet
        // while the card changes hands, and a pause would make a failed open look like a paused
        // track to `handle_output_failure`, which then would not retry.
        self.teardown_direct_resources().await;
        match DeviceReservation::acquire(card, &desc).await {
            Ok(res) => self.reservation = Some(res),
            // Without a session bus nobody can be asked: try the card anyway, it may be free.
            Err(ReservationError::NoBus(err)) => {
                warn!(card, error = %err, "No session bus for ReserveDevice1; trying the card anyway");
            }
            Err(err) => {
                warn!(card, error = %err, "Could not reserve the card");
                let who = match &err {
                    ReservationError::Refused { holder: Some(h) } => h.to_uppercase(),
                    _ => "ANOTHER PROGRAM".to_owned(),
                };
                self.notice(NoticeKind::Warn, format!("DIRECT_OUTPUT // CARD IN USE BY {who}"));
                self.emit_direct_status();
                return false;
            }
        }

        // PipeWire lets go of the card's PCM a moment after it gave the reservation up.
        if let Err(holders) = Self::wait_for_card_free(device).await {
            warn!(card, ?holders, "Card still open by another program after the reservation");
        }

        let (control, hw_lvl, hw_mute) = tokio::task::spawn_blocking(move || {
            let control = AlsaMixer::find_control(card);
            let hw_lvl = control
                .as_deref()
                .and_then(|c| AlsaMixer::read_level(card, c));
            let hw_mute = control
                .as_deref()
                .and_then(|c| AlsaMixer::read_muted(card, c));
            (control, hw_lvl, hw_mute)
        })
        .await
        .unwrap_or((None, None, None));

        self.direct_card = Some(card);
        self.direct_control = control.clone();
        self.output_override = None;
        self.last_hw_params = None;
        self.output_retries = 0;

        if control.is_some() {
            if let Some(lvl) = hw_lvl {
                self.current_vol = lvl;
            }
            self.current_muted = hw_mute.unwrap_or(false);
        } else {
            self.current_vol = self.shared_vol;
            self.current_muted = self.shared_muted;
        }

        // Nothing in mpv may touch the samples: no EQ, no ReplayGain, no software volume.
        self.apply_mpv_filters_and_gain();
        self.mpv.set_volume(1.0);
        self.mpv.set_mute(false);
        if switch_live_mpv {
            self.mpv.set_audio_device(device);
        }
        if control.is_none() {
            // No hardware control: the volume stays at 100 %, mute still works in mpv.
            self.mpv.set_mute(self.current_muted);
        }

        if self.settings.direct_volume_bridge == DirectVolumeBridge::VirtualSink {
            self.start_pw_bridge(&desc).await;
        }

        self.sync_mpv_next();
        self.emit_volume();
        self.emit_direct_status();
        self.sync_mpris().await;
        true
    }

    async fn start_pw_bridge(&mut self, desc: &str) {
        let label = format!("{desc} (Direct)");
        let vol = self.current_vol;
        let muted = self.current_muted;
        let res = tokio::task::spawn_blocking(move || PipeWireBridge::connect(label, vol, muted))
            .await
            .unwrap_or_else(|e| Err(format!("PipeWire connect task failed: {e}")));

        match res {
            Ok((bridge, rx)) => {
                self.pw_bridge = Some(bridge);
                self.pw_rx = Some(rx);
            }
            Err(err) => {
                warn!(error = %err, "PipeWire bridge unavailable; staying hardware-only");
                self.pw_bridge = None;
                self.pw_rx = None;
                self.pw_bridge_node_id = None;
            }
        }
    }

    async fn teardown_direct_resources(&mut self) {
        self.pw_rx = None;
        self.pw_bridge_node_id = None;
        if let Some(mut bridge) = self.pw_bridge.take() {
            let _ = tokio::task::spawn_blocking(move || bridge.shutdown()).await;
        }
        if let Some(res) = self.reservation.take() {
            res.release().await;
        }
        self.direct_card = None;
        self.direct_control = None;
        self.last_hw_params = None;
        self.volume_lock_told = false;
    }

    async fn switch_output_device(&mut self, device: String) {
        if AlsaDirect::is_direct(&device) {
            let usb = tokio::task::spawn_blocking({
                let dev = device.clone();
                move || AlsaDirect::card_of(&dev).is_some_and(AlsaDirect::is_usb_card)
            })
            .await
            .unwrap_or(false);
            if !usb {
                self.notice(NoticeKind::Warn, "DIRECT_OUTPUT // ONLY USB DACS CAN BE OPENED DIRECTLY");
                self.emit_direct_status();
                return;
            }
            if !self.enter_direct_mode(&device, true).await {
                // The selection stays where it was (C#).
                let _ = self
                    .ui_tx
                    .send(ControllerEvent::SettingsSaved(self.settings.clone()));
                self.emit_direct_status();
                return;
            }
            self.settings.audio_device = device;
            self.save_settings_async();
            let _ = self
                .ui_tx
                .send(ControllerEvent::SettingsSaved(self.settings.clone()));
        } else {
            self.settings.audio_device = device.clone();
            self.save_settings_async();
            let _ = self
                .ui_tx
                .send(ControllerEvent::SettingsSaved(self.settings.clone()));
            self.output_override = None;
            self.output_retries = 0;
            let was_direct = self.direct_card.is_some() || self.reservation.is_some();
            self.current_vol = self.shared_vol;
            self.current_muted = self.shared_muted;
            if was_direct {
                // C# `LeaveDirectAsync`: mpv lets go of the hw: device, then the card goes back to
                // PipeWire, which sets its node up again a moment later; connect after that.
                let was_playing = self.is_playing;
                if was_playing {
                    self.mpv.set_paused(true);
                }
                self.mpv.set_audio_device(&device);
                self.teardown_direct_resources().await;
                self.apply_mpv_filters_and_gain();
                self.emit_direct_status();
                tokio::time::sleep(LEAVE_DIRECT_SETTLE).await;
                self.mpv.reload_audio();
                self.mpv.set_volume(self.shared_vol);
                self.mpv.set_mute(self.shared_muted);
                if was_playing {
                    self.mpv.set_paused(false);
                }
            } else {
                self.mpv.set_audio_device(&device);
                self.mpv.set_volume(self.shared_vol);
                self.mpv.set_mute(self.shared_muted);
                self.apply_mpv_filters_and_gain();
            }
            self.sync_mpv_next();
            self.emit_volume();
            self.sync_mpris().await;
        }
        self.last_sample_alteration = self.mpv.get_sample_alteration().await;
        self.emit_direct_status();
    }

    /// mpv could not open the output (`AO_INIT_FAILED`): a device problem, not the file's, so the
    /// queue does not skip on. The same track is loaded again at its position a few times with a
    /// growing pause (the card can be busy for a moment, e.g. PipeWire closing it); then playback
    /// stops there with a message saying what is in the way.
    async fn handle_output_failure(&mut self, path: &str) {
        let Some(track) = self
            .queue
            .current()
            .filter(|t| path.is_empty() || t.file_path == path)
            .filter(|t| self.mpv_loaded_track_id.as_deref() == Some(t.id.as_str()))
            .cloned()
        else {
            self.notice(NoticeKind::Error, format!("PLAYBACK_FAILED // {}", AO_INIT_FAILED.to_uppercase()));
            return;
        };
        let position = self.position_s.max(0.0);
        let was_playing = self.is_playing;
        self.mpv_loaded_track_id = None;
        self.is_playing = false;
        self.sync_discord_rpc();
        let device = self.mpv_device();
        let direct = AlsaDirect::is_direct(&device);

        // A paused track (output switched while paused) is not started by itself: it gets the
        // message, and the next Play loads it again here, through the same retries.
        self.output_retries += 1;
        if was_playing && self.output_retries <= OUTPUT_RETRIES {
            let backoff = OUTPUT_RETRY_BASE * 2u32.pow(self.output_retries - 1);
            info!(attempt = self.output_retries, ?backoff, device, "Output failed to open; retrying");
            tokio::time::sleep(backoff).await;
            let free = if direct { Self::wait_for_card_free(&device).await } else { Ok(()) };
            if free.is_ok() {
                // Something else may have been asked for meanwhile; only retry the same track.
                if self.queue.current().is_some_and(|t| t.id == track.id) && self.mpv_loaded_track_id.is_none() {
                    self.start_current_track(position).await;
                }
                return;
            }
        }

        self.output_retries = 0;
        let message = if direct {
            let holders = tokio::task::spawn_blocking({
                let dev = device.clone();
                move || AlsaDirect::pcm_holders(&dev)
            })
            .await
            .unwrap_or_default();
            direct_failure_message(&holders)
        } else {
            "AUDIO_OUTPUT_FAILED // THE OUTPUT DEVICE COULD NOT BE OPENED. PRESS PLAY TO RETRY".to_owned()
        };
        self.notice(NoticeKind::Error, message);
        self.pending_restore_seek = (position > 0.0).then_some(position);
        self.position_s = position;
        self.emit_playback();
        self.save_session_state_async();
        self.sync_mpris().await;
        self.sync_tray().await;
    }

    async fn handle_command(&mut self, cmd: ControllerCommand) {
        match cmd {
            ControllerCommand::PlayTrackList {
                tracks,
                start_index,
            } => {
                self.crossfade
                    .cancel(&self.mpv, self.shared_vol, self.shared_muted);
                self.queue.set_queue(tracks, start_index, &self.albums);
                self.playback_failures = 0;
                self.emit_queue();
                self.save_queue_async();
                self.start_current_track(0.0).await;
            }
            ControllerCommand::JumpToQueueIndex(idx) => {
                self.playback_failures = 0;
                self.crossfade
                    .cancel(&self.mpv, self.shared_vol, self.shared_muted);
                if self.queue.jump_to(idx).is_some() {
                    self.emit_queue();
                    self.start_current_track(0.0).await;
                }
            }
            ControllerCommand::PlayNext(track) => {
                let title = track.title.clone();
                self.enqueue_hand(vec![track], true).await;
                self.notice(NoticeKind::Success, format!("PLAYS_NEXT // {title}"));
            }
            ControllerCommand::PlayNextTracks(tracks) => {
                let n = tracks.len();
                if n > 0 {
                    self.enqueue_hand(tracks, true).await;
                    self.notice(NoticeKind::Success, format!("PLAYS_NEXT // {n} TRACKS"));
                }
            }
            ControllerCommand::EnqueueTracks(tracks) => {
                let n = tracks.len();
                let first_title = tracks.first().map(|t| t.title.clone());
                if n > 0 {
                    self.enqueue_hand(tracks, false).await;
                    let msg = if n == 1 {
                        format!("TRACK_ADDED // QUEUE: {}", first_title.unwrap_or_default())
                    } else {
                        format!("TRACKS_ADDED // QUEUE ({n})")
                    };
                    self.notice(NoticeKind::Success, msg);
                }
            }
            ControllerCommand::RemoveQueueIndex(idx) => {
                let was_current = self.queue.index() == Some(idx);
                if self.queue.remove_at(idx) {
                    self.crossfade_attempted_track_id = None;
                    if let Some((target_id, hint_idx)) = self
                        .crossfade
                        .active_target()
                        .map(|(id, i)| (id.to_owned(), i))
                        && (was_current || self.queue.find_track_near(hint_idx, &target_id).is_none())
                    {
                        self.crossfade
                            .cancel(&self.mpv, self.shared_vol, self.shared_muted);
                    }
                    self.emit_queue();
                    self.save_queue_async();
                    if was_current {
                        if self.queue.current().is_some() {
                            self.start_current_track(0.0).await;
                        } else {
                            self.stop_to_idle().await;
                        }
                    } else {
                        self.sync_mpv_next();
                        self.sync_mpris().await;
                    }
                }
            }
            ControllerCommand::ClearQueue => {
                self.crossfade
                    .cancel(&self.mpv, self.shared_vol, self.shared_muted);
                self.crossfade_attempted_track_id = None;
                self.queue.clear();
                self.emit_queue();
                self.save_queue_async();
                self.stop_to_idle().await;
            }
            ControllerCommand::TogglePlayPause => {
                self.playback_failures = 0;
                self.output_retries = 0;
                if self.mpv_loaded_track_id.is_some() && !self.ensure_mpv_running() {
                    return;
                }
                if self.mpv_loaded_track_id.is_some() {
                    if self.is_playing && self.crossfade.is_active() {
                        self.crossfade
                            .cancel(&self.mpv, self.shared_vol, self.shared_muted);
                    }
                    self.mpv.toggle_pause();
                } else if self.queue.current().is_some() {
                    let seek = self.pending_restore_seek.take().unwrap_or(0.0);
                    self.start_current_track(seek).await;
                } else if let Some(first) = self.first_library_track() {
                    self.queue.set_queue(vec![first], 0, &self.albums);
                    self.emit_queue();
                    self.save_queue_async();
                    self.start_current_track(0.0).await;
                }
            }
            ControllerCommand::Next => {
                self.playback_failures = 0;
                self.crossfade
                    .cancel(&self.mpv, self.shared_vol, self.shared_muted);
                self.step_queue(false).await;
            }
            ControllerCommand::Previous => {
                self.crossfade
                    .cancel(&self.mpv, self.shared_vol, self.shared_muted);
                self.crossfade_attempted_track_id = None;
                if self.position_s > 3.0 {
                    if self.mpv_loaded_track_id.is_some() {
                        self.mpv.seek(0.0);
                    } else {
                        self.pending_restore_seek = Some(0.0);
                    }
                    self.position_s = 0.0;
                    self.emit_playback();
                    self.sync_discord_rpc();
                    if self.mpv_loaded_track_id.is_none() {
                        self.save_session_state_async();
                    }
                } else if self.queue.step_prev().is_some() {
                    self.emit_queue();
                    self.start_current_track(0.0).await;
                }
            }
            ControllerCommand::ToggleShuffle => {
                self.crossfade
                    .cancel(&self.mpv, self.shared_vol, self.shared_muted);
                self.crossfade_attempted_track_id = None;
                self.queue.toggle_shuffle(&self.albums);
                self.emit_queue();
                self.save_queue_async();
                self.save_session_state_async();
                self.sync_mpv_next();
                self.sync_mpris().await;
            }
            ControllerCommand::CycleRepeat => {
                self.queue.cycle_repeat();
                self.crossfade_attempted_track_id = None;
                if self.queue.repeat() == RepeatMode::One && self.crossfade.is_active() {
                    self.crossfade
                        .cancel(&self.mpv, self.shared_vol, self.shared_muted);
                }
                self.emit_queue();
                self.save_session_state_async();
                self.sync_mpv_next();
                self.sync_mpris().await;
            }
            ControllerCommand::SeekSeconds(sec) => {
                let target = if self.duration_s > 0.0 {
                    sec.clamp(0.0, self.duration_s)
                } else {
                    sec.max(0.0)
                };
                self.crossfade
                    .cancel(&self.mpv, self.shared_vol, self.shared_muted);
                if self.mpv_loaded_track_id.is_some() {
                    self.mpv.seek(target);
                } else if self.queue.current().is_some() {
                    self.pending_restore_seek = Some(target);
                }
                self.position_s = target;
                self.crossfade_attempted_track_id = None;
                self.emit_playback();
                self.sync_discord_rpc();
                if self.mpv_loaded_track_id.is_none() {
                    self.save_session_state_async();
                }
                if let Some(mpris) = &self.mpris {
                    mpris.seeked_to((self.position_s * 1_000_000.0) as i64).await;
                }
            }
            ControllerCommand::SetVolume(vol) => self.set_volume_from_ui_or_mpris(vol).await,
            ControllerCommand::ToggleMute => self.set_mute(!self.current_muted).await,
            ControllerCommand::SelectAudioDevice(dev) => self.switch_output_device(dev).await,
            ControllerCommand::RefreshDevices => self.refresh_devices().await,
            ControllerCommand::UpdateSettings(next) => self.apply_updated_settings(next).await,
            ControllerCommand::SetEqEnabled(enabled) => {
                self.settings.eq_enabled = enabled;
                self.ensure_eq_gains_len();
                self.finish_dsp_update().await;
            }
            ControllerCommand::SelectEqPreset(name) => {
                if let Some((_, bands)) = EqualizerConfig::PRESETS
                    .iter()
                    .find(|(p, _)| *p == name)
                {
                    self.settings.eq_preset = name;
                    self.settings.eq_gains = bands.to_vec();
                    if !self.settings.eq_enabled {
                        // Picking a curve means wanting to hear it (C# `ApplyPreset`).
                        self.settings.eq_enabled = true;
                        self.finish_dsp_update().await;
                        return;
                    }
                    if !self.is_direct_output() {
                        self.mpv.update_eq_all_live(&self.settings.eq_gains);
                        self.crossfade.update_eq_all_live(&self.settings.eq_gains);
                    }
                    self.settings_dirty_since = Some(Instant::now());
                    let _ = self
                        .ui_tx
                        .send(ControllerEvent::SettingsSaved(self.settings.clone()));
                }
            }
            ControllerCommand::SetEqBand { band, gain } => {
                if band < 10 {
                    self.ensure_eq_gains_len();
                    self.settings.eq_gains[band] =
                        gain.clamp(-EqualizerConfig::MAX_GAIN, EqualizerConfig::MAX_GAIN);
                    self.settings.eq_preset = "CUSTOM".to_owned();
                    if self.settings.eq_enabled && !self.is_direct_output() {
                        self.mpv.update_eq_band_live(band, &self.settings.eq_gains);
                        self.crossfade
                            .update_eq_band_live(band, &self.settings.eq_gains);
                    }
                    self.settings_dirty_since = Some(Instant::now());
                }
            }
            ControllerCommand::SetReplayGain(mode) => {
                self.settings.replay_gain = MpvEngine::normalize_replay_gain(&mode).to_owned();
                self.finish_dsp_update().await;
            }
            ControllerCommand::SetCrossfadeSeconds(secs) => {
                self.settings.crossfade_seconds = if crate::popups::CROSSFADE_CHOICES.contains(&secs) { secs } else { 0 };
                self.crossfade_attempted_track_id = None;
                self.sync_mpv_next();
                self.settings_dirty_since = Some(Instant::now());
                let _ = self
                    .ui_tx
                    .send(ControllerEvent::SettingsSaved(self.settings.clone()));
            }
            ControllerCommand::ToggleLike(track_id) => {
                if let Some(pos) = self.collections.liked.iter().position(|id| id == &track_id) {
                    self.collections.liked.remove(pos);
                } else {
                    self.collections.liked.push(track_id);
                }
                self.persist_collections();
            }
            ControllerCommand::ToggleLikeTracks(track_ids) => {
                if track_ids.is_empty() {
                    return;
                }
                let liked: HashSet<&str> =
                    self.collections.liked.iter().map(String::as_str).collect();
                let all_liked = track_ids.iter().all(|id| liked.contains(id.as_str()));
                let count = track_ids.len();
                if all_liked {
                    let drop_set: HashSet<&str> = track_ids.iter().map(String::as_str).collect();
                    self.collections
                        .liked
                        .retain(|id| !drop_set.contains(id.as_str()));
                    self.persist_collections();
                    self.notice(
                        NoticeKind::Info,
                        format!("TRACKS_REMOVED // FAVORITES ({count})"),
                    );
                } else {
                    let mut existing: HashSet<String> =
                        self.collections.liked.iter().cloned().collect();
                    for id in track_ids {
                        if existing.insert(id.clone()) {
                            self.collections.liked.push(id);
                        }
                    }
                    self.persist_collections();
                    self.notice(
                        NoticeKind::Success,
                        format!("TRACKS_ADDED // FAVORITES ({count})"),
                    );
                }
            }
            ControllerCommand::CreatePlaylist(name) => {
                let trimmed = name.trim();
                if !trimmed.is_empty() {
                    let next_id = self
                        .collections
                        .playlists
                        .iter()
                        .map(|p| p.id)
                        .max()
                        .unwrap_or(0)
                        + 1;
                    self.collections.playlists.push(retrograde_core::PlaylistData {
                        id: next_id,
                        name: trimmed.to_owned(),
                        ..Default::default()
                    });
                    self.persist_collections();
                }
            }
            ControllerCommand::DeletePlaylist(id) => {
                self.collections.playlists.retain(|p| p.id != id);
                self.persist_collections();
            }
            ControllerCommand::RenamePlaylist { id, name } => {
                let trimmed = name.trim();
                if !trimmed.is_empty()
                    && let Some(p) = self.collections.playlists.iter_mut().find(|p| p.id == id)
                {
                    p.name = trimmed.to_owned();
                    self.persist_collections();
                }
            }
            ControllerCommand::AddTracksToPlaylist {
                playlist_id,
                track_ids,
            } => {
                if let Some(p) = self
                    .collections
                    .playlists
                    .iter_mut()
                    .find(|p| p.id == playlist_id)
                {
                    let name = p.name.clone();
                    for id in track_ids {
                        if !p.track_ids.contains(&id) {
                            p.track_ids.push(id);
                        }
                    }
                    self.persist_collections();
                    self.notice(NoticeKind::Success, format!("TRACKS_ADDED // PLAYLIST: {name}"));
                }
            }
            ControllerCommand::TogglePlaylistTracks {
                playlist_id,
                track_ids,
            } => {
                if track_ids.is_empty() {
                    return;
                }
                if let Some(p) = self
                    .collections
                    .playlists
                    .iter_mut()
                    .find(|p| p.id == playlist_id)
                {
                    let name = p.name.clone();
                    let set: HashSet<&str> = p.track_ids.iter().map(String::as_str).collect();
                    let all_present = track_ids.iter().all(|id| set.contains(id.as_str()));
                    let msg = if all_present {
                        let drop_set: HashSet<&str> =
                            track_ids.iter().map(String::as_str).collect();
                        p.track_ids.retain(|id| !drop_set.contains(id.as_str()));
                        (
                            NoticeKind::Info,
                            format!("TRACKS_REMOVED // PLAYLIST: {name}"),
                        )
                    } else {
                        let mut existing: HashSet<String> =
                            p.track_ids.iter().cloned().collect();
                        for id in track_ids {
                            if existing.insert(id.clone()) {
                                p.track_ids.push(id);
                            }
                        }
                        (
                            NoticeKind::Success,
                            format!("TRACKS_ADDED // PLAYLIST: {name}"),
                        )
                    };
                    self.persist_collections();
                    self.notice(msg.0, msg.1);
                }
            }
            ControllerCommand::RemoveTrackFromPlaylist {
                playlist_id,
                track_id,
            } => {
                if let Some(p) = self
                    .collections
                    .playlists
                    .iter_mut()
                    .find(|p| p.id == playlist_id)
                {
                    p.track_ids.retain(|id| id != &track_id);
                    self.persist_collections();
                }
            }
            ControllerCommand::SetPlaylistCover {
                playlist_id,
                image_path,
            } => {
                let prev = self
                    .collections
                    .playlists
                    .iter()
                    .find(|p| p.id == playlist_id)
                    .and_then(|p| p.cover_art.clone());
                match tokio::task::spawn_blocking(move || {
                    PlaylistCovers::save(playlist_id, &image_path, prev.as_deref())
                })
                .await
                {
                    Ok(Ok(url)) => {
                        if let Some(p) = self
                            .collections
                            .playlists
                            .iter_mut()
                            .find(|p| p.id == playlist_id)
                        {
                            p.cover_art = Some(url);
                            self.persist_collections();
                        }
                    }
                    Ok(Err(err)) => {
                        self.notice(NoticeKind::Error, format!("COVER_FAILED // {err}"));
                    }
                    Err(err) => {
                        self.notice(NoticeKind::Error, format!("COVER_FAILED // {err}"));
                    }
                }
            }
            ControllerCommand::RemovePlaylistCover(playlist_id) => {
                if let Some(p) = self
                    .collections
                    .playlists
                    .iter_mut()
                    .find(|p| p.id == playlist_id)
                    && let Some(prev) = p.cover_art.take()
                {
                    self.persist_collections();
                    tokio::task::spawn_blocking(move || {
                        PlaylistCovers::delete(Some(&prev));
                    });
                }
            }
            ControllerCommand::SaveSmartPlaylist(sp) => {
                let list = self.collections.smart.get_or_insert_with(Vec::new);
                if let Some(existing) = list.iter_mut().find(|x| x.id == sp.id && sp.id > 0) {
                    *existing = sp;
                } else {
                    let next_id = list.iter().map(|x| x.id).max().unwrap_or(0) + 1;
                    let mut created = sp;
                    if created.id <= 0 {
                        created.id = next_id;
                    }
                    list.push(created);
                }
                self.persist_collections();
            }
            ControllerCommand::DeleteSmartPlaylist(id) => {
                if let Some(list) = &mut self.collections.smart {
                    list.retain(|x| x.id != id);
                    self.persist_collections();
                }
            }
            ControllerCommand::DeleteAlbums(names) => self.delete_albums(names).await,
            ControllerCommand::ScanFolders(folders) => self.scan_folders(folders).await,
            ControllerCommand::ScanFiles(files) => self.scan_files(files),
            ControllerCommand::ImportPaths(paths) => self.import_paths(paths).await,
            ControllerCommand::RemoveLibraryFolder(folder) => {
                self.settings.library_folders.retain(|f| f != &folder);
                self.save_settings_async();
                let _ = self
                    .ui_tx
                    .send(ControllerEvent::SettingsSaved(self.settings.clone()));
                self.sync_folder_watcher().await;
            }
            ControllerCommand::RescanLibrary => self.rescan_library(),
            ControllerCommand::PruneMissing { roots } => {
                if roots.is_empty() {
                    return;
                }
                if self.scan_cancel.is_some() {
                    // Run it once the current scan is done rather than dropping the confirmation.
                    self.pending_prune = Some(roots);
                } else {
                    self.start_scan_paths(roots, ScanKind::Manual, true);
                }
            }
            ControllerCommand::SaveTags {
                paths,
                album_key,
                edit,
            } => {
                if self.is_playing
                    && self.queue.current().is_some_and(|t| {
                        let cur = Path::new(&t.file_path);
                        paths.iter().any(|p| p == cur)
                    })
                {
                    let msg = "Pause or switch tracks before editing the currently playing file."
                        .to_owned();
                    self.notice(NoticeKind::Error, msg.clone());
                    let _ = self.ui_tx.send(ControllerEvent::TagsSaved {
                        first_path: paths.first().cloned(),
                        written: 0,
                        failed: vec![msg],
                    });
                } else {
                    let changes_cover = edit.changes_cover();
                    let mut affected_names: Vec<String> = Vec::new();
                    let mut old_covers: Vec<PathBuf> = Vec::new();
                    if changes_cover {
                        if let Some(k) = &album_key
                            && !k.is_empty()
                        {
                            affected_names.push(k.clone());
                            old_covers.push(CoverPaths::thumb_path(k));
                            old_covers.push(CoverPaths::full_path(k));
                        }
                        if let Some(new_name) = &edit.album {
                            let trimmed = new_name.trim();
                            if !trimmed.is_empty() && !affected_names.iter().any(|x| x == trimmed) {
                                affected_names.push(trimmed.to_owned());
                                old_covers.push(CoverPaths::thumb_path(trimmed));
                                old_covers.push(CoverPaths::full_path(trimmed));
                            }
                        }
                        let path_set: std::collections::HashSet<&PathBuf> = paths.iter().collect();
                        for a in &self.albums {
                            if a.tracks
                                .iter()
                                .any(|t| path_set.contains(&PathBuf::from(&t.file_path)))
                            {
                                if !affected_names.contains(&a.name) {
                                    affected_names.push(a.name.clone());
                                }
                                if let Some(p) = &a.cover_thumb_path
                                    && !old_covers.contains(p)
                                {
                                    old_covers.push(p.clone());
                                }
                                if let Some(p) = &a.cover_full_path
                                    && !old_covers.contains(p)
                                {
                                    old_covers.push(p.clone());
                                }
                            }
                        }
                    }

                    let tx = self.tag_done_tx.clone();
                    tokio::task::spawn_blocking(move || {
                        let first_path = paths.first().cloned();
                        let (written, errs) = TagWriter::write_batch(&paths, &edit);
                        let err_paths: std::collections::HashSet<PathBuf> =
                            errs.iter().map(|(p, _)| p.clone()).collect();
                        let ok_paths: Vec<PathBuf> = paths
                            .into_iter()
                            .filter(|p| !err_paths.contains(p))
                            .collect();
                        let failed: Vec<String> = errs
                            .into_iter()
                            .map(|(p, e)| {
                                let name = p
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("file");
                                format!("{name}: {e}")
                            })
                            .collect();
                        if !ok_paths.is_empty() {
                            if changes_cover && !affected_names.is_empty() {
                                let _ = LibraryStore::clear_covers(&affected_names);
                            }
                            let _ = Scanner::scan(
                                &ok_paths,
                                ScanOptions {
                                    prune_missing: false,
                                    max_threads: None,
                                    force: true,
                                },
                                None,
                                None,
                            );
                        }
                        let _ = tx.send(TagBatchResult {
                            first_path,
                            written,
                            ok_paths,
                            failed,
                            changes_cover,
                            old_covers,
                        });
                    });
                }
            }
            ControllerCommand::Shutdown => {}
        }
    }

    fn ensure_eq_gains_len(&mut self) {
        if self.settings.eq_gains.len() < 10 {
            self.settings.eq_gains.resize(10, 0.0);
        }
    }

    async fn finish_dsp_update(&mut self) {
        self.apply_mpv_filters_and_gain();
        self.last_sample_alteration = self.mpv.get_sample_alteration().await;
        self.emit_direct_status();
        self.settings_dirty_since = Some(Instant::now());
        let _ = self
            .ui_tx
            .send(ControllerEvent::SettingsSaved(self.settings.clone()));
    }

    async fn stop_to_idle(&mut self) {
        self.mpv.stop_playback();
        self.mpv_next = None;
        self.mpv_loaded_track_id = None;
        self.pending_restore_seek = None;
        self.crossfade_attempted_track_id = None;
        self.is_playing = false;
        self.position_s = 0.0;
        self.duration_s = 0.0;
        self.play_tracker.reset(None);
        let _ = self.ui_tx.send(ControllerEvent::TrackChanged(None));
        let _ = self.ui_tx.send(ControllerEvent::LyricsUpdated {
            track_id: None,
            lyrics: None,
        });
        self.load_accent_async(None);
        self.emit_playback();
        self.sync_mpris().await;
        self.sync_tray().await;
    }

    async fn start_current_track(&mut self, start_seconds: f64) {
        let Some(track) = self.queue.current().cloned() else {
            return;
        };
        if !self.ensure_mpv_running() {
            // Nothing can play; keep the track where it was so Play can retry later.
            self.mpv_loaded_track_id = None;
            self.pending_restore_seek = (start_seconds > 0.0).then_some(start_seconds);
            self.is_playing = false;
            self.emit_playback();
            self.sync_mpris().await;
            return;
        }
        self.mpv_loaded_track_id = Some(track.id.clone());
        self.pending_restore_seek = None;
        self.crossfade_attempted_track_id = None;
        self.position_s = start_seconds;
        self.duration_s = track.duration;
        self.play_tracker.reset(Some(track.id.clone()));

        self.mpv.play(&track.file_path, start_seconds);
        // `play` unpauses; mpv only reports a pause *change*, so say so here (C# sets IsPlaying too).
        self.is_playing = true;
        self.sync_mpv_next();

        let _ = self
            .ui_tx
            .send(ControllerEvent::TrackChanged(Some(track.clone())));
        self.emit_playback();
        self.load_lyrics_async(&track);
        self.load_accent_async(Some(&track));
        self.save_session_state_async();
        self.sync_mpris().await;
        self.sync_tray().await;
    }

    /// Tells mpv which file comes next for gapless playback (C# `SyncNext`): the same file on
    /// repeat-one, nothing when the change fades over to the spare engine instead.
    fn sync_mpv_next(&mut self) {
        let direct = self.is_direct_output();
        let next = self.queue.auto_next_index().filter(|&idx| {
            !self
                .queue
                .crossfade_applies(idx, self.settings.crossfade_seconds, direct)
        });
        self.hand_mpv_next(next);
    }

    /// Queues `next` in mpv for gapless playback and remembers which slot that was.
    fn hand_mpv_next(&mut self, next: Option<usize>) {
        let next = next.filter(|&i| i < self.queue.tracks().len());
        self.mpv_next = next;
        let path = next.map(|i| self.queue.tracks()[i].file_path.as_str());
        self.mpv.set_next(path);
    }

    async fn step_queue(&mut self, from_eof: bool) {
        if let Some(_next) = self.queue.step_next(from_eof) {
            self.emit_queue();
            self.start_current_track(0.0).await;
        } else if from_eof {
            self.is_playing = false;
            self.mpv_loaded_track_id = None;
            self.pending_restore_seek = None;
            self.crossfade_attempted_track_id = None;
            self.position_s = 0.0;
            self.emit_playback();
            self.save_session_state_async();
            self.sync_mpris().await;
            self.sync_tray().await;
        }
    }

    fn load_lyrics_async(&mut self, track: &Track) {
        let id = track.id.clone();
        let path = PathBuf::from(&track.file_path);
        let (tx, rx) = mpsc::unbounded_channel();
        self.lyrics_done_rx = Some(rx);
        tokio::task::spawn_blocking(move || {
            let doc = LyricsLoader::load(&path);
            let _ = tx.send((id, doc));
        });
    }

    fn load_accent_async(&mut self, track: Option<&Track>) {
        if !self.settings.accent_from_cover {
            self.accent_done_rx = None;
            let _ = self.ui_tx.send(ControllerEvent::AccentPaletteChanged(None));
            return;
        }
        let Some(track) = track else {
            self.accent_done_rx = None;
            let _ = self.ui_tx.send(ControllerEvent::AccentPaletteChanged(None));
            return;
        };
        let cover_path = track
            .cover_thumb_path
            .as_ref()
            .or(track.cover_full_path.as_ref())
            .cloned();
        let Some(path) = cover_path else {
            self.accent_done_rx = None;
            let _ = self.ui_tx.send(ControllerEvent::AccentPaletteChanged(None));
            return;
        };
        let id = track.id.clone();
        let (tx, rx) = mpsc::unbounded_channel();
        self.accent_done_rx = Some(rx);
        tokio::task::spawn_blocking(move || {
            let pal = extract_cover_palette(&path);
            let _ = tx.send((id, pal));
        });
    }

    async fn step_crossfade(&mut self) {
        let step = self.crossfade.step(&self.mpv, self.shared_vol);
        if matches!(step, FadeStep::Completed) {
            let _ = self.finish_crossfade_swap().await;
        }
    }

    async fn finish_crossfade_swap(&mut self) -> bool {
        let Some((target_id, hint_idx)) = self
            .crossfade
            .active_target()
            .map(|(id, i)| (id.to_owned(), i))
        else {
            return false;
        };

        if self.queue.repeat() == RepeatMode::One
            || self.queue.find_track_near(hint_idx, &target_id).is_none()
        {
            self.crossfade
                .cancel(&self.mpv, self.shared_vol, self.shared_muted);
            self.emit_queue();
            self.sync_mpv_next();
            self.sync_mpris().await;
            return false;
        }

        let Some((track, _, spare_ap, spare_pos)) = self.crossfade.complete_swap(
            &mut self.mpv,
            &mut self.mpv_rx,
            self.shared_vol,
            self.shared_muted,
        ) else {
            return false;
        };

        let _ = self.queue.jump_to_track_near(hint_idx, &track.id);
        self.mpv_loaded_track_id = Some(track.id.clone());
        self.pending_restore_seek = None;
        self.crossfade_attempted_track_id = None;
        self.position_s = spare_pos.unwrap_or(0.0);
        self.duration_s = track.duration;
        self.is_playing = true;
        self.play_tracker.reset(Some(track.id.clone()));

        if let Some(ap) = spare_ap {
            self.last_audio_path = ap;
            self.emit_direct_status();
        }

        self.emit_queue();
        self.sync_mpv_next();
        let _ = self
            .ui_tx
            .send(ControllerEvent::TrackChanged(Some(track.clone())));
        self.emit_playback();
        self.load_lyrics_async(&track);
        self.load_accent_async(Some(&track));
        self.save_session_state_async();
        self.sync_mpris().await;
        self.sync_tray().await;
        true
    }

    async fn set_volume_from_ui_or_mpris(&mut self, vol: f64) {
        let v = vol.clamp(0.0, 1.0);
        if self.direct_card.is_some() && self.direct_control.is_none() {
            self.tell_volume_is_locked();
            self.emit_volume();
            return;
        }
        self.current_vol = v;

        if let (Some(card), Some(ctrl)) = (self.direct_card, self.direct_control.as_deref()) {
            self.hw_writer.push_level(card, ctrl, v);
            if let Some(pw) = &self.pw_bridge {
                pw.set_volume(v);
            }
        } else {
            self.shared_vol = v;
            self.settings.volume = v;
            self.settings_dirty_since = Some(Instant::now());
            if !self.crossfade.is_active() {
                self.mpv.set_volume(v);
            }
        }

        self.emit_volume();
        self.sync_mpris().await;
    }

    /// Direct output on a card without a volume control of its own: the volume stays at 100 %.
    /// Said once instead of letting the keys do nothing silently (C# `TellVolumeIsLocked`).
    fn tell_volume_is_locked(&mut self) {
        if self.volume_lock_told {
            return;
        }
        self.volume_lock_told = true;
        self.notice(
            NoticeKind::Info,
            "DIRECT_OUTPUT // THIS CARD HAS NO VOLUME CONTROL: VOLUME STAYS AT 100%, USE THE DAC'S OWN VOLUME",
        );
    }

    async fn set_mute(&mut self, muted: bool) {
        self.current_muted = muted;
        if let (Some(card), Some(ctrl)) = (self.direct_card, self.direct_control.as_deref()) {
            self.hw_writer
                .push_mute(card, ctrl, muted, self.current_vol);
            if let Some(pw) = &self.pw_bridge {
                pw.set_mute(muted);
            }
        } else {
            self.shared_muted = muted;
            self.mpv.set_mute(muted);
            self.crossfade.set_mute(muted);
        }
        self.emit_volume();
    }

    async fn handle_mpv_event(&mut self, evt: EngineEvent) {
        match evt {
            EngineEvent::TimePos(t) => {
                self.position_s = t;
                self.emit_playback_tick();
                self.set_mpris_position(t).await;

                if let Some(track_id) =
                    self.play_tracker
                        .on_time_pos(t, self.duration_s, self.is_playing)
                {
                    self.stats.record_now(&track_id);
                    let stats_save = self.stats.clone();
                    let _ = self.ui_tx.send(ControllerEvent::StatsUpdated(stats_save.clone()));
                    let _ = self.persist_tx.send(PersistMsg::Stats(stats_save));
                }

                let direct = self.is_direct_output();
                if self.is_playing
                    && !direct
                    && !self.crossfade.is_active()
                    && self.settings.crossfade_seconds > 0
                    && self.duration_s > 0.0
                {
                    let remaining = self.duration_s - t;
                    if remaining > 0.25
                        && remaining <= self.settings.crossfade_seconds as f64
                        && let Some(next_idx) = self.queue.auto_next_index()
                        && let Some(cur) = self.queue.current()
                        && self.crossfade_attempted_track_id.as_deref() != Some(cur.id.as_str())
                        && self
                            .queue
                            .crossfade_applies(next_idx, self.settings.crossfade_seconds, direct)
                        && let Some(next_track) = self.queue.tracks().get(next_idx).cloned()
                    {
                        self.crossfade_attempted_track_id = Some(cur.id.clone());
                        let fade_dur = Duration::from_secs_f64((remaining - 0.1).clamp(0.4, 12.0));
                        let poll = Duration::from_millis(self.render_tier.poll_interval_ms);
                        let muted = self.shared_muted;
                        if let Err(err) = self
                            .crossfade
                            .begin_async(next_track, next_idx, fade_dur, poll, muted)
                            .await
                        {
                            warn!(error = %err, "Crossfade begin failed; falling back to gapless/EOF");
                            self.notice(NoticeKind::Warn, format!("CROSSFADE_FAILED // {err}"));
                            self.sync_mpv_next_ignoring_crossfade();
                        }
                    }
                }
            }
            EngineEvent::Duration(d) => {
                self.duration_s = d;
                // A file really loaded: the run of failed tracks is over (C# resets it here too).
                self.playback_failures = 0;
                self.emit_playback();
                self.sync_discord_rpc();
            }
            EngineEvent::Pause(p) => {
                if p && self.crossfade.is_active() {
                    // With keep-open=yes, main emits Pause(true) right before EndOfFile;
                    // let EndOfFile / step_crossfade promote the spare engine cleanly.
                    return;
                }
                // An idle mpv (nothing loaded yet, or just restarted) reports pause=no: that is not
                // playback.
                let playing = !p && self.mpv_loaded_track_id.is_some();
                if playing == self.is_playing {
                    return;
                }
                self.is_playing = playing;
                self.emit_playback();
                self.save_session_state_async();
                self.sync_mpris().await;
                self.sync_tray().await;
            }
            EngineEvent::EndOfFile => {
                if self.crossfade.is_active() {
                    if !self.finish_crossfade_swap().await {
                        self.step_queue(true).await;
                    }
                } else {
                    self.step_queue(true).await;
                }
            }
            EngineEvent::AdvancedToNext(path) => {
                let expected = self.mpv_next.take();
                let track = match self.queue.on_mpv_advanced(&path, expected) {
                    MpvAdvance::Followed(track) => Some(track),
                    MpvAdvance::Reload(track) => {
                        info!(path, intended = track.file_path, "mpv entered a file that is no longer next; loading the intended one");
                        self.emit_queue();
                        self.start_current_track(0.0).await;
                        None
                    }
                    MpvAdvance::Stop => {
                        // As at the end of the queue: stopped on the last track.
                        self.mpv.stop_playback();
                        self.is_playing = false;
                        self.mpv_loaded_track_id = None;
                        self.pending_restore_seek = None;
                        self.position_s = 0.0;
                        self.emit_playback();
                        self.save_session_state_async();
                        self.sync_mpris().await;
                        None
                    }
                };
                if let Some(track) = track {
                    self.mpv_loaded_track_id = Some(track.id.clone());
                    self.position_s = 0.0;
                    self.duration_s = track.duration;
                    self.play_tracker.reset(Some(track.id.clone()));
                    self.emit_queue();
                    self.sync_mpv_next();
                    let _ = self
                        .ui_tx
                        .send(ControllerEvent::TrackChanged(Some(track.clone())));
                    self.emit_playback();
                    self.load_lyrics_async(&track);
                    self.load_accent_async(Some(&track));
                    self.save_session_state_async();
                    self.sync_mpris().await;
                    self.sync_tray().await;
                }
            }
            EngineEvent::AudioPathChanged(ap) => {
                if ap.output.is_some() {
                    // The output opened: a run of failed opens is over.
                    self.output_retries = 0;
                }
                self.last_audio_path = ap;
                if self.direct_card.is_some() {
                    let dev = self.settings.audio_device.clone();
                    let (hw, altered) = tokio::join!(
                        tokio::task::spawn_blocking(move || AlsaDirect::read_hw_params(&dev)),
                        self.mpv.get_sample_alteration()
                    );
                    self.last_hw_params = hw.ok().flatten();
                    self.last_sample_alteration = altered;
                }
                self.emit_direct_status();
            }
            EngineEvent::PlaybackFailed { path, reason } if reason == AO_INIT_FAILED => {
                warn!(path, reason, "mpv could not open the audio output");
                self.handle_output_failure(&path).await;
            }
            EngineEvent::PlaybackFailed { path, reason } => {
                warn!(path, reason, "mpv playback failed");
                let current = self
                    .queue
                    .current()
                    .filter(|t| path.is_empty() || t.file_path == path)
                    .cloned();
                match current {
                    Some(track) if self.mpv_loaded_track_id.as_deref() == Some(track.id.as_str()) => {
                        self.skip_failed_track(&track, &reason).await;
                    }
                    _ => self.notice(NoticeKind::Error, format!("PLAYBACK_FAILED // {reason}")),
                }
            }
            EngineEvent::Exited => {
                warn!("libmpv exited unexpectedly");
                self.crossfade
                    .cancel(&self.mpv, self.shared_vol, self.shared_muted);
                if self.is_playing {
                    self.notice(
                        NoticeKind::Error,
                        "AUDIO_ENGINE_STOPPED // PRESS PLAY TO RESUME",
                    );
                }
                // The next Play starts a new mpv (see `ensure_mpv_running`) and resumes here.
                if self.mpv_loaded_track_id.take().is_some() && self.position_s > 0.0 {
                    self.pending_restore_seek = Some(self.position_s);
                }
                self.is_playing = false;
                self.emit_playback();
                self.save_session_state_async();
                self.sync_mpris().await;
                self.sync_tray().await;
            }
        }
    }

    fn sync_mpv_next_ignoring_crossfade(&mut self) {
        let next = self.queue.auto_next_index();
        self.hand_mpv_next(next);
    }

    /// Moves on after the current track failed to load (C# `OnPlaybackFailed`): says so, then
    /// plays the next track, at most [`MAX_FAILED_SKIPS`] times in a row.
    async fn skip_failed_track(&mut self, track: &Track, reason: &str) {
        let missing = {
            let path = PathBuf::from(&track.file_path);
            !tokio::task::spawn_blocking(move || path.exists())
                .await
                .unwrap_or(true)
        };
        let title = track.title.to_uppercase();
        self.notice(
            NoticeKind::Error,
            if missing {
                format!("FILE_NOT_FOUND // {title}")
            } else {
                format!("PLAYBACK_FAILED // {title} // {}", reason.to_uppercase())
            },
        );
        self.crossfade
            .cancel(&self.mpv, self.shared_vol, self.shared_muted);
        self.mpv_loaded_track_id = None;
        self.is_playing = false;
        self.playback_failures += 1;
        if self.playback_failures < MAX_FAILED_SKIPS
            && self.queue.tracks().len() > 1
            && self.queue.step_next(false).is_some()
        {
            self.emit_queue();
            self.start_current_track(0.0).await;
            return;
        }
        // Too many in a row, or nothing left: stop on this track, ready to retry with Play.
        self.pending_restore_seek = None;
        self.position_s = 0.0;
        self.emit_playback();
        self.save_session_state_async();
        self.sync_mpris().await;
        self.sync_tray().await;
    }

    async fn handle_spare_mpv_event(&mut self, evt: EngineEvent) {
        match evt {
            EngineEvent::AudioPathChanged(ap) => {
                if self.crossfade.is_active() {
                    self.crossfade.set_spare_audio_path(ap);
                }
            }
            EngineEvent::TimePos(t) => {
                if self.crossfade.is_active() {
                    self.crossfade.set_spare_position(t);
                }
            }
            EngineEvent::PlaybackFailed { path, reason } => {
                warn!(path, reason, "Spare crossfade mpv playback failed");
                if self.crossfade.is_active()
                    && self
                        .crossfade
                        .active_target_path()
                        .is_none_or(|p| p == path || path.is_empty())
                {
                    self.crossfade
                        .cancel(&self.mpv, self.shared_vol, self.shared_muted);
                    self.crossfade_attempted_track_id =
                        self.queue.current().map(|t| t.id.clone());
                    self.sync_mpv_next_ignoring_crossfade();
                    self.notice(NoticeKind::Warn, format!("CROSSFADE_FAILED // {reason}"));
                }
            }
            EngineEvent::Exited => {
                warn!("Spare crossfade mpv exited unexpectedly");
                if self.crossfade.is_active() {
                    self.crossfade
                        .cancel(&self.mpv, self.shared_vol, self.shared_muted);
                    self.crossfade_attempted_track_id =
                        self.queue.current().map(|t| t.id.clone());
                    self.sync_mpv_next_ignoring_crossfade();
                }
                self.crossfade.drop_dead_spare();
            }
            _ => {}
        }
    }

    async fn handle_pw_bridge_event(&mut self, evt: BridgeEvent) {
        match evt {
            BridgeEvent::Ready { node_id, .. } => {
                self.pw_bridge_node_id = Some(node_id);
                self.emit_direct_status();
            }
            BridgeEvent::VolumeChanged(v) => {
                let v = v.clamp(0.0, 1.0);
                if self.direct_card.is_some() && self.direct_control.is_none() {
                    // Direct output on a card without a volume control: nothing may scale the samples.
                    self.tell_volume_is_locked();
                    return;
                }
                self.current_vol = v;
                if let (Some(card), Some(ctrl)) = (self.direct_card, self.direct_control.as_deref())
                {
                    self.hw_writer.push_level(card, ctrl, v);
                } else {
                    self.shared_vol = v;
                    self.settings.volume = v;
                    self.settings_dirty_since = Some(Instant::now());
                    if !self.crossfade.is_active() {
                        self.mpv.set_volume(v);
                    }
                }
                self.emit_volume();
                self.sync_mpris().await;
            }
            BridgeEvent::MuteChanged(muted) => {
                self.current_muted = muted;
                if let (Some(card), Some(ctrl)) = (self.direct_card, self.direct_control.as_deref())
                {
                    self.hw_writer
                        .push_mute(card, ctrl, muted, self.current_vol);
                } else {
                    self.shared_muted = muted;
                    self.mpv.set_mute(muted);
                    self.crossfade.set_mute(muted);
                }
                self.emit_volume();
            }
            BridgeEvent::Disconnected(_) => {
                self.pw_rx = None;
                self.pw_bridge = None;
                self.pw_bridge_node_id = None;
                self.emit_direct_status();
            }
        }
    }

    async fn handle_mpris_command(&mut self, cmd: MprisCommand) {
        match cmd {
            MprisCommand::Play => {
                self.playback_failures = 0;
                if self.mpv_loaded_track_id.is_some() {
                    self.mpv.set_paused(false);
                } else if self.queue.current().is_some() {
                    let seek = self.pending_restore_seek.take().unwrap_or(0.0);
                    self.start_current_track(seek).await;
                } else if let Some(first) = self.first_library_track() {
                    self.queue.set_queue(vec![first], 0, &self.albums);
                    self.emit_queue();
                    self.save_queue_async();
                    self.start_current_track(0.0).await;
                }
            }
            MprisCommand::Pause => {
                if self.crossfade.is_active() {
                    self.crossfade
                        .cancel(&self.mpv, self.shared_vol, self.shared_muted);
                }
                self.mpv.set_paused(true);
            }
            MprisCommand::PlayPause => self.handle_command(ControllerCommand::TogglePlayPause).await,
            MprisCommand::Stop => {
                self.crossfade
                    .cancel(&self.mpv, self.shared_vol, self.shared_muted);
                self.mpv.stop_playback();
                self.mpv_loaded_track_id = None;
                self.pending_restore_seek = None;
                self.crossfade_attempted_track_id = None;
                self.is_playing = false;
                self.position_s = 0.0;
                self.emit_playback();
                self.sync_mpris().await;
                self.sync_tray().await;
            }
            MprisCommand::Next => self.handle_command(ControllerCommand::Next).await,
            MprisCommand::Previous => self.handle_command(ControllerCommand::Previous).await,
            MprisCommand::SeekOffsetUs(off) => {
                let target = (self.position_s + off as f64 / 1_000_000.0).max(0.0);
                self.handle_command(ControllerCommand::SeekSeconds(target))
                    .await;
            }
            MprisCommand::SetPositionUs(pos) => {
                self.handle_command(ControllerCommand::SeekSeconds(
                    (pos as f64 / 1_000_000.0).max(0.0),
                ))
                .await;
            }
            MprisCommand::SetVolume(vol) => self.set_volume_from_ui_or_mpris(vol).await,
            MprisCommand::SetLoopStatus(status) => {
                self.queue
                    .set_repeat(RepeatMode::from_mpris_loop_status(&status));
                self.crossfade_attempted_track_id = None;
                if self.queue.repeat() == RepeatMode::One && self.crossfade.is_active() {
                    self.crossfade
                        .cancel(&self.mpv, self.shared_vol, self.shared_muted);
                }
                self.emit_queue();
                self.save_session_state_async();
                self.sync_mpv_next();
                self.sync_mpris().await;
            }
            MprisCommand::SetShuffle(shuffle) => {
                self.crossfade
                    .cancel(&self.mpv, self.shared_vol, self.shared_muted);
                self.crossfade_attempted_track_id = None;
                self.queue.set_shuffle(shuffle, &self.albums);
                self.emit_queue();
                self.save_queue_async();
                self.save_session_state_async();
                self.sync_mpv_next();
                self.sync_mpris().await;
            }
            MprisCommand::Raise => {
                let _ = self
                    .ui_tx
                    .send(ControllerEvent::WindowCommand(WindowAction::Raise));
            }
            MprisCommand::Quit => {
                let _ = self
                    .ui_tx
                    .send(ControllerEvent::WindowCommand(WindowAction::Quit));
            }
        }
    }

    async fn handle_tray_command(&mut self, cmd: TrayCommand) {
        match cmd {
            TrayCommand::ToggleWindow => {
                let _ = self
                    .ui_tx
                    .send(ControllerEvent::WindowCommand(WindowAction::Toggle));
            }
            TrayCommand::ShowWindow => {
                let _ = self
                    .ui_tx
                    .send(ControllerEvent::WindowCommand(WindowAction::Raise));
            }
            TrayCommand::ToggleMiniPlayer => {
                let _ = self
                    .ui_tx
                    .send(ControllerEvent::WindowCommand(WindowAction::ToggleMiniPlayer));
            }
            TrayCommand::RescanLibrary => self.rescan_library(),
            TrayCommand::ToggleMinimizeToTray => {
                // Unset (never asked) shows as on, like the C# checkbox (`!= false`).
                let on = self.settings.minimize_to_tray != Some(false);
                let mut next = self.settings.clone();
                next.minimize_to_tray = Some(!on);
                self.apply_updated_settings(next).await;
            }
            TrayCommand::ToggleWatchFolders => {
                let mut next = self.settings.clone();
                next.watch_folders = !next.watch_folders;
                self.apply_updated_settings(next).await;
            }
            TrayCommand::ToggleAccentFromCover => {
                let mut next = self.settings.clone();
                next.accent_from_cover = !next.accent_from_cover;
                self.apply_updated_settings(next).await;
            }
            TrayCommand::Quit => {
                let _ = self
                    .ui_tx
                    .send(ControllerEvent::WindowCommand(WindowAction::Quit));
            }
        }
    }

    async fn poll_external_mixer_and_save(&mut self) {
        if let Some(card) = self.direct_card {
            let poll_mixer = !self.hw_writer.recently_wrote(Duration::from_millis(500))
                && self.direct_control.is_some();
            let ctrl = self.direct_control.clone();
            let dev = self.settings.audio_device.clone();

            let (hw_lvl, hw_mute, hw_params) = tokio::task::spawn_blocking(move || {
                let hw_params = AlsaDirect::read_hw_params(&dev);
                if poll_mixer && let Some(ctrl_name) = ctrl.as_deref() {
                    let lvl = AlsaMixer::read_level(card, ctrl_name);
                    let mute = AlsaMixer::read_muted(card, ctrl_name);
                    (lvl, mute, hw_params)
                } else {
                    (None, None, hw_params)
                }
            })
            .await
            .unwrap_or((None, None, None));

            let params_changed = self.last_hw_params != hw_params;
            self.last_hw_params = hw_params;

            let mut vol_changed = false;
            if let Some(lvl) = hw_lvl
                && (lvl - self.current_vol).abs() >= 0.008
            {
                self.current_vol = lvl;
                if let Some(pw) = &self.pw_bridge {
                    pw.set_volume(lvl);
                }
                vol_changed = true;
            }
            if let Some(muted) = hw_mute
                && muted != self.current_muted
            {
                self.current_muted = muted;
                if let Some(pw) = &self.pw_bridge {
                    pw.set_mute(muted);
                }
                vol_changed = true;
            }
            if vol_changed {
                self.emit_volume();
                self.sync_mpris().await;
            }
            if params_changed {
                self.emit_direct_status();
            }
        }

        if let Some(since) = self.settings_dirty_since
            && since.elapsed() >= Duration::from_millis(800)
        {
            self.settings_dirty_since = None;
            self.save_settings_async();
        }

        if self.is_playing && self.session_last_saved.elapsed() >= Duration::from_secs(2) {
            self.save_session_state_async();
        }
    }

    async fn apply_updated_settings(&mut self, mut next: AppSettings) {
        let prev = self.settings.clone();
        next.volume = prev.volume;
        next.audio_device = prev.audio_device.clone();
        next.library_folders = prev.library_folders.clone();
        next.eq_enabled = prev.eq_enabled;
        next.eq_preset = prev.eq_preset.clone();
        next.eq_gains = prev.eq_gains.clone();
        next.replay_gain = prev.replay_gain.clone();
        next.crossfade_seconds = prev.crossfade_seconds;
        self.settings = next.clone();

        if prev.performance_mode != next.performance_mode {
            self.apply_render_tier();
        }
        if prev.watch_folders != next.watch_folders {
            self.sync_folder_watcher().await;
        }
        if prev.accent_from_cover != next.accent_from_cover {
            let cur = self.queue.current().cloned();
            self.load_accent_async(cur.as_ref());
        }

        if prev.direct_volume_bridge != next.direct_volume_bridge && self.direct_card.is_some() {
            match next.direct_volume_bridge {
                DirectVolumeBridge::VirtualSink => {
                    let dev = self.settings.audio_device.clone();
                    let desc = tokio::task::spawn_blocking(move || {
                        AlsaDirect::list_devices()
                            .into_iter()
                            .find(|d| d.name == dev)
                            .map(|d| d.description)
                    })
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "ALSA DAC".to_owned());
                    self.start_pw_bridge(&desc).await;
                }
                DirectVolumeBridge::HardwareOnly => {
                    self.pw_rx = None;
                    self.pw_bridge_node_id = None;
                    if let Some(mut b) = self.pw_bridge.take() {
                        let _ = tokio::task::spawn_blocking(move || b.shutdown()).await;
                    }
                }
            }
            self.emit_direct_status();
        }

        if prev.enable_mpris != next.enable_mpris {
            if next.enable_mpris && self.mpris.is_none() {
                if let Ok((svc, rx)) = MprisService::start().await {
                    self.mpris = Some(svc);
                    self.mpris_rx = Some(rx);
                    self.sync_mpris().await;
                }
            } else if !next.enable_mpris {
                self.mpris_rx = None;
                self.mpris = None;
            }
        }

        if prev.enable_discord_rpc != next.enable_discord_rpc {
            if next.enable_discord_rpc {
                if self.discord_rpc.is_none() {
                    let _ = self
                        .ui_tx
                        .send(ControllerEvent::DiscordRpcStatus(DiscordRpcStatus::WaitingForDiscord));
                    let (tx, rx) = mpsc::unbounded_channel();
                    let client_id = resolve_client_id(&next.discord_client_id);
                    self.discord_rpc = Some(DiscordRpcService::start(
                        client_id,
                        next.discord_fetch_artwork,
                        CoverPaths::app_dir().join("discord-art-cache.json"),
                        tx,
                    ));
                    self.discord_status_rx = Some(rx);
                    self.sync_discord_rpc();
                }
            } else {
                self.discord_status_rx = None;
                if let Some(rpc) = self.discord_rpc.take() {
                    tokio::spawn(rpc.shutdown());
                }
                let _ = self
                    .ui_tx
                    .send(ControllerEvent::DiscordRpcStatus(DiscordRpcStatus::Disabled));
            }
        } else if let Some(rpc) = &self.discord_rpc {
            if prev.discord_client_id != next.discord_client_id {
                rpc.set_client_id(resolve_client_id(&next.discord_client_id));
            }
            if prev.discord_fetch_artwork != next.discord_fetch_artwork {
                rpc.set_artwork_enabled(next.discord_fetch_artwork);
            }
        }

        if prev.enable_tray != next.enable_tray {
            if next.enable_tray && self.tray.is_none() {
                if let Ok((svc, rx)) = TrayService::start().await {
                    self.tray = Some(svc);
                    self.tray_rx = Some(rx);
                }
            } else if !next.enable_tray {
                self.tray_rx = None;
                if let Some(t) = self.tray.take() {
                    t.shutdown().await;
                }
            }
            let _ = self
                .ui_tx
                .send(ControllerEvent::TrayAvailable(self.tray.is_some()));
        }

        self.settings_dirty_since = None;
        self.save_settings_async();
        self.sync_tray_toggles().await;
        let _ = self.ui_tx.send(ControllerEvent::SettingsSaved(next));
    }

    fn apply_render_tier(&mut self) {
        self.render_tier =
            RenderTier::resolve(self.settings.performance_mode, self.power_profile);
        let interval = Duration::from_millis(self.render_tier.poll_interval_ms);
        self.mpv.set_poll_interval(interval);
        self.crossfade.set_poll_interval(interval);
        let _ = self.ui_tx.send(ControllerEvent::PowerTierChanged {
            profile: self.power_profile,
            tier: self.render_tier,
        });
    }

    async fn sync_folder_watcher(&mut self) {
        if !self.settings.watch_folders || self.settings.library_folders.is_empty() {
            self.watcher = None;
            self.watch_rx = None;
            return;
        }
        let roots: Vec<PathBuf> = self
            .settings
            .library_folders
            .iter()
            .map(PathBuf::from)
            .collect();
        if let Some(mut watcher) = self.watcher.take() {
            match tokio::task::spawn_blocking(move || {
                watcher.update_roots(&roots);
                watcher
            })
            .await
            {
                Ok(w) => self.watcher = Some(w),
                Err(err) => warn!(error = %err, "Failed updating folder watcher roots"),
            }
        } else {
            match tokio::task::spawn_blocking(move || FolderWatcher::start(&roots)).await {
                Ok(Ok((watcher, rx))) => {
                    self.watcher = Some(watcher);
                    self.watch_rx = Some(rx);
                }
                Ok(Err(err)) => warn!(error = %err, "Failed starting folder watcher"),
                Err(err) => warn!(error = %err, "Folder watcher init task failed"),
            }
        }
    }

    fn handle_watched_paths(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        if self.scan_cancel.is_some() {
            for p in paths {
                if !self.pending_watch_paths.contains(&p) {
                    self.pending_watch_paths.push(p);
                }
            }
        } else {
            // Automatic: picks up new and changed files only, never drops anything.
            self.start_scan_paths(paths, ScanKind::Watch, false);
        }
    }

    fn drain_pending_watch_scan(&mut self) {
        if self.scan_cancel.is_some() {
            return;
        }
        if let Some(roots) = self.pending_prune.take() {
            self.start_scan_paths(roots, ScanKind::Manual, true);
            return;
        }
        if self.pending_full_rescan {
            self.pending_full_rescan = false;
            let roots = library_roots(&self.settings.library_folders, &self.albums);
            // The rescan covers everything under the library roots; loose files elsewhere still need their own scan.
            self.pending_watch_paths
                .retain(|p| !roots.iter().any(|r| p.starts_with(r)));
            if !roots.is_empty() {
                self.start_scan_paths(roots, ScanKind::Manual, false);
                return;
            }
        }
        if !self.pending_watch_paths.is_empty() {
            let batch = std::mem::take(&mut self.pending_watch_paths);
            self.start_scan_paths(batch, ScanKind::Watch, false);
        }
    }

    async fn refresh_devices(&mut self) {
        let mpv_devs: Vec<AudioDevice> = self.mpv.get_audio_devices().await;
        let direct_devs = tokio::task::spawn_blocking(AlsaDirect::list_devices)
            .await
            .unwrap_or_default();
        let opts = build_device_options(&mpv_devs, &direct_devs, &self.settings.audio_device);
        let _ = self.ui_tx.send(ControllerEvent::DevicesUpdated(opts));
    }

    async fn load_initial_state(
        &mut self,
        library_task: tokio::task::JoinHandle<Result<Vec<Album>, String>>,
        saved_state_task: tokio::task::JoinHandle<SavedState>,
    ) {
        match library_task.await {
            // The UI has these already (sent by the loading task).
            Ok(Ok(albums)) => {
                info!(albums = albums.len(), "Loaded library");
                self.albums = albums;
            }
            Ok(Err(err)) => {
                warn!(error = %err, "Failed loading library");
                self.notice(NoticeKind::Error, format!("LIBRARY_ERROR // {err}"));
                let _ = self.ui_tx.send(ControllerEvent::LibraryLoaded(Vec::new()));
            }
            Err(err) => {
                warn!(error = %err, "Library load task panicked");
                let _ = self.ui_tx.send(ControllerEvent::LibraryLoaded(Vec::new()));
            }
        }

        let (mut collections, stats, session, orig_queue) =
            saved_state_task.await.unwrap_or_default();

        if ensure_default_smart_playlists(&mut collections) {
            let _ = self
                .persist_tx
                .send(PersistMsg::Collections(collections.clone()));
        }
        self.collections = collections.clone();
        self.stats = stats.clone();
        let _ = self
            .ui_tx
            .send(ControllerEvent::CollectionsUpdated(collections));
        let _ = self.ui_tx.send(ControllerEvent::StatsUpdated(stats));

        if let Some((state, queue_ids)) = session {
            let index = build_track_index(&self.albums);
            if let Some(restored) =
                self.queue
                    .restore_session(&index, &state, &queue_ids, orig_queue.as_deref())
            {
                self.duration_s = restored.duration;
                self.position_s = state.position.clamp(0.0, restored.duration.max(0.0));
                self.pending_restore_seek = (self.position_s > 0.0).then_some(self.position_s);
                self.play_tracker.reset(Some(restored.id.clone()));
                self.load_lyrics_async(&restored);
                self.load_accent_async(Some(&restored));
                let _ = self
                    .ui_tx
                    .send(ControllerEvent::TrackChanged(Some(restored)));
                self.emit_playback();
            }
            self.emit_queue();
            self.sync_mpris().await;
        }
    }

    async fn reload_library(&mut self) {
        let albums = match tokio::task::spawn_blocking(LibraryStore::load).await {
            Ok(Ok(albums)) => albums,
            Ok(Err(err)) => {
                warn!(error = %err, "Failed loading library");
                self.notice(NoticeKind::Error, format!("LIBRARY_ERROR // {err}"));
                Vec::new()
            }
            Err(err) => {
                warn!(error = %err, "Library load task panicked");
                Vec::new()
            }
        };
        info!(albums = albums.len(), "Loaded library");
        self.albums = albums.clone();
        if !self.queue.is_empty() {
            let index = build_track_index(&self.albums);
            self.queue.refresh_from_library(&index);
            if let Some(cur) = self.queue.current().cloned() {
                // A rescan may have produced new cover art for the current track.
                self.load_accent_async(Some(&cur));
                let _ = self.ui_tx.send(ControllerEvent::TrackChanged(Some(cur)));
                self.sync_discord_rpc();
            }
            self.emit_queue();
        }
        let _ = self.ui_tx.send(ControllerEvent::LibraryLoaded(albums));
    }

    /// Handles "Open with Retrograde": imports every resolvable path and plays the
    /// opened audio file(s) — immediately when already indexed, else after the scan.
    async fn open_uris(&mut self, uris: Vec<String>) {
        let paths: Vec<PathBuf> = uris.iter().filter_map(|u| resolve_open_uri(u)).collect();
        if paths.is_empty() {
            return;
        }
        let audio: Vec<PathBuf> = paths
            .iter()
            .filter(|p| p.is_file() && Scanner::is_audio_file(p))
            .cloned()
            .collect();
        if !audio.is_empty() {
            let found = self.tracks_for_paths(&audio);
            if found.len() == audio.len() {
                self.pending_open_play.clear();
                self.handle_command(ControllerCommand::PlayTrackList {
                    tracks: found,
                    start_index: 0,
                })
                .await;
            } else {
                self.pending_open_play = audio;
            }
        }
        self.import_paths(paths).await;
    }

    /// Plays pending "Open with" files once a scan has indexed them. Keeps waiting while
    /// another scan batch is still running; gives up once scanning is idle.
    async fn play_pending_open(&mut self) {
        if self.pending_open_play.is_empty() {
            return;
        }
        let found = self.tracks_for_paths(&self.pending_open_play.clone());
        if found.is_empty() {
            if self.scan_cancel.is_none() {
                self.pending_open_play.clear();
            }
            return;
        }
        self.pending_open_play.clear();
        self.handle_command(ControllerCommand::PlayTrackList {
            tracks: found,
            start_index: 0,
        })
        .await;
    }

    fn tracks_for_paths(&self, paths: &[PathBuf]) -> Vec<Track> {
        paths
            .iter()
            .filter_map(|p| {
                let canon = std::fs::canonicalize(p).ok();
                let file_name = p.file_name();
                self.albums
                    .iter()
                    .flat_map(|a| a.tracks.iter())
                    .find(|t| {
                        let tp = Path::new(&t.file_path);
                        if tp == p.as_path() {
                            return true;
                        }
                        // Library roots may be stored via symlinked paths: only canonicalize
                        // candidates whose file name already matches.
                        tp.file_name() == file_name
                            && canon.is_some()
                            && std::fs::canonicalize(tp).ok() == canon
                    })
                    .cloned()
            })
            .collect()
    }

    async fn enqueue_hand(&mut self, tracks: Vec<Track>, at_front: bool) {
        if tracks.is_empty() {
            return;
        }
        // Mid-fade the current index is still the outgoing track: the hand line would land
        // between it and the incoming one and be skipped by the swap. Stop the fade first.
        if self.crossfade.is_active() {
            self.crossfade
                .cancel(&self.mpv, self.shared_vol, self.shared_muted);
        }
        self.crossfade_attempted_track_id = None;
        let started_new = if at_front {
            self.queue.play_next(tracks, &self.albums)
        } else {
            self.queue.enqueue(tracks, &self.albums)
        };
        if started_new {
            self.emit_queue();
            self.save_queue_async();
            self.start_current_track(0.0).await;
        } else {
            self.emit_queue();
            self.save_queue_async();
            self.sync_mpv_next();
            self.sync_mpris().await;
        }
    }

    fn scan_files(&mut self, paths: Vec<PathBuf>) {
        let to_scan: Vec<PathBuf> = paths
            .into_iter()
            .filter(|p| p.is_file() && Scanner::is_audio_file(p))
            .collect();
        if to_scan.is_empty() {
            return;
        }
        if self.scan_cancel.is_some() {
            for p in to_scan {
                if !self.pending_watch_paths.contains(&p) {
                    self.pending_watch_paths.push(p);
                }
            }
            self.notice(NoticeKind::Info, "SCAN_IN_PROGRESS // FILES QUEUED");
        } else {
            self.start_scan_paths(to_scan, ScanKind::Manual, false);
        }
    }

    async fn delete_albums(&mut self, names: Vec<String>) {
        if names.is_empty() {
            let _ = self.ui_tx.send(ControllerEvent::AlbumsDeleteFinished);
            return;
        }
        if self.scan_cancel.is_some() {
            self.notice(
                NoticeKind::Warn,
                "SCAN_IN_PROGRESS // DELETE AGAIN WHEN IT FINISHES",
            );
            let _ = self.ui_tx.send(ControllerEvent::AlbumsDeleteFinished);
            return;
        }
        let name_set: HashSet<&str> = names.iter().map(String::as_str).collect();
        let del_ids: HashSet<String> = self
            .albums
            .iter()
            .filter(|a| name_set.contains(a.name.as_str()))
            .flat_map(|a| a.tracks.iter().map(|t| t.id.clone()))
            .collect();

        let names_owned = names.clone();
        let res = tokio::task::spawn_blocking(move || LibraryStore::delete_albums(&names_owned))
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r.map_err(|e| e.to_string()));
        if let Err(err) = res {
            self.notice(NoticeKind::Error, format!("PURGE_FAILED // {err}"));
            let _ = self.ui_tx.send(ControllerEvent::AlbumsDeleteFinished);
            return;
        }

        let (q_changed, cur_removed) = self.queue.remove_tracks_by_id(&del_ids);
        if cur_removed {
            self.crossfade
                .cancel(&self.mpv, self.shared_vol, self.shared_muted);
            self.emit_queue();
            self.save_queue_async();
            self.stop_to_idle().await;
        } else if q_changed {
            if let Some((tid, hint)) = self
                .crossfade
                .active_target()
                .map(|(id, i)| (id.to_owned(), i))
                && self.queue.find_track_near(hint, &tid).is_none()
            {
                self.crossfade
                    .cancel(&self.mpv, self.shared_vol, self.shared_muted);
            }
            self.emit_queue();
            self.save_queue_async();
            self.sync_mpv_next();
            self.sync_mpris().await;
        }

        self.reload_library().await;
        let _ = self.ui_tx.send(ControllerEvent::AlbumsDeleteFinished);
        let msg = if names.len() == 1 {
            format!("DATA_PURGED // {}", names[0])
        } else {
            format!("DATA_PURGED // {} ALBUMS", names.len())
        };
        self.notice(NoticeKind::Info, msg);
    }

    async fn scan_folders(&mut self, folders: Vec<PathBuf>) {
        if folders.is_empty() {
            return;
        }
        if self.scan_cancel.is_some() {
            self.notice(NoticeKind::Info, "SCAN_IN_PROGRESS // TRY AGAIN WHEN IT IS DONE");
            return;
        }
        for f in &folders {
            let s = f.to_string_lossy().into_owned();
            if !self.settings.library_folders.contains(&s) {
                self.settings.library_folders.push(s);
            }
        }
        self.save_settings_async();
        let _ = self
            .ui_tx
            .send(ControllerEvent::SettingsSaved(self.settings.clone()));
        self.sync_folder_watcher().await;

        let all_folders: Vec<PathBuf> = self
            .settings
            .library_folders
            .iter()
            .map(PathBuf::from)
            .collect();
        self.start_scan_paths(all_folders, ScanKind::Manual, false);
    }

    async fn import_paths(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let mut dirs_added = false;
        let mut to_scan = Vec::new();
        for p in paths {
            if p.is_dir() {
                let p = std::fs::canonicalize(&p).unwrap_or(p);
                let already_covered = self.settings.library_folders.iter().any(|f| {
                    let root = Path::new(f);
                    let root_canon = std::fs::canonicalize(root).ok();
                    p.starts_with(root) || root_canon.is_some_and(|rc| p.starts_with(rc))
                });
                if !already_covered {
                    self.settings
                        .library_folders
                        .push(p.to_string_lossy().into_owned());
                    dirs_added = true;
                }
                to_scan.push(p);
            } else if p.is_file() && Scanner::is_audio_file(&p) {
                to_scan.push(p);
            }
        }
        if dirs_added {
            self.save_settings_async();
            let _ = self
                .ui_tx
                .send(ControllerEvent::SettingsSaved(self.settings.clone()));
            self.sync_folder_watcher().await;
        }
        if !to_scan.is_empty() {
            if self.scan_cancel.is_some() {
                for p in to_scan {
                    if !self.pending_watch_paths.contains(&p) {
                        self.pending_watch_paths.push(p);
                    }
                }
            } else {
                self.start_scan_paths(to_scan, ScanKind::Manual, false);
            }
        }
    }

    /// Starts a scan unless one is running. Only a confirmed [`ControllerCommand::PruneMissing`]
    /// passes `prune_missing = true`; every other scan keeps tracks whose files are gone.
    fn start_scan_paths(&mut self, paths: Vec<PathBuf>, kind: ScanKind, prune_missing: bool) {
        if paths.is_empty() || self.scan_cancel.is_some() {
            return;
        }
        let threads = self.render_tier.scan_threads;
        let ui_tx = self.ui_tx.clone();
        let report_progress = kind == ScanKind::Manual;
        if report_progress {
            let _ = ui_tx.send(ControllerEvent::Notice {
                kind: NoticeKind::Info,
                text: if prune_missing {
                    "SCAN_STARTED // REMOVING MISSING FILES".to_owned()
                } else {
                    "SCAN_STARTED".to_owned()
                },
            });
            let _ = ui_tx.send(ControllerEvent::ScanProgress {
                phase: "discovering",
                scanned: 0,
                total: 0,
            });
        }
        self.scan_job = Some(ScanJob {
            kind,
            roots: paths.clone(),
            prune: prune_missing,
        });

        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_worker = Arc::clone(&cancel);
        let (done_tx, done_rx) = mpsc::unbounded_channel();
        self.scan_cancel = Some(cancel);
        self.scan_done_rx = Some(done_rx);

        tokio::task::spawn_blocking(move || {
            let mut last = Instant::now();
            let mut cb = |p: retrograde_core::ScanProgress| {
                if report_progress && last.elapsed() >= Duration::from_millis(120) {
                    last = Instant::now();
                    let _ = ui_tx.send(ControllerEvent::ScanProgress {
                        phase: p.phase,
                        scanned: p.scanned,
                        total: p.total,
                    });
                }
            };
            let result = Scanner::scan(
                &paths,
                ScanOptions {
                    prune_missing,
                    max_threads: Some(threads),
                    force: false,
                },
                Some(&cancel_worker),
                Some(&mut cb),
            )
            .map_err(|e| e.to_string());
            let _ = done_tx.send(result);
        });
    }

    fn notice(&self, kind: NoticeKind, text: impl Into<String>) {
        let _ = self.ui_tx.send(ControllerEvent::Notice {
            kind,
            text: text.into(),
        });
    }

    /// The result toast of a finished scan (C# `RunScanAsync` / the watcher's pick-up), and the
    /// missing-files prompt: nothing is dropped until the user confirms.
    fn report_scan(&self, job: &ScanJob, r: &ScanResult) {
        match job.kind {
            ScanKind::Manual => {
                if r.changed() {
                    self.notice(NoticeKind::Success, format!("SCAN_COMPLETE // {}", scan_summary(r)));
                } else if r.missing_tracks == 0 {
                    self.notice(NoticeKind::Info, "SCAN_COMPLETE // NO CHANGES");
                }
                if !job.prune && r.missing_tracks > 0 {
                    let _ = self.ui_tx.send(ControllerEvent::MissingTracks {
                        count: r.missing_tracks,
                        roots: job.roots.clone(),
                    });
                }
            }
            ScanKind::Watch => {
                if r.changed() {
                    self.notice(NoticeKind::Success, format!("LIBRARY_UPDATED // {}", scan_summary(r)));
                }
            }
        }
    }

    /// F5 / tray rescan: every library root, nothing dropped.
    fn rescan_library(&mut self) {
        let roots = library_roots(&self.settings.library_folders, &self.albums);
        if roots.is_empty() {
            self.notice(NoticeKind::Info, "RESCAN // LIBRARY IS EMPTY");
            return;
        }
        if self.scan_cancel.is_some() {
            self.pending_full_rescan = true;
            self.notice(NoticeKind::Info, "SCAN_IN_PROGRESS // RESCAN QUEUED");
        } else {
            self.start_scan_paths(roots, ScanKind::Manual, false);
        }
    }

    fn persist_collections(&self) {
        let data = self.collections.clone();
        let _ = self
            .ui_tx
            .send(ControllerEvent::CollectionsUpdated(data.clone()));
        let _ = self.persist_tx.send(PersistMsg::Collections(data));
    }

    fn save_settings_async(&self) {
        let _ = self
            .persist_tx
            .send(PersistMsg::Settings(self.settings.clone()));
    }

    fn save_session_state_async(&mut self) {
        // State only: the queue ids can be library-sized while shuffling.
        let state = self.queue.session_state(self.position_s);
        self.session_last_saved = Instant::now();
        let _ = self.persist_tx.send(PersistMsg::SessionState(state));
    }

    fn save_queue_async(&mut self) {
        let (state, active_ids, original_ids) = self.queue.to_session(self.position_s);
        self.session_last_saved = Instant::now();
        let _ = self.persist_tx.send(PersistMsg::SessionQueue {
            state,
            active_ids,
            original_ids,
        });
    }

    async fn shutdown(&mut self) {
        if let Some(cancel) = &self.scan_cancel {
            cancel.store(true, Ordering::Release);
        }
        self.watcher = None;
        self.watch_rx = None;

        let (state, active_ids, original_ids) = self.queue.to_session(self.position_s);
        let _ = self.persist_tx.send(PersistMsg::SessionQueue {
            state,
            active_ids,
            original_ids,
        });
        if self.settings_dirty_since.take().is_some() {
            let _ = self
                .persist_tx
                .send(PersistMsg::Settings(self.settings.clone()));
        }
        let (done_tx, done_rx) = oneshot::channel();
        if self.persist_tx.send(PersistMsg::Flush(done_tx)).is_ok() {
            let _ = tokio::time::timeout(Duration::from_secs(2), done_rx).await;
        }

        self.crossfade.shutdown_async().await;
        self.mpv.shutdown_async().await;
        self.teardown_direct_resources().await;
        if let Some(t) = self.tray.take() {
            t.shutdown().await;
        }
        if let Some(rpc) = self.discord_rpc.take() {
            rpc.shutdown().await;
        }
        self.mpris = None;
    }

    fn first_library_track(&self) -> Option<Track> {
        self.albums.first().and_then(|a| a.tracks.first().cloned())
    }

    fn emit_queue(&self) {
        let _ = self.ui_tx.send(ControllerEvent::QueueUpdated {
            tracks: self.queue.shared_tracks(),
            index: self.queue.index(),
            shuffle: self.queue.shuffle(),
            repeat: self.queue.repeat(),
            hand_len: self.queue.hand_len(),
        });
    }

    /// Sends the playback state now (play/pause, seek, new track ...).
    fn emit_playback(&mut self) {
        self.last_playback_emit = Some((
            Instant::now(),
            self.position_s.floor() as i64,
            self.is_playing,
            self.duration_s,
        ));
        let _ = self.ui_tx.send(ControllerEvent::PlaybackState {
            is_playing: self.is_playing,
            position_s: self.position_s,
            duration_s: self.duration_s,
        });
    }

    /// A position poll: sent only when the UI would show something else (another second, a new
    /// state or length) or [`PLAYBACK_TICK_MIN`] has passed, so the view is not rebuilt per poll.
    fn emit_playback_tick(&mut self) {
        if !playback_tick_due(
            self.last_playback_emit,
            Instant::now(),
            self.position_s,
            self.is_playing,
            self.duration_s,
        ) {
            return;
        }
        self.emit_playback();
    }

    fn emit_volume(&self) {
        let _ = self.ui_tx.send(ControllerEvent::VolumeState {
            volume: self.current_vol,
            muted: self.current_muted,
        });
    }

    fn emit_direct_status(&self) {
        let active = self.direct_card.is_some();
        let status = if active {
            let bits = self
                .queue
                .current()
                .map(|t| t.bits_per_sample)
                .unwrap_or(0);
            evaluate_bit_perfect(
                self.last_audio_path.input.as_ref(),
                self.last_audio_path.output.is_some(),
                self.last_hw_params.as_ref(),
                bits,
                self.last_sample_alteration.as_deref(),
            )
        } else if AlsaDirect::is_direct(&self.settings.audio_device) {
            "NOT ACTIVE. PLAYING THROUGH PIPEWIRE.".to_owned()
        } else {
            String::new()
        };
        let direct_volume_hint = match (&self.direct_control, active) {
            (_, false) => String::new(),
            (Some(control), true) => {
                let system = if self.pw_bridge.is_some() {
                    "THE SYSTEM VOLUME REACHES IT THROUGH THE (DIRECT) BRIDGE SINK."
                } else {
                    "THE SYSTEM VOLUME DOES NOT REACH THIS CARD."
                };
                format!(
                    "VOLUME SETS THE CARD'S OWN {} CONTROL, THE FILE'S SAMPLES ARE NOT TOUCHED. {system}",
                    control.to_uppercase()
                )
            }
            (None, true) => "THIS CARD HAS NO VOLUME CONTROL OF ITS OWN: THE VOLUME STAYS AT 100%. USE THE DAC'S OWN VOLUME.".to_owned(),
        };
        let _ = self.ui_tx.send(ControllerEvent::DirectStatus {
            audio_device: self.settings.audio_device.clone(),
            bit_perfect_status: status,
            direct_active: active,
            direct_volume_hint,
            audio_path: self.last_audio_path.clone(),
            pw_bridge_node_id: self.pw_bridge_node_id,
            direct_mixer_control: self.direct_control.clone(),
        });
    }

    /// Live MPRIS position (no PropertiesChanged); a jump of more than 2 s emits `Seeked`.
    async fn set_mpris_position(&self, seconds: f64) {
        if let Some(mpris) = &self.mpris {
            mpris.update_position((seconds.max(0.0) * 1_000_000.0) as i64).await;
        }
    }

    /// Pushes current track & playback state to Discord Rich Presence when enabled.
    fn sync_discord_rpc(&self) {
        let Some(rpc) = &self.discord_rpc else {
            return;
        };
        let track = self.queue.current();
        let status = if track.is_none() || (self.mpv_loaded_track_id.is_none() && !self.is_playing) {
            DiscordPlaybackStatus::Stopped
        } else if self.is_playing {
            DiscordPlaybackStatus::Playing
        } else {
            DiscordPlaybackStatus::Paused
        };
        rpc.update_presence(DiscordPresence {
            status,
            title: track.map(|t| t.title.clone()).unwrap_or_default(),
            artist: track.map(|t| t.artist.clone()).unwrap_or_default(),
            album: track.map(|t| t.album.clone()).unwrap_or_default(),
            position_s: self.position_s.max(0.0),
            duration_s: self.duration_s.max(0.0),
            artwork_url: None,
        });
    }

    async fn sync_mpris(&self) {
        self.sync_discord_rpc();
        let Some(mpris) = &self.mpris else {
            return;
        };
        let track = self.queue.current();
        let status = if track.is_none() || (self.mpv_loaded_track_id.is_none() && !self.is_playing) {
            "Stopped"
        } else if self.is_playing {
            "Playing"
        } else {
            "Paused"
        };
        let art_url = track
            .and_then(|t| t.cover_full_path.as_deref().or(t.cover_thumb_path.as_deref()))
            .map(CoverPaths::to_file_url);
        let state = MprisState {
            playback_status: status.to_owned(),
            track_id: track.map(|t| t.id.clone()),
            title: track.map(|t| t.title.clone()),
            artist: track.map(|t| t.artist.clone()),
            album: track.map(|t| t.album.clone()),
            art_url,
            length_us: (self.duration_s * 1_000_000.0) as i64,
            position_us: (self.position_s * 1_000_000.0) as i64,
            volume: self.current_vol,
            can_go_next: self.queue.can_go_next(),
            can_go_previous: self.queue.can_go_previous(),
            can_play: true,
            can_pause: true,
            can_seek: track.is_some(),
            loop_status: self.queue.repeat().mpris_loop_status().to_owned(),
            shuffle: self.queue.shuffle(),
        };
        mpris.update_state(state).await;
    }

    async fn sync_tray(&self) {
        let Some(tray) = &self.tray else {
            return;
        };
        let title = self
            .queue
            .current()
            .map(|t| format!("Retrograde — {} - {}", t.artist, t.title))
            .unwrap_or_else(|| "Retrograde".to_owned());
        tray.update_title(title).await;
    }

    /// The tray's checkboxes follow the settings, whoever changed them.
    async fn sync_tray_toggles(&self) {
        let Some(tray) = &self.tray else {
            return;
        };
        tray.update_toggles(TrayToggles {
            minimize_to_tray: self.settings.minimize_to_tray != Some(false),
            watch_folders: self.settings.watch_folders,
            accent_from_cover: self.settings.accent_from_cover,
        })
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_open_uri_handles_plain_paths_file_urls_and_missing() {
        let dir = std::env::temp_dir().join(format!("retrograde-open-uri-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("a b.flac");
        std::fs::write(&file, b"x").unwrap();
        let canon = std::fs::canonicalize(&file).unwrap();

        assert_eq!(resolve_open_uri(file.to_str().unwrap()), Some(canon.clone()));
        let url = CoverPaths::to_file_url(&file);
        assert_eq!(resolve_open_uri(&url), Some(canon));
        // Non-existent plain paths and URLs are rejected rather than imported blindly.
        assert_eq!(resolve_open_uri(dir.join("missing.flac").to_str().unwrap()), None);
        assert_eq!(resolve_open_uri("file:///definitely/not/here.flac"), None);
        assert_eq!(resolve_open_uri("   "), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn album_at(paths: &[&str]) -> Album {
        Album {
            name: "A".to_owned(),
            artist: "Artist".to_owned(),
            cover_thumb_path: None,
            cover_full_path: None,
            year: 0,
            added_at: 0,
            tracks: paths
                .iter()
                .map(|p| Track {
                    id: (*p).to_owned(),
                    title: String::new(),
                    artist: String::new(),
                    album: "A".to_owned(),
                    duration: 0.0,
                    file_path: (*p).to_owned(),
                    track_number: 0,
                    disc_number: 0,
                    codec: String::new(),
                    sample_rate: 0,
                    bits_per_sample: 0,
                    channels: 0,
                    cover_thumb_path: None,
                    cover_full_path: None,
                })
                .collect(),
        }
    }

    #[test]
    fn library_roots_merge_folders_and_track_dirs_without_nesting() {
        let folders = vec!["/music".to_owned(), "/music/Rock".to_owned(), String::new()];
        let albums = vec![
            album_at(&["/music/Rock/A/01.flac", "/music/Rock/A/02.flac"]),
            // Imported from elsewhere (Electron library, no LibraryFolders entry).
            album_at(&["/mnt/usb/Jazz/B/01.flac"]),
            album_at(&["/mnt/usb/Jazz/C/01.flac"]),
            // A sibling whose name only starts like a root is not inside it.
            album_at(&["/musicbox/D/01.flac"]),
        ];
        let roots = library_roots(&folders, &albums);
        assert_eq!(
            roots,
            vec![
                PathBuf::from("/music"),
                PathBuf::from("/musicbox/D"),
                PathBuf::from("/mnt/usb/Jazz/B"),
                PathBuf::from("/mnt/usb/Jazz/C"),
            ]
        );
        // Nothing known: nothing to rescan (the caller says "library is empty").
        assert!(library_roots(&[], &[]).is_empty());
    }

    #[test]
    fn scan_summary_lists_only_nonzero_parts() {
        let r = ScanResult {
            added_tracks: 3,
            removed_tracks: 1,
            covers: 2,
            ..ScanResult::default()
        };
        assert_eq!(scan_summary(&r), "3 NEW, 1 REMOVED, 2 COVERS");
        assert_eq!(scan_summary(&ScanResult::default()), "");
    }

    fn dev(name: &str, desc: &str) -> AudioDevice {
        AudioDevice {
            name: name.to_owned(),
            description: desc.to_owned(),
        }
    }

    #[test]
    fn device_list_has_each_sink_once_without_the_bridge() {
        // What mpv listed on the user's machine with direct output active (U-31 screenshot).
        let mpv = vec![
            dev("auto", "Autoselect device"),
            dev("pipewire", "Default (pipewire)"),
            dev("pipewire/alsa_output.pci-0000_00_1f.3.analog-stereo", "Built-in Audio Analog Stereo"),
            dev("pipewire/retrograde-direct-bridge", "TP35 Pro, USB Audio (Direct)"),
            dev("pulse/alsa_output.pci-0000_00_1f.3.analog-stereo", "Built-in Audio Analog Stereo"),
            dev("pulse/retrograde-direct-bridge", "TP35 Pro, USB Audio (Direct)"),
            dev("alsa/sysdefault:CARD=Pro", "TP35 Pro, USB Audio/Default Audio Device"),
        ];
        let direct = vec![DirectDevice {
            name: "alsa/hw:CARD=Pro,DEV=0".to_owned(),
            description: "TP35 Pro, USB Audio".to_owned(),
            card: 0,
        }];
        let opts = build_device_options(&mpv, &direct, "alsa/hw:CARD=Pro,DEV=0");
        let labels: Vec<(&str, bool)> = opts.iter().map(|o| (o.label.as_str(), o.is_direct)).collect();
        assert_eq!(
            labels,
            vec![
                ("System Default", false),
                ("Built-in Audio Analog Stereo", false),
                ("TP35 Pro, USB Audio", true),
            ]
        );
        assert_eq!(opts[1].id, "pipewire/alsa_output.pci-0000_00_1f.3.analog-stereo");

        // A saved PulseAudio id (older settings) keeps its own entry so it shows as selected.
        let opts = build_device_options(&mpv, &[], "pulse/alsa_output.pci-0000_00_1f.3.analog-stereo");
        assert_eq!(opts.len(), 2);
        assert_eq!(opts[1].id, "pulse/alsa_output.pci-0000_00_1f.3.analog-stereo");

        // Two different sinks with the same name stay apart.
        let twins = vec![dev("pipewire/a", "DAC"), dev("pipewire/b", "DAC"), dev("pulse/b", "DAC")];
        let opts = build_device_options(&twins, &[], "auto");
        let labels: Vec<&str> = opts.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(labels, vec!["System Default", "DAC", "DAC #2"]);
    }

    #[test]
    fn saved_bridge_output_falls_back_to_auto() {
        assert_eq!(
            saved_device_replacement("pipewire/retrograde-direct-bridge").as_deref(),
            Some("auto")
        );
        assert_eq!(saved_device_replacement("pulse/retrograde-direct-bridge").as_deref(), Some("auto"));
        assert_eq!(saved_device_replacement("auto"), None);
        assert_eq!(saved_device_replacement("pipewire/alsa_output.usb-x"), None);
        // A direct device on a card that is not there keeps its setting (the DAC may be unplugged).
        assert_eq!(saved_device_replacement("alsa/hw:CARD=NoSuchCard,DEV=0"), None);
    }

    #[test]
    fn position_ticks_are_merged_until_the_second_changes() {
        let t0 = Instant::now();
        let last = Some((t0, 10_i64, true, 200.0));
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        assert!(playback_tick_due(None, t0, 0.0, true, 200.0));
        // Same second, soon after: merged.
        assert!(!playback_tick_due(last, at(100), 10.4, true, 200.0));
        // Another second shows.
        assert!(playback_tick_due(last, at(100), 11.0, true, 200.0));
        // 250 ms passed.
        assert!(playback_tick_due(last, at(250), 10.6, true, 200.0));
        // A state or length change is never held back.
        assert!(playback_tick_due(last, at(10), 10.1, false, 200.0));
        assert!(playback_tick_due(last, at(10), 10.1, true, 180.0));
    }

    #[test]
    fn direct_failure_message_names_the_holders() {
        assert_eq!(
            direct_failure_message(&["pipewire".to_owned()]),
            "DIRECT_OUTPUT // CARD BUSY: IN USE BY PIPEWIRE. PRESS PLAY TO RETRY"
        );
        assert!(direct_failure_message(&[]).contains("COULD NOT BE OPENED"));
    }

    /// Bug 1 on real hardware, with digital silence and the card's own volume at its minimum:
    /// one track goes shared → direct (taken the old way, straight after the reservation) →
    /// shared → direct (taken the fixed way, after the card is closed). Prints whether the old way
    /// failed; asserts the fixed way plays. Run with
    /// `RETRO_LIVE_DAC=alsa/hw:CARD=Pro,DEV=0 RETRO_LIVE_SHARED=pipewire/<sink> RETRO_LIVE_FILE=<silent.flac>`
    /// and nothing else using the DAC.
    #[test]
    #[ignore = "Needs a free USB DAC and its PipeWire sink; plays digital silence"]
    fn live_direct_shared_direct_on_one_track() {
        let direct = std::env::var("RETRO_LIVE_DAC").expect("RETRO_LIVE_DAC");
        let shared = std::env::var("RETRO_LIVE_SHARED").expect("RETRO_LIVE_SHARED");
        let file = std::env::var("RETRO_LIVE_FILE").expect("RETRO_LIVE_FILE");
        let card = AlsaDirect::card_of(&direct).expect("card");
        assert_eq!(AlsaDirect::pcm_is_open(&direct), Some(false), "the DAC is in use");

        struct Restore(i32, Option<(String, f64)>);
        impl Drop for Restore {
            fn drop(&mut self) {
                if let Some((c, lvl)) = &self.1 {
                    AlsaMixer::write_level(self.0, c, *lvl);
                }
            }
        }
        let control = AlsaMixer::find_control(card);
        let saved = control
            .as_deref()
            .and_then(|c| AlsaMixer::read_level(card, c).map(|l| (c.to_owned(), l)));
        let _restore = Restore(card, saved);
        if let Some(c) = control.as_deref() {
            AlsaMixer::write_level(card, c, 0.0);
        }

        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            async fn watch(rx: &mut mpsc::UnboundedReceiver<EngineEvent>, ms: u64) -> (bool, bool) {
                let (mut failed, mut opened) = (false, false);
                let end = tokio::time::Instant::now() + Duration::from_millis(ms);
                while let Ok(Some(e)) = tokio::time::timeout_at(end, rx.recv()).await {
                    match e {
                        EngineEvent::PlaybackFailed { reason, .. } => {
                            eprintln!("  PlaybackFailed: {reason}");
                            failed |= reason == AO_INIT_FAILED;
                        }
                        EngineEvent::AudioPathChanged(ap) => opened |= ap.output.is_some(),
                        EngineEvent::EndOfFile => eprintln!("  EndOfFile"),
                        _ => {}
                    }
                }
                (failed, opened)
            }

            let (tx, mut rx) = mpsc::unbounded_channel();
            let mut mpv = MpvEngine::new(tx);
            mpv.start(Some(&shared), Some("no")).expect("libmpv");
            mpv.set_volume(1.0);
            mpv.play(&file, 0.0);
            let (_, opened) = watch(&mut rx, 2500).await;
            assert!(opened, "shared output did not open");
            eprintln!("shared: PCM open by PipeWire = {:?}", AlsaDirect::pcm_is_open(&direct));

            if std::env::var("RETRO_LIVE_SKIP_NAIVE").is_err() {
                // The old way: reserve, then move mpv at once.
                let res = DeviceReservation::acquire(card, "Retrograde live test").await.expect("reserve");
                mpv.set_audio_device(&direct);
                let (naive_failed, _) = watch(&mut rx, 2000).await;
                eprintln!("old switch failed with '{AO_INIT_FAILED}': {naive_failed}");

                // Back to shared as C# does it, then the fixed way.
                mpv.set_audio_device(&shared);
                res.release().await;
                tokio::time::sleep(LEAVE_DIRECT_SETTLE).await;
                mpv.reload_audio();
                if naive_failed {
                    mpv.play(&file, 3.0);
                }
                let (_, opened) = watch(&mut rx, 2500).await;
                assert!(opened, "shared output did not reopen");
            }

            eprintln!(
                "before reserve: open={:?} holders={:?}",
                AlsaDirect::pcm_is_open(&direct),
                AlsaDirect::pcm_holders(&direct)
            );
            let res = DeviceReservation::acquire(card, "Retrograde live test").await.expect("reserve");
            eprintln!(
                "after reserve: open={:?} holders={:?}",
                AlsaDirect::pcm_is_open(&direct),
                AlsaDirect::pcm_holders(&direct)
            );
            let free = Coordinator::wait_for_card_free(&direct).await;
            eprintln!("card free after reservation: {free:?}");
            mpv.set_audio_device(&direct);
            let (failed, opened) = watch(&mut rx, 2500).await;
            let hw = AlsaDirect::read_hw_params(&direct);
            eprintln!("fixed switch: failed={failed} opened={opened} hw={hw:?}");
            mpv.shutdown();
            res.release().await;
            assert!(free.is_ok());
            assert!(!failed && opened && hw.is_some(), "direct output did not open");
        });
    }
}
