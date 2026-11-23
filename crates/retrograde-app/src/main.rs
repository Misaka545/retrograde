mod context_menu;
mod controller;
mod covers_ui;
mod devshot;
mod egg;
mod fullscreen;
mod pages;
mod popups;
mod crossfade;
mod dialogs;
mod icon_paths;
mod icons;
mod library_view;
mod playback;
mod shader;
mod theme;
mod toasts;
mod views;

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use iced::event;
use iced::futures::{SinkExt, Stream};
use iced::keyboard::{self, Key, Modifiers};
use iced::widget::{Id, canvas, operation};
use iced::{Element, Event, Point, Subscription, Task, Theme, window};
use retrograde_audio::AudioPath;
use retrograde_core::{
    Album, AppSettings, ArtistSummary, CollectionsData, CoverPalette, CoverPaths,
    DirectVolumeBridge, DisplayBackend, ExtraMap, GpuPreference, LyricsDoc, PerformanceMode,
    PlayStats, SmartPlaylistData, SmartPlaylists, SmartRule, TagEdit, Track,
};
use retrograde_linux::{
    DiscordRpcStatus, GraphicsPolicyReport, PowerProfile, RenderTier, SingleInstance,
    SingleInstanceOutcome, apply_startup_graphics_policy, pick_cover_image, pick_music_files,
    pick_music_folders,
};
use tokio::sync::mpsc;

use crate::controller::{
    ControllerCommand, ControllerEvent, DeviceOption, NoticeKind, PlaybackTickMode,
    StartupContext, WindowAction, run_coordinator,
};
use crate::covers_ui::CoverCache;
use crate::library_view::{
    LibrarySort, artist_key, fold, group_artists, search_albums, search_artists, sort_albums,
    sort_artists,
};
use crate::playback::RepeatMode;
use crate::theme::AccentAnimator;

struct BootstrapHandoff {
    cmd_rx: mpsc::UnboundedReceiver<ControllerCommand>,
    startup: StartupContext,
}

static BOOTSTRAP_HANDOFF: OnceLock<Mutex<Option<BootstrapHandoff>>> = OnceLock::new();
/// Incremented on every presented frame (`window::Event::RedrawRequested`), used to detect when
/// a Wayland compositor suspends frame callbacks for an unfocused window on another workspace.
static PRESENT_SEQ: AtomicU64 = AtomicU64::new(0);
/// Armed only while `App` has paused ticks due to compositor suspension, so the very next
/// `RedrawRequested` wakes `App` once (even if the window becomes visible without keyboard focus).
static WAKE_ON_PRESENT: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuOption(pub GpuPreference);

impl fmt::Display for GpuOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            GpuPreference::Igpu => f.write_str("iGPU Locked (Hybrid Power Saver)"),
            GpuPreference::Auto => f.write_str("Auto (System Default)"),
            GpuPreference::Dgpu => f.write_str("dGPU (High Performance)"),
            GpuPreference::Software => f.write_str("Software CPU (tiny-skia)"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayOption(pub DisplayBackend);

impl fmt::Display for DisplayOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            DisplayBackend::Auto => f.write_str("Auto (Wayland / X11)"),
            DisplayBackend::Wayland => f.write_str("Wayland Native"),
            DisplayBackend::X11 => f.write_str("X11 / XWayland"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerfOption(pub PerformanceMode);

impl fmt::Display for PerfOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            PerformanceMode::Auto => f.write_str("Auto (Follow PowerProfilesDaemon)"),
            PerformanceMode::Performance => f.write_str("Performance (120 FPS + Effects)"),
            PerformanceMode::Balanced => f.write_str("Balanced (60 FPS + Effects)"),
            PerformanceMode::PowerSaver => f.write_str("Power Saver (30 FPS, Effects Off)"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeOption(pub DirectVolumeBridge);

impl fmt::Display for BridgeOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            DirectVolumeBridge::VirtualSink => {
                f.write_str("PipeWire Virtual Bridge (takes system slider / default sink)")
            }
            DirectVolumeBridge::HardwareOnly => f.write_str("ALSA Hardware Mixer Only"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayGainOption(pub String);

impl fmt::Display for ReplayGainOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.as_str() {
            "track" => f.write_str("Track Gain"),
            "album" => f.write_str("Album Gain"),
            _ => f.write_str("Off (Bit-Perfect)"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmartFieldOption {
    pub key: &'static str,
    pub label: &'static str,
}

impl fmt::Display for SmartFieldOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmartOpOption {
    pub op: &'static str,
    pub label: &'static str,
}

impl fmt::Display for SmartOpOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmartSortOption {
    pub key: &'static str,
    pub label: &'static str,
}

impl fmt::Display for SmartSortOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Library,
    NowPlaying,
    Playlists,
    Search,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchTab {
    All,
    Tracks,
    Albums,
    Artists,
    Playlists,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryMode {
    Albums,
    Artists,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistView {
    Liked,
    Custom(i64),
    Smart(i64),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AlbumKey {
    pub name: String,
    pub artist: String,
}

impl AlbumKey {
    pub fn from_album(album: &Album) -> Self {
        Self {
            name: album.name.clone(),
            artist: album.artist.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TagDraft {
    pub is_track: bool,
    pub paths: Vec<PathBuf>,
    pub album_key: Option<String>,
    pub target_label: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub year: String,
    pub track: String,
    pub disc: String,
    pub orig_title: String,
    pub orig_artist: String,
    pub orig_album: String,
    pub orig_album_artist: String,
    pub orig_year: String,
    pub orig_track: String,
    pub orig_disc: String,
    pub cover_preview: Option<PathBuf>,
    pub new_cover: Option<PathBuf>,
    pub remove_cover: bool,
    pub saving: bool,
}

#[derive(Debug, Clone)]
pub struct SmartDraft {
    pub id: i64,
    pub name: String,
    pub match_all: bool,
    pub rules: Vec<SmartRule>,
    pub sort: String,
    pub limit: String,
    pub extra: ExtraMap,
    pub match_count: usize,
}

#[derive(Debug, Clone)]
pub enum Message {
    Noop,
    WindowOpened(window::Id),
    MiniWindowOpened(window::Id),
    WindowCloseRequested(window::Id),
    WindowClosed(window::Id),
    WindowFocused(window::Id, bool),
    WindowMoved(window::Id, Point),
    FileHovered,
    FilesHoveredLeft,
    FileDropped(PathBuf),
    /// Imports every file dropped in the last moment as one batch.
    FlushDrops,
    GoHome,
    OpenArtist(String),
    CloseArtistDetail,
    SetSearchTab(SearchTab),
    PlayDetail(pages::DetailCtx, usize),
    QueueDetail(pages::DetailCtx),
    PlaySearchTrack(usize),
    ToggleCreatePlaylist(bool),
    ToggleQueuePanel,
    ToggleAudioPanel,
    SetAudioOutputTab(bool),
    SetFsLyricsTab(bool),
    GoToCurrentAlbum,
    ShotStep(usize),
    ShotGo(usize),
    ShotTaken(usize, window::Screenshot),
    FocusSearch,
    OpenPlaylistPage(PlaylistView),
    SetBankPlaylists(bool),
    KeyShortcut {
        key: Key,
        modifiers: Modifiers,
        ignored: bool,
    },
    /// Second stage of a shortcut: dispatched only after checking no `text_input` has focus.
    FocusCheckedShortcut {
        key: Key,
        modifiers: Modifiers,
        text_input_focused: bool,
    },
    MainWindowMinimized(window::Id, bool),
    WindowPresented(window::Id),
    RestoreLibScroll,
    AnimTick(Instant),
    ToggleMiniPlayer,
    DragMiniWindow,
    CloseMiniPlayerToMain,
    MiniHover(bool),
    DragMainWindow,
    MinimizeMainWindow,
    ToggleMaximizeMainWindow,
    RequestCloseMainWindow,
    ResizeMainWindow(window::Direction),
    LibScrolled(f32),
    DetailScrolled(f32),
    BankScrolled(f32, f32),
    ToggleShortcutsModal,
    ToggleSidebar,
    CloseAlbumDetail,
    Controller(ControllerEvent),
    SelectTab(Tab),
    SetLibraryMode(LibraryMode),
    SearchChanged(String),
    SelectLibrarySort(LibrarySort),
    SelectAlbum(AlbumKey),
    SelectArtist(String),
    PlayAlbumTrack(AlbumKey, usize),
    QueueAlbum(AlbumKey),
    PlayTrackFromList(Vec<Track>, usize),
    PlayNext(Track),
    EnqueueTracks(Vec<Track>),
    JumpToQueueIndex(usize),
    RemoveQueueIndex(usize),
    ClearQueue,
    TogglePlayPause,
    Previous,
    Next,
    ToggleShuffle,
    CycleRepeat,
    SeekChanged(f64),
    SeekDragging(f64),
    SeekCommit,
    VolumeDragging(f64),
    VolumeCommit,
    ToggleMute,
    SelectDevice(DeviceOption),
    RefreshDevices,
    ToggleLike(String),
    SelectPlaylistView(PlaylistView),
    NewPlaylistNameChanged(String),
    CreatePlaylist,
    DeletePlaylist(i64),
    RenamePlaylistInput(String),
    RenamePlaylistSubmit(i64),
    AddToTargetPlaylist(String),
    RemoveFromPlaylist { playlist_id: i64, track_id: String },
    NewSmartPlaylist,
    EditSmartPlaylist(SmartPlaylistData),
    SmartDraftNameChanged(String),
    SmartRuleFieldChanged(usize, SmartFieldOption),
    SmartRuleOpChanged(usize, SmartOpOption),
    SmartRuleValueChanged(usize, String),
    AddSmartRule,
    RemoveSmartRule(usize),
    SmartDraftSortChanged(SmartSortOption),
    SmartDraftLimitChanged(String),
    SmartDraftMatchAllChanged(bool),
    SaveSmartPlaylist,
    CancelSmartPlaylist,
    DeleteSmartPlaylist(i64),
    ToggleLikeTracks(Vec<String>),
    EnterSelectMode,
    ExitSelectMode,
    ToggleAlbumSelected(AlbumKey),
    SelectAllAlbums,
    DeleteSelectedAlbums,
    ToggleAddMusicMenu,
    PickFiles,
    FilesPicked(Result<Vec<PathBuf>, String>),
    PickPlaylistCover(i64),
    PlaylistCoverPicked(i64, Result<Option<PathBuf>, String>),
    OpenTrackMenu {
        track: Track,
        album_artist: String,
        album_year: i32,
        removable_playlist: Option<i64>,
        pos: Point,
    },
    OpenAlbumMenu(AlbumKey, Point),
    OpenPlaylistMenu(i64, Point),
    OpenDetailMenu(pages::DetailCtx, Point),
    CloseContextMenu,
    ContextToggleLike(String),
    ContextToggleLikeTracks(Vec<String>),
    ContextPlayNext(Vec<Track>),
    ContextEnqueue(Vec<Track>),
    ContextOpenTrackTags {
        track: Track,
        album_artist: String,
        album_year: i32,
    },
    ContextOpenAlbumTags(AlbumKey),
    ContextTogglePlaylistTracks {
        playlist_id: i64,
        track_ids: Vec<String>,
    },
    ContextRemoveFromPlaylist {
        playlist_id: i64,
        track_id: String,
    },
    ContextRemovePlaylistCover(i64),
    ContextDeletePlaylist(i64),
    ContextDeleteSmart(i64),
    ContextEditSmart(SmartPlaylistData),
    ContextDeleteAlbumFromMenu(AlbumKey),
    ContextDeleteAlbumFromDetail(AlbumKey),
    SetEqEnabled(bool),
    SelectEqPreset(String),
    SetEqBand { band: usize, gain: f64 },
    SelectReplayGain(ReplayGainOption),
    SetCrossfadeSeconds(i32),
    OpenTagEditor(Track, String, i32),
    OpenAlbumTagEditor(AlbumKey),
    CloseTagEditor,
    TagTitleChanged(String),
    TagArtistChanged(String),
    TagAlbumChanged(String),
    TagAlbumArtistChanged(String),
    TagYearChanged(String),
    TagTrackChanged(String),
    TagDiscChanged(String),
    TagPickCover,
    TagCoverPicked(Result<Option<PathBuf>, String>),
    TagRemoveCover,
    SaveTrackTags,
    PickFolders,
    FoldersPicked(Result<Vec<PathBuf>, String>),
    RemoveLibraryFolder(String),
    RescanLibrary,
    SetGpuPreference(GpuOption),
    SetDisplayBackend(DisplayOption),
    SetPerformanceMode(PerfOption),
    SetDirectBridge(BridgeOption),
    ToggleTray(bool),
    ToggleMpris(bool),
    ToggleDiscordRpc(bool),
    ToggleDiscordArtwork(bool),
    DiscordClientIdChanged(String),
    ApplyDiscordClientId,
    ResetDiscordClientId,
    ToggleWatchFolders(bool),
    ToggleAccentFromCover(bool),
    ToggleMinimizeToTray(bool),
    VisualizerBarPressed(usize),
    VisualizerBarRelease(usize, u64),
    EggMorseTimeout(u64),
    EggAssetsReady(Option<egg::EggAssets>),
    EggImageHover(bool),
    CloseEgg,
    ToastExpire(u64),
    ToastRemove(u64),
    /// The search text after the debounce: rebuilds the results if no key came since (`search_gen`).
    SearchDebounced(u64),
    ConfirmAccept,
    ConfirmCancel,
    BackgroundRemember(bool),
    /// BACKGROUND_PLAY answer: true = minimize to the tray, false = quit.
    BackgroundChoice(bool),
    ToggleSortMenu,
    ToggleBankMenu,
    WindowResized(window::Id, iced::Size),
    /// The cover color of a detail page, worked out off the UI thread; dropped if the page changed.
    DetailTintReady(String, Option<iced::Color>),
    /// The full-screen backdrop of a track, blurred off the UI thread.
    FsBlurReady(String, Option<iced::widget::image::Handle>),
    /// Player bar / page meta: opens the artist page of the playing track.
    OpenCurrentArtist,
    /// The back button of detail pages: one page back, or out to where the pages were opened from.
    Back,
    /// Watchdog fallback if the coordinator never reports `ShutdownComplete`.
    ForceExit,
}

pub struct App {
    pub cmd_tx: mpsc::UnboundedSender<ControllerCommand>,
    pub main_window: Option<window::Id>,
    pub mini_window: Option<window::Id>,
    pub main_window_focused: bool,
    pub main_window_minimized: bool,
    pub window_visible: bool,
    playback_tick_mode: PlaybackTickMode,
    watchdog: PresentWatchdog,
    pub shutting_down: bool,
    pub active_tab: Tab,
    pub previous_tab: Tab,
    pub library_mode: LibraryMode,
    pub album_detail_open: bool,
    pub search_query: String,
    pub search_input_id: Id,
    pub library_sort: LibrarySort,
    pub graphics_report: GraphicsPolicyReport,
    pub settings: AppSettings,
    pub power_profile: PowerProfile,
    pub render_tier: RenderTier,
    pub covers: CoverCache,
    pub accent_anim: AccentAnimator,
    pub orbit_theme: Theme,
    /// Monotonic shader phase; speed is integrated per frame so play/pause never jumps.
    pub shader_phase: f64,
    /// Mini-player was moved since the last `push_settings()`.
    pub mini_pos_dirty: bool,
    pub last_frame_at: Option<Instant>,
    pub hovered_files: usize,
    /// The sidebar's DATA_BANK shows playlists instead of albums.
    pub bank_playlists: bool,
    pub artist_detail_open: bool,
    pub search_tab: SearchTab,
    pub search_tracks: Vec<Track>,
    pub search_playlists: Vec<(PlaylistView, String, usize)>,
    pub create_playlist_open: bool,
    /// Player-bar popups: the queue, and the sound popup with its output tab.
    pub queue_open: bool,
    pub audio_open: bool,
    pub audio_output_tab: bool,
    /// Full-screen player: the reveal in progress, how far open it is (0..1), the blurred backdrop of the
    /// current track, whether the lyrics tab shows, and the lyric line last scrolled to.
    pub fs_anim: Option<fullscreen::FsAnim>,
    pub fs_share: f32,
    pub fs_blur: Option<(String, iced::widget::image::Handle)>,
    pub fs_lyrics_tab: bool,
    fs_last_lyric: Option<usize>,
    pub lyrics_scroll_id: Id,
    /// Cover color of the open album page (its header fades from it).
    pub detail_tint: Option<iced::Color>,
    /// Developer screenshot run (RETRO_SHOT), see devshot.rs.
    shot: Option<devshot::Plan>,
    dropped_files: Vec<PathBuf>,
    pub show_shortcuts_modal: bool,
    pub albums: std::sync::Arc<[Album]>,
    pub artists: Vec<ArtistSummary>,
    /// The library grid order (every album / artist, never filtered by the search).
    pub library_albums: Vec<usize>,
    pub library_artists: Vec<usize>,
    /// Sidebar album indices sorted A-Z.
    pub sidebar_albums: Vec<usize>,
    pub select_mode: bool,
    pub selected_albums: HashSet<AlbumKey>,
    pub deleting_albums: bool,
    pub context_menu: Option<context_menu::ContextMenu>,
    pub add_music_menu_open: bool,
    pub smart_counts: HashMap<i64, usize>,
    pub visualizer_cache: canvas::Cache,
    pub sidebar_width: f32,
    pub sidebar_anim: Option<(Instant, f32, f32)>,
    last_titlebar_press: Option<Instant>,
    pub mini_hovered: bool,
    pub mini_marquee_phase: f32,
    pub lib_scroll_y: f32,
    /// Unquantized grid offset and the grid scrollable's id, to put the grid back after a detail page.
    pub lib_scroll_raw: f32,
    pub lib_scroll_id: Id,
    /// Offset to put the grid back to; scroll reports are ignored until then (a fresh scrollable reports 0).
    pub lib_restore: Option<f32>,
    pub detail_scroll_y: f32,
    pub bank_scroll_y: f32,
    pub bank_viewport_h: Option<f32>,
    pub bank_scroll_id: Id,
    /// SEARCH results.
    pub search_albums: Vec<usize>,
    pub search_artists: Vec<usize>,
    search_gen: u64,
    /// Play stats reached the library order once (they may arrive after the library).
    stats_seen: bool,
    pub toasts: toasts::Toasts,
    pub scan: Option<toasts::ScanState>,
    pub confirm: Option<dialogs::ConfirmDialog>,
    pub bg_prompt: bool,
    pub bg_remember: bool,
    pub sort_menu_open: bool,
    pub bank_menu_open: bool,
    /// Main window size, for the grid columns and the queue popup height.
    pub window_size: iced::Size,
    /// The page whose cover color was last asked for, and the track whose backdrop was last blurred.
    tint_for: Option<String>,
    fs_blur_tried: Option<String>,
    /// Detail pages opened from a detail page, for BACK (at most 30), and where the first one was opened from.
    page_stack: Vec<pages::Page>,
    view_before_detail: Tab,
    pub create_input_id: Id,
    pub queue_hand_len: usize,
    pub track_map: HashMap<String, (usize, usize)>,
    pub liked_set: HashSet<String>,
    pub cached_smart_tracks: Vec<Track>,
    pub collections: CollectionsData,
    pub stats: PlayStats,
    pub selected_album: Option<AlbumKey>,
    pub selected_artist: Option<String>,
    pub queue_tracks: Arc<Vec<Track>>,
    pub queue_index: Option<usize>,
    pub shuffle: bool,
    pub repeat_mode: RepeatMode,
    pub current_track: Option<Track>,
    pub lyrics: Option<LyricsDoc>,
    pub is_playing: bool,
    pub position_s: f64,
    pub duration_s: f64,
    pub seeking_pos: Option<f64>,
    pub volume: f64,
    pub dragging_volume: Option<f64>,
    pub last_volume_emit: Option<Instant>,
    pub muted: bool,
    pub tray_available: bool,
    pub devices: Vec<DeviceOption>,
    pub audio_device: String,
    pub bit_perfect_status: String,
    /// Direct (bit-perfect) output is playing to the card: EQ, ReplayGain, crossfade bypassed.
    pub direct_active: bool,
    pub direct_volume_hint: String,
    pub audio_path: AudioPath,
    pub pw_bridge_node_id: Option<u32>,
    pub direct_mixer_control: Option<String>,
    pub playlist_view: PlaylistView,
    pub new_playlist_name: String,
    pub rename_playlist_buf: String,
    pub smart_draft: Option<SmartDraft>,
    pub tag_draft: Option<TagDraft>,
    pub tag_status: Option<Result<String, String>>,
    pub discord_rpc_status: DiscordRpcStatus,
    pub discord_client_id_draft: String,
    pub egg_morse: egg::EggMorseState,
    pub egg_open: bool,
    pub egg_hovered: bool,
    pub egg_color_alpha: f32,
    pub egg_pulse_phase: f32,
    pub egg_assets: Option<egg::EggAssets>,
    pub egg_loading: bool,
}

impl App {
    pub fn target_playlist_id(&self) -> Option<i64> {
        match self.playlist_view {
            PlaylistView::Custom(id) if self.collections.playlists.iter().any(|p| p.id == id) => {
                Some(id)
            }
            _ => self.collections.playlists.first().map(|p| p.id),
        }
    }

    pub fn find_album(&self, key: &AlbumKey) -> Option<&Album> {
        self.albums
            .iter()
            .find(|a| a.name == key.name && a.artist == key.artist)
    }

    pub fn find_artist(&self, key: &str) -> Option<&ArtistSummary> {
        self.artists.iter().find(|a| artist_key(&a.name) == key)
    }

    pub fn lookup_track(&self, track_id: &str) -> Option<&Track> {
        let &(aidx, tidx) = self.track_map.get(track_id)?;
        self.albums.get(aidx)?.tracks.get(tidx)
    }

    /// The SEARCH results, like the original's `RebuildSearch`: tracks by title or artist, albums by name or
    /// artist, artists by name, playlists by name. Never touches the library grid.
    fn rebuild_search(&mut self) {
        self.search_tracks.clear();
        self.search_playlists.clear();
        self.search_albums.clear();
        self.search_artists.clear();
        let q = fold(self.search_query.trim());
        if q.is_empty() {
            return;
        }
        for a in self.albums.iter() {
            for t in &a.tracks {
                if fold(&t.title).contains(&q) || fold(&t.artist).contains(&q) {
                    self.search_tracks.push(t.clone());
                }
            }
        }
        self.search_albums = search_albums(&self.albums, &q);
        self.search_artists = search_artists(&self.artists, &q);
        for p in &self.collections.playlists {
            if fold(&p.name).contains(&q) {
                self.search_playlists.push((PlaylistView::Custom(p.id), p.name.clone(), p.track_ids.len()));
            }
        }
    }

    /// The library grid order (albums and artists) in the current sort.
    fn rebuild_library_view(&mut self) {
        self.library_albums = sort_albums(&self.albums, self.library_sort, &self.stats);
        self.library_artists = sort_artists(&self.artists, self.library_sort, &self.stats);
        let mut sb: Vec<usize> = (0..self.albums.len()).collect();
        sb.sort_by_cached_key(|&i| fold(&self.albums[i].name));
        self.sidebar_albums = sb;
        let albums = &self.albums;
        self.selected_albums
            .retain(|k| albums.iter().any(|a| crate::AlbumKey::from_album(a) == *k));

        let album_exists = self
            .selected_album
            .as_ref()
            .is_some_and(|key| self.find_album(key).is_some());
        if !album_exists {
            self.selected_album = None;
            self.album_detail_open = false;
        }
        let artist_exists = self
            .selected_artist
            .as_deref()
            .is_some_and(|key| self.find_artist(key).is_some());
        if !artist_exists {
            self.selected_artist = None;
            self.artist_detail_open = false;
        }
    }

    fn rebuild_smart_counts(&mut self) {
        self.smart_counts.clear();
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        for sp in self.collections.smart.iter().flatten() {
            let count = SmartPlaylists::evaluate(
                sp,
                self.albums
                    .iter()
                    .flat_map(|a| a.tracks.iter().map(move |t| (t, a))),
                |tid| self.stats.get(tid),
                |tid| self.liked_set.contains(tid),
                now_ms,
            )
            .len();
            self.smart_counts.insert(sp.id, count);
        }
    }

    fn recompute_smart_draft_count(&mut self) {
        let Some(d) = &mut self.smart_draft else {
            return;
        };
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let sp = SmartPlaylistData {
            id: d.id,
            name: d.name.clone(),
            match_all: d.match_all,
            rules: d.rules.clone(),
            sort: d.sort.clone(),
            limit: d.limit.trim().parse::<usize>().unwrap_or(0),
            extra: ExtraMap::new(),
        };
        d.match_count = SmartPlaylists::evaluate(
            &sp,
            self.albums
                .iter()
                .flat_map(|a| a.tracks.iter().map(move |t| (t, a))),
            |tid| self.stats.get(tid),
            |tid| self.liked_set.contains(tid),
            now_ms,
        )
        .len();
    }

    fn rebuild_track_map(&mut self) {
        self.track_map.clear();
        for (aidx, album) in self.albums.iter().enumerate() {
            for (tidx, track) in album.tracks.iter().enumerate() {
                self.track_map.insert(track.id.clone(), (aidx, tidx));
            }
        }
    }

    fn rebuild_liked_set(&mut self) {
        self.liked_set = self.collections.liked.iter().cloned().collect();
    }

    fn rebuild_smart_cache(&mut self) {
        let PlaylistView::Smart(id) = self.playlist_view else {
            self.cached_smart_tracks.clear();
            return;
        };
        let smart_list = self.collections.smart.as_deref().unwrap_or(&[]);
        let Some(sp) = smart_list.iter().find(|s| s.id == id) else {
            self.cached_smart_tracks.clear();
            return;
        };
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        self.cached_smart_tracks = SmartPlaylists::evaluate(
            sp,
            self.albums
                .iter()
                .flat_map(|a| a.tracks.iter().map(move |t| (t, a))),
            |tid| self.stats.get(tid),
            |tid| self.liked_set.contains(tid),
            now_ms,
        );
    }
}

fn main() -> iced::Result {
    // Before any thread exists, so every thread shares the capped arenas.
    let arenas_capped = retrograde_linux::limit_malloc_arenas();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    tracing::debug!(arenas_capped, "malloc arena limit");

    // A panic is written to native-error.log (as the original's fatal errors), then reported as usual
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        retrograde_core::error_log::write("fatal", &info.to_string());
        default_hook(info);
    }));

    let args: Vec<String> = std::env::args().collect();
    let skip_single_instance = args.iter().any(|a| a == "--no-single-instance");
    let open_args: Vec<String> = args
        .iter()
        .skip(1)
        .filter(|a| !a.starts_with("--") && !a.trim().is_empty())
        .map(|a| normalize_open_arg(a))
        .collect();

    let settings = AppSettings::load();
    // SAFETY: called on the main thread at startup before SingleInstance, Tokio, or Iced threads spawn.
    let graphics_report = unsafe { apply_startup_graphics_policy(&settings) };

    let _single_guard;
    let single_instance_rx = if !skip_single_instance {
        match SingleInstance::acquire(&CoverPaths::app_dir(), &open_args) {
            Ok(SingleInstanceOutcome::SecondaryNotified) => {
                return Ok(());
            }
            Ok(SingleInstanceOutcome::Primary(guard, rx)) => {
                _single_guard = Some(guard);
                Some(rx)
            }
            Err(_) => {
                _single_guard = None;
                None
            }
        }
    } else {
        _single_guard = None;
        None
    };

    let initial_vol = settings.volume.clamp(0.0, 1.0);
    let initial_device = settings.audio_device.clone();
    let initial_tier = RenderTier::resolve(settings.performance_mode, PowerProfile::Balanced);

    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let handoff = BootstrapHandoff {
        cmd_rx,
        startup: StartupContext {
            settings: settings.clone(),
            single_instance_rx,
            open_uris: open_args,
        },
    };
    let _ = BOOTSTRAP_HANDOFF.set(Mutex::new(Some(handoff)));

    let app_init = InitialAppConfig {
        cmd_tx,
        settings,
        graphics_report,
        initial_vol,
        initial_device,
        initial_tier,
    };
    let _ = INITIAL_CONFIG.set(Mutex::new(Some(app_init)));

    // Avoid allocating a full-window 4x MSAA render target in i915 shmem; quads/borders, text,
    // SVGs, and album covers do not go through the MSAA triangle blit.
    iced::daemon(App::boot, App::update, App::view)
        .title(App::title)
        .subscription(App::subscription)
        .theme(App::theme)
        .antialiasing(false)
        .executor::<CappedRuntime>()
        .run()
}

/// iced's tokio executor with a few worker threads instead of one per CPU: the controller and
/// the UI streams need little, and blocking work goes to `spawn_blocking` (pool left as is).
struct CappedRuntime(tokio::runtime::Runtime);

impl iced::Executor for CappedRuntime {
    fn new() -> Result<Self, iced::futures::io::Error> {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(3)
            .thread_name("retrograde-rt")
            .enable_all()
            .build()
            .map(Self)
    }

    #[allow(clippy::let_underscore_future)]
    fn spawn(&self, future: impl Future<Output = ()> + Send + 'static) {
        let _ = self.0.spawn(future);
    }

    fn enter<R>(&self, f: impl FnOnce() -> R) -> R {
        let _guard = self.0.enter();
        f()
    }

    fn block_on<T>(&self, future: impl Future<Output = T>) -> T {
        self.0.block_on(future)
    }
}

struct InitialAppConfig {
    cmd_tx: mpsc::UnboundedSender<ControllerCommand>,
    settings: AppSettings,
    graphics_report: GraphicsPolicyReport,
    initial_vol: f64,
    initial_device: String,
    initial_tier: RenderTier,
}

static INITIAL_CONFIG: OnceLock<Mutex<Option<InitialAppConfig>>> = OnceLock::new();

impl App {
    fn boot() -> (Self, Task<Message>) {
        let cfg = INITIAL_CONFIG
            .get()
            .and_then(|m| m.lock().ok())
            .and_then(|mut g| g.take())
            .expect("initial app config missing");

        let (main_id, open_task) = open_main_window();
        let sort = LibrarySort::from_key(&cfg.settings.library_sort);
        let lib_mode = if cfg.settings.library_view == "artists" {
            LibraryMode::Artists
        } else {
            LibraryMode::Albums
        };

        let sidebar_visible = cfg.settings.sidebar_visible;
        let discord_client_id_draft = cfg.settings.discord_client_id.clone();
        let accent_anim = AccentAnimator::default();
        let orbit_theme = theme::orbit_theme(accent_anim.current_accent);

        let state = Self {
            cmd_tx: cfg.cmd_tx,
            main_window: Some(main_id),
            mini_window: None,
            main_window_focused: true,
            main_window_minimized: false,
            window_visible: true,
            playback_tick_mode: PlaybackTickMode::Active,
            watchdog: PresentWatchdog::default(),
            shutting_down: false,
            active_tab: Tab::Library,
            previous_tab: Tab::Library,
            library_mode: lib_mode,
            album_detail_open: false,
            search_query: String::new(),
            search_input_id: Id::new("library-search"),
            library_sort: sort,
            graphics_report: cfg.graphics_report,
            settings: cfg.settings,
            power_profile: PowerProfile::Balanced,
            render_tier: cfg.initial_tier,
            covers: CoverCache::default(),
            accent_anim,
            orbit_theme,
            shader_phase: 0.0,
            mini_pos_dirty: false,
            last_frame_at: None,
            hovered_files: 0,
            bank_playlists: false,
            artist_detail_open: false,
            search_tab: SearchTab::All,
            search_tracks: Vec::new(),
            search_playlists: Vec::new(),
            create_playlist_open: false,
            queue_open: false,
            audio_open: false,
            audio_output_tab: false,
            fs_anim: None,
            fs_share: 0.0,
            fs_blur: None,
            fs_lyrics_tab: false,
            fs_last_lyric: None,
            lyrics_scroll_id: Id::unique(),
            detail_tint: None,
            shot: devshot::Plan::from_env(),
            dropped_files: Vec::new(),
            show_shortcuts_modal: false,
            albums: std::sync::Arc::from(Vec::new()),
            artists: Vec::new(),
            library_albums: Vec::new(),
            library_artists: Vec::new(),
            sidebar_albums: Vec::new(),
            select_mode: false,
            selected_albums: HashSet::new(),
            deleting_albums: false,
            context_menu: None,
            add_music_menu_open: false,
            smart_counts: HashMap::new(),
            visualizer_cache: canvas::Cache::default(),
            sidebar_width: if sidebar_visible { 244.0 } else { 60.0 },
            sidebar_anim: None,
            last_titlebar_press: None,
            mini_hovered: false,
            mini_marquee_phase: 0.0,
            lib_scroll_y: 0.0,
            lib_scroll_raw: 0.0,
            lib_scroll_id: Id::unique(),
            lib_restore: None,
            detail_scroll_y: 0.0,
            bank_scroll_y: 0.0,
            bank_viewport_h: None,
            bank_scroll_id: Id::unique(),
            search_albums: Vec::new(),
            search_artists: Vec::new(),
            search_gen: 0,
            stats_seen: false,
            toasts: toasts::Toasts::default(),
            scan: None,
            confirm: None,
            bg_prompt: false,
            bg_remember: false,
            sort_menu_open: false,
            bank_menu_open: false,
            window_size: iced::Size::new(1200.0, 800.0),
            tint_for: None,
            fs_blur_tried: None,
            page_stack: Vec::new(),
            view_before_detail: Tab::Library,
            create_input_id: Id::new("create-playlist-name"),
            queue_hand_len: 0,
            track_map: HashMap::new(),
            liked_set: HashSet::new(),
            cached_smart_tracks: Vec::new(),
            collections: CollectionsData::default(),
            stats: PlayStats::default(),
            selected_album: None,
            selected_artist: None,
            queue_tracks: Arc::default(),
            queue_index: None,
            shuffle: false,
            repeat_mode: RepeatMode::Off,
            current_track: None,
            lyrics: None,
            is_playing: false,
            position_s: 0.0,
            duration_s: 0.0,
            seeking_pos: None,
            volume: cfg.initial_vol,
            dragging_volume: None,
            last_volume_emit: None,
            muted: false,
            tray_available: false,
            devices: vec![DeviceOption {
                id: cfg.initial_device.clone(),
                label: cfg.initial_device.clone(),
                is_direct: false,
            }],
            audio_device: cfg.initial_device,
            bit_perfect_status: String::new(),
            direct_active: false,
            direct_volume_hint: String::new(),
            audio_path: AudioPath {
                input: None,
                output: None,
            },
            pw_bridge_node_id: None,
            direct_mixer_control: None,
            playlist_view: PlaylistView::Liked,
            new_playlist_name: String::new(),
            rename_playlist_buf: String::new(),
            smart_draft: None,
            tag_draft: None,
            tag_status: None,
            discord_rpc_status: DiscordRpcStatus::Disabled,
            discord_client_id_draft,
            egg_morse: egg::EggMorseState::default(),
            egg_open: false,
            egg_hovered: false,
            egg_color_alpha: 0.0,
            egg_pulse_phase: 0.0,
            egg_assets: None,
            egg_loading: false,
        };

        (state, open_task.map(Message::WindowOpened))
    }

    fn title(&self, window_id: window::Id) -> String {
        if self.mini_window == Some(window_id) {
            return match &self.current_track {
                Some(t) => format!("{} — {} | Retrograde Mini", t.title, t.artist),
                None => "Retrograde Mini".to_owned(),
            };
        }
        if let Some(track) = &self.current_track {
            format!("{} — {} | Retrograde", track.title, track.artist)
        } else {
            "Retrograde".to_owned()
        }
    }

    fn theme(&self, _window: window::Id) -> Theme {
        self.orbit_theme.clone()
    }

    pub fn wgsl_shader_enabled(&self) -> bool {
        shader::can_use_wgsl_shader(
            &self.render_tier,
            &self.graphics_report,
            self.settings.gpu_preference,
        )
    }

    fn subscription(&self) -> Subscription<Message> {
        let controller_sub = Subscription::run(controller_event_stream).map(Message::Controller);
        let close_req_sub = window::close_requests().map(Message::WindowCloseRequested);
        let close_evt_sub = window::close_events().map(Message::WindowClosed);
        let events_sub = event::listen_with(map_runtime_event);
        let present_sub = event::listen_raw(map_present_event);

        let mut subs = vec![controller_sub, close_req_sub, close_evt_sub, events_sub, present_sub];
        let mini_needs_tick = self.mini_window.is_some()
            && (self.mini_hovered
                || (self.is_playing
                    && self.render_tier.enable_animations
                    && self.current_track.as_ref().is_none_or(|t| {
                        t.cover_thumb_path.is_none() && t.cover_full_path.is_none()
                    })));
        let egg_target = if self.egg_hovered { 1.0 } else { 0.0 };
        let egg_needs_tick = self.egg_open
            && self.window_visible
            && !self.main_window_minimized
            && (self.render_tier.enable_animations
                || (self.egg_color_alpha - egg_target).abs() > 0.001);
        if self.fs_running()
            || self.sidebar_anim.is_some()
            || (self.toasts.animating() && self.main_window.is_some())
        {
            subs.push(iced::time::every(Duration::from_millis(16)).map(Message::AnimTick));
        } else if mini_needs_tick || egg_needs_tick {
            subs.push(iced::time::every(Duration::from_millis(33)).map(Message::AnimTick));
        } else if (self.scan.is_some() || self.deleting_albums) && self.window_visible && !self.main_window_minimized {
            // The scan or delete card's spinner
            subs.push(iced::time::every(Duration::from_millis(50)).map(Message::AnimTick));
        } else if self.is_playing
            && self.window_visible
            && !self.main_window_minimized
            && self.main_window_focused
            && self.settings.sidebar_visible
            && self.sidebar_width >= 150.0
            && self.render_tier.enable_animations
        {
            // The sidebar's level meter moves at ~12 fps while playing, focused and visible.
            subs.push(iced::time::every(Duration::from_millis(83)).map(Message::AnimTick));
        }
        if let Some(ms) = shader::animation_interval_ms(
            &self.render_tier,
            self.wgsl_shader_enabled(),
            self.accent_anim.is_animating(),
            false,
            self.main_window_focused,
        ) {
            subs.push(iced::time::every(Duration::from_millis(ms)).map(Message::AnimTick));
        }
        Subscription::batch(subs)
    }

    fn update(&mut self, msg: Message) -> Task<Message> {
        let fs_was_open = self.active_tab == Tab::NowPlaying;
        // A flyout closes on whatever the user does next (its own toggle and background messages aside)
        let background = matches!(
            msg,
            Message::ToggleSortMenu
                | Message::ToggleBankMenu
                | Message::ToggleAddMusicMenu
                | Message::AnimTick(_)
                | Message::Controller(_)
                | Message::ToastExpire(_)
                | Message::ToastRemove(_)
                | Message::WindowResized(..)
                | Message::WindowFocused(..)
                | Message::WindowMoved(..)
                | Message::MainWindowMinimized(..)
                | Message::WindowPresented(..)
                | Message::SearchDebounced(_)
                | Message::DetailTintReady(..)
                | Message::FsBlurReady(..)
                | Message::ShotGo(_)
                | Message::ShotTaken(..)
                | Message::MiniHover(_)
                | Message::VisualizerBarRelease(..)
                | Message::EggMorseTimeout(_)
                | Message::EggAssetsReady(_)
                | Message::EggImageHover(_)
                | Message::LibScrolled(_)
                | Message::RestoreLibScroll
                | Message::DetailScrolled(_)
                | Message::BankScrolled(..)
                | Message::KeyShortcut { .. }
                | Message::FocusCheckedShortcut { .. }
        );
        if !background {
            self.sort_menu_open = false;
            self.bank_menu_open = false;
            self.add_music_menu_open = false;
        }
        let task = self.update_inner(msg);
        self.fs_sync(fs_was_open);
        let backdrop = self.fs_refresh_backdrop();
        self.sync_playback_tick_mode();
        Task::batch([task, backdrop, self.fs_lyric_scroll()])
    }

    fn is_unfocused_open_main(&self) -> bool {
        self.mini_window.is_none()
            && self.window_visible
            && self.main_window.is_some()
            && !self.main_window_minimized
            && !self.main_window_focused
    }

    fn sync_playback_tick_mode(&mut self) {
        if !self.is_unfocused_open_main() {
            self.watchdog.reset();
            WAKE_ON_PRESENT.store(false, Ordering::SeqCst);
        }
        let next = resolve_playback_tick_mode(
            self.mini_window.is_some(),
            self.window_visible,
            self.main_window.is_some(),
            self.main_window_minimized,
            self.main_window_focused,
            self.watchdog.is_suspended(),
        );
        if next != self.playback_tick_mode {
            tracing::debug!(
                prev = ?self.playback_tick_mode,
                ?next,
                suspended = self.watchdog.is_suspended(),
                "playback tick mode changed"
            );
            self.playback_tick_mode = next;
            let _ = self.cmd_tx.send(ControllerCommand::SetPlaybackTickMode(next));
        }
    }

    /// Keeps the current lyric line in view in the full-screen panel.
    fn fs_lyric_scroll(&mut self) -> Task<Message> {
        if !(self.fs_lyrics_tab && self.active_tab == Tab::NowPlaying) {
            return Task::none();
        }
        let Some(doc) = &self.lyrics else { return Task::none() };
        if !doc.is_synced || doc.lines.len() < 2 {
            return Task::none();
        }
        let idx = doc.index_at(self.position_s);
        if idx == self.fs_last_lyric {
            return Task::none();
        }
        self.fs_last_lyric = idx;
        let y = idx.map_or(0.0, |i| (i as f32 / (doc.lines.len() - 1) as f32).clamp(0.0, 1.0));
        operation::snap_to(
            self.lyrics_scroll_id.clone(),
            iced::widget::scrollable::RelativeOffset { x: Some(0.0), y: Some(y) },
        )
    }

    fn update_inner(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::Noop => Task::none(),
            Message::WindowOpened(id) => {
                if self.main_window != Some(id) {
                    // Stale open from a fast mini/main toggle: the app no longer expects it.
                    return window::close(id);
                }
                self.window_visible = true;
                self.main_window_focused = true;
                self.main_window_minimized = false;
                self.shot_begin()
            }
            Message::MiniWindowOpened(id) => {
                if self.mini_window != Some(id) {
                    return window::close(id);
                }
                Task::none()
            }
            Message::WindowCloseRequested(id) => {
                if self.mini_window == Some(id) {
                    if self.settings.minimize_to_tray == Some(true) && self.tray_available {
                        self.mini_window = None;
                        self.window_visible = false;
                        self.push_settings();
                        window::close(id)
                    } else {
                        // WM close on the mini-player means "back to the full window", not quit.
                        self.restore_main_from_mini()
                    }
                } else {
                    self.request_close_main()
                }
            }
            Message::BackgroundRemember(on) => {
                self.bg_remember = on;
                Task::none()
            }
            Message::BackgroundChoice(minimize) => {
                self.bg_prompt = false;
                if self.bg_remember {
                    self.settings.minimize_to_tray = Some(minimize);
                    self.push_settings();
                }
                if minimize && self.tray_available {
                    self.hide_main_window()
                } else {
                    self.begin_shutdown()
                }
            }
            Message::ToastExpire(id) => self.toasts.expire(id),
            Message::ToastRemove(id) => {
                self.toasts.remove(id);
                Task::none()
            }
            Message::ConfirmCancel => {
                self.confirm = None;
                Task::none()
            }
            Message::ConfirmAccept => self.accept_confirm(),
            Message::ToggleSortMenu => {
                self.sort_menu_open = !self.sort_menu_open;
                self.bank_menu_open = false;
                self.add_music_menu_open = false;
                Task::none()
            }
            Message::ToggleBankMenu => {
                self.bank_menu_open = !self.bank_menu_open;
                self.sort_menu_open = false;
                self.add_music_menu_open = false;
                Task::none()
            }
            Message::ToggleAddMusicMenu => {
                self.add_music_menu_open = !self.add_music_menu_open;
                self.sort_menu_open = false;
                self.bank_menu_open = false;
                Task::none()
            }
            Message::WindowResized(id, size) => {
                if self.main_window == Some(id) {
                    self.window_size = size;
                    self.bank_viewport_h = None;
                }
                Task::none()
            }
            Message::DetailTintReady(key, color) => {
                if self.tint_for.as_deref() == Some(key.as_str()) {
                    self.detail_tint = color;
                }
                Task::none()
            }
            Message::FsBlurReady(id, handle) => {
                if self.current_track.as_ref().is_some_and(|t| t.id == id) {
                    self.fs_blur = handle.map(|h| (id, h));
                }
                Task::none()
            }
            Message::OpenCurrentArtist => {
                let Some(t) = &self.current_track else { return Task::none() };
                // The artist of the playing track's album, else the track's own artist
                let album_artist = self.track_map.get(&t.id).and_then(|&(a, _)| self.albums.get(a)).map(|a| a.artist.clone());
                let key = album_artist
                    .map(|a| artist_key(&a))
                    .filter(|k| self.find_artist(k).is_some())
                    .unwrap_or_else(|| artist_key(&t.artist));
                if self.find_artist(&key).is_some() {
                    Task::done(Message::OpenArtist(key))
                } else {
                    let text = format!("ARTIST_NOT_IN_LIBRARY // {}", t.artist);
                    self.toasts.push(NoticeKind::Info, text)
                }
            }
            Message::Back => self.go_back(),
            Message::SearchDebounced(generation) => {
                if generation == self.search_gen {
                    self.rebuild_search();
                }
                Task::none()
            }
            Message::WindowClosed(id) => {
                if self.main_window == Some(id) {
                    self.main_window = None;
                    self.window_visible = false;
                    self.hovered_files = 0;
                }
                if self.mini_window == Some(id) {
                    self.mini_window = None;
                    self.push_settings();
                }
                Task::none()
            }
            Message::WindowFocused(id, focused) => {
                if self.main_window == Some(id) {
                    self.main_window_focused = focused;
                    if focused {
                        self.main_window_minimized = false;
                    } else {
                        self.watchdog
                            .on_unfocus(PRESENT_SEQ.load(Ordering::SeqCst), Instant::now());
                        return window::is_minimized(id)
                            .map(move |m| Message::MainWindowMinimized(id, m.unwrap_or(false)));
                    }
                }
                Task::none()
            }
            Message::WindowPresented(id) => {
                if self.main_window == Some(id) {
                    tracing::debug!(?id, "main window presented after compositor suspend");
                    self.watchdog.on_presented(PRESENT_SEQ.load(Ordering::SeqCst));
                } else if self.watchdog.is_suspended() {
                    // Another (e.g. closing mini) window consumed WAKE_ON_PRESENT; re-arm with the
                    // Dekker check so the main window's next RedrawRequested still wakes us.
                    let seq = PRESENT_SEQ.load(Ordering::SeqCst);
                    WAKE_ON_PRESENT.store(true, Ordering::SeqCst);
                    let after = PRESENT_SEQ.load(Ordering::SeqCst);
                    if after != seq {
                        WAKE_ON_PRESENT.store(false, Ordering::SeqCst);
                        self.watchdog.on_presented(after);
                    }
                }
                Task::none()
            }
            Message::MainWindowMinimized(id, minimized) => {
                if self.main_window == Some(id) && !self.main_window_focused {
                    self.main_window_minimized = minimized;
                    if minimized {
                        self.last_frame_at = None;
                    }
                }
                Task::none()
            }
            Message::WindowMoved(id, pt) => {
                if self.mini_window == Some(id) {
                    let x = Some(pt.x.round() as i32);
                    let y = Some(pt.y.round() as i32);
                    if self.settings.mini_x != x || self.settings.mini_y != y {
                        self.settings.mini_x = x;
                        self.settings.mini_y = y;
                        self.mini_pos_dirty = true;
                    }
                }
                Task::none()
            }
            Message::FileHovered => {
                self.hovered_files = self.hovered_files.saturating_add(1);
                Task::none()
            }
            Message::FilesHoveredLeft => {
                self.hovered_files = 0;
                Task::none()
            }
            Message::FileDropped(path) => {
                self.hovered_files = 0;
                let first = self.dropped_files.is_empty();
                self.dropped_files.push(path);
                if first {
                    Task::perform(tokio::time::sleep(Duration::from_millis(150)), |_| {
                        Message::FlushDrops
                    })
                } else {
                    Task::none()
                }
            }
            Message::FocusSearch => {
                self.page_stack.clear();
                if self.active_tab != Tab::Search && self.active_tab != Tab::NowPlaying {
                    self.previous_tab = self.active_tab;
                }
                self.active_tab = Tab::Search;
                operation::focus(self.search_input_id.clone())
            }
            Message::OpenPlaylistPage(view) => {
                self.enter_page(&pages::Page::Playlist(view));
                self.open_playlist(view)
            }
            Message::SetBankPlaylists(on) => {
                if self.bank_playlists != on {
                    self.bank_playlists = on;
                    self.bank_scroll_y = 0.0;
                    self.bank_viewport_h = None;
                    return operation::scroll_to(
                        self.bank_scroll_id.clone(),
                        iced::widget::scrollable::AbsoluteOffset { x: 0.0, y: 0.0 },
                    );
                }
                Task::none()
            }
            Message::ShotStep(i) => self.shot_step(i),
            Message::ShotGo(i) => self.shot_go(i),
            Message::ShotTaken(i, s) => self.shot_taken(i, s),
            Message::SetFsLyricsTab(on) => {
                self.fs_lyrics_tab = on;
                self.fs_last_lyric = None;
                Task::none()
            }
            Message::GoToCurrentAlbum => {
                let Some(t) = &self.current_track else { return Task::none() };
                let key = AlbumKey { name: t.album.clone(), artist: t.artist.clone() };
                let key = self
                    .albums
                    .iter()
                    .find(|a| a.tracks.iter().any(|x| x.id == t.id))
                    .map(AlbumKey::from_album)
                    .unwrap_or(key);
                self.previous_tab = Tab::Library;
                self.active_tab = Tab::Library;
                Task::done(Message::SelectAlbum(key))
            }
            Message::ToggleQueuePanel => {
                self.queue_open = !self.queue_open;
                self.audio_open = false;
                Task::none()
            }
            Message::ToggleAudioPanel => {
                self.audio_open = !self.audio_open;
                self.queue_open = false;
                if self.audio_open {
                    // As the original: the device list is read again whenever the popup opens
                    let _ = self.cmd_tx.send(ControllerCommand::RefreshDevices);
                }
                Task::none()
            }
            Message::SetAudioOutputTab(on) => {
                self.audio_output_tab = on;
                Task::none()
            }
            Message::OpenArtist(key) => {
                self.enter_page(&pages::Page::Artist(key.clone()));
                self.open_artist(key)
            }
            Message::CloseArtistDetail => self.go_back(),
            Message::SetSearchTab(t) => {
                self.search_tab = t;
                Task::none()
            }
            Message::PlayDetail(ctx, idx) => {
                let tracks = self.detail_tracks(&ctx);
                if !tracks.is_empty() {
                    let start_index = idx.min(tracks.len() - 1);
                    let _ = self.cmd_tx.send(ControllerCommand::PlayTrackList { tracks, start_index });
                }
                Task::none()
            }
            Message::QueueDetail(ctx) => {
                let tracks = self.detail_tracks(&ctx);
                if !tracks.is_empty() {
                    let _ = self.cmd_tx.send(ControllerCommand::EnqueueTracks(tracks));
                }
                Task::none()
            }
            Message::PlaySearchTrack(i) => {
                if i < self.search_tracks.len() {
                    let _ = self.cmd_tx.send(ControllerCommand::PlayTrackList { tracks: self.search_tracks.clone(), start_index: i });
                }
                Task::none()
            }
            Message::ToggleCreatePlaylist(on) => {
                self.create_playlist_open = on;
                self.bank_menu_open = false;
                if on {
                    self.new_playlist_name.clear();
                    return operation::focus(self.create_input_id.clone());
                }
                Task::none()
            }
            Message::GoHome => {
                self.active_tab = Tab::Library;
                self.lib_scroll_y = 0.0;
                self.select_mode = false;
                self.selected_albums.clear();
                self.album_detail_open = false;
                self.artist_detail_open = false;
                self.tag_draft = None;
                self.page_stack.clear();
                // As the original: home clears the search
                self.search_query.clear();
                self.search_gen += 1;
                self.rebuild_search();
                Task::none()
            }
            Message::FlushDrops => {
                let paths = std::mem::take(&mut self.dropped_files);
                if !paths.is_empty() {
                    let _ = self.cmd_tx.send(ControllerCommand::ImportPaths(paths));
                }
                Task::none()
            }
            Message::KeyShortcut {
                key,
                modifiers,
                ignored,
            } => self.handle_key_shortcut(key, modifiers, ignored),
            Message::FocusCheckedShortcut {
                key,
                modifiers,
                text_input_focused,
            } => {
                if text_input_focused
                    || self.tag_draft.is_some()
                    || self.smart_draft.is_some()
                    || self.shutting_down
                {
                    Task::none()
                } else {
                    self.dispatch_shortcut(key, modifiers)
                }
            }
            Message::AnimTick(now) => {
                let anim_phase = ((self.is_playing || self.deleting_albums)
                    && (self.window_visible || self.mini_window.is_some()))
                    || self.fs_anim.is_some()
                    || self.mini_hovered
                    || self.egg_open;
                if let Some(prev) = self.last_frame_at {
                    let dt = now.saturating_duration_since(prev).as_secs_f32().min(0.1);
                    if self.is_playing || self.deleting_albums {
                        self.shader_phase += (dt as f64) * 0.7;
                    }
                    if self.mini_window.is_some() && self.mini_hovered {
                        self.mini_marquee_phase += dt;
                    }
                    if self.egg_open {
                        self.egg_pulse_phase = (self.egg_pulse_phase + dt * 2.2) % std::f32::consts::TAU;
                        let target = if self.egg_hovered { 1.0 } else { 0.0 };
                        if self.render_tier.enable_animations {
                            if self.egg_color_alpha < target {
                                self.egg_color_alpha = (self.egg_color_alpha + dt).min(1.0);
                            } else if self.egg_color_alpha > target {
                                self.egg_color_alpha = (self.egg_color_alpha - dt).max(0.0);
                            }
                        } else {
                            self.egg_color_alpha = target;
                        }
                    }
                }
                self.last_frame_at = anim_phase.then_some(now);
                if let Some((start, from, to)) = self.sidebar_anim {
                    let t = (now.saturating_duration_since(start).as_secs_f32() / 0.2).clamp(0.0, 1.0);
                    let ease = if t < 0.5 {
                        4.0 * t * t * t
                    } else {
                        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                    };
                    self.sidebar_width = from + (to - from) * ease;
                    if t >= 1.0 {
                        self.sidebar_width = to;
                        self.sidebar_anim = None;
                    }
                }
                if self.is_playing
                    && self.window_visible
                    && !self.main_window_minimized
                    && self.main_window_focused
                    && self.settings.sidebar_visible
                    && self.sidebar_width >= 150.0
                    && self.render_tier.enable_animations
                {
                    self.visualizer_cache.clear();
                }
                if self.fs_anim.is_some() {
                    self.fs_tick(now);
                }
                if self.accent_anim.is_animating() {
                    let _ = self.accent_anim.tick();
                    self.orbit_theme = theme::orbit_theme(self.accent_anim.current_accent);
                }
                Task::none()
            }
            Message::ToggleMiniPlayer => self.toggle_mini_player(),
            Message::DragMiniWindow => {
                if let Some(mid) = self.mini_window {
                    window::drag(mid)
                } else {
                    Task::none()
                }
            }
            Message::CloseMiniPlayerToMain => self.restore_main_from_mini(),
            Message::MiniHover(hovered) => {
                self.mini_hovered = hovered;
                if !hovered {
                    self.mini_marquee_phase = 0.0;
                }
                Task::none()
            }
            Message::DragMainWindow => {
                if let Some(id) = self.main_window {
                    let now = Instant::now();
                    if self
                        .last_titlebar_press
                        .is_some_and(|t| now.duration_since(t) <= Duration::from_millis(350))
                    {
                        self.last_titlebar_press = None;
                        return window::toggle_maximize(id);
                    }
                    self.last_titlebar_press = Some(now);
                    return window::drag(id);
                }
                Task::none()
            }
            Message::MinimizeMainWindow => {
                if let Some(id) = self.main_window {
                    window::minimize(id, true)
                } else {
                    Task::none()
                }
            }
            Message::ToggleMaximizeMainWindow => {
                if let Some(id) = self.main_window {
                    window::toggle_maximize(id)
                } else {
                    Task::none()
                }
            }
            Message::RequestCloseMainWindow => self.request_close_main(),
            Message::ResizeMainWindow(dir) => {
                if let Some(id) = self.main_window {
                    window::drag_resize(id, dir)
                } else {
                    Task::none()
                }
            }
            Message::LibScrolled(_) if self.lib_restore.is_some() => Task::none(),
            Message::LibScrolled(y) => {
                self.lib_scroll_raw = y.max(0.0);
                let q = (y.max(0.0) / 48.0).floor() * 48.0;
                if (self.lib_scroll_y - q).abs() >= 1.0 {
                    self.lib_scroll_y = q;
                }
                Task::none()
            }
            Message::DetailScrolled(y) => {
                let q = (y.max(0.0) / 32.0).floor() * 32.0;
                if (self.detail_scroll_y - q).abs() >= 1.0 {
                    self.detail_scroll_y = q;
                }
                Task::none()
            }
            Message::BankScrolled(y, viewport_h) => {
                let q = (y.max(0.0) / 32.0).floor() * 32.0;
                if (self.bank_scroll_y - q).abs() >= 1.0 {
                    self.bank_scroll_y = q;
                }
                if viewport_h > 0.0 {
                    self.bank_viewport_h = Some(viewport_h);
                }
                Task::none()
            }
            Message::RestoreLibScroll => {
                let Some(y) = self.lib_restore.take() else { return Task::none() };
                self.lib_scroll_raw = y;
                self.lib_scroll_y = (y / 48.0).floor() * 48.0;
                operation::scroll_to(
                    self.lib_scroll_id.clone(),
                    iced::widget::scrollable::AbsoluteOffset { x: 0.0, y },
                )
            }
            Message::ToggleShortcutsModal => {
                self.show_shortcuts_modal = !self.show_shortcuts_modal;
                Task::none()
            }
            Message::ToggleSidebar => {
                self.toggle_sidebar();
                Task::none()
            }
            Message::CloseAlbumDetail => self.go_back(),
            Message::ForceExit => iced::exit(),
            Message::Controller(evt) => self.handle_controller_event(evt),
            Message::SelectTab(tab) => {
                if self.active_tab != Tab::NowPlaying && tab != Tab::NowPlaying && tab != self.active_tab {
                    self.page_stack.clear();
                }
                if self.active_tab != tab {
                    if self.active_tab != Tab::NowPlaying {
                        self.previous_tab = self.active_tab;
                    }
                    self.active_tab = tab;
                    if tab != Tab::Library {
                        self.select_mode = false;
                        self.selected_albums.clear();
                    }
                }
                self.tag_draft = None;
                self.show_shortcuts_modal = false;
                Task::none()
            }
            Message::SetLibraryMode(mode) => {
                self.library_mode = mode;
                self.lib_scroll_y = 0.0;
                self.select_mode = false;
                self.selected_albums.clear();
                self.album_detail_open = false;
                self.artist_detail_open = false;
                self.settings.library_view = match mode {
                    LibraryMode::Albums => "albums".to_owned(),
                    LibraryMode::Artists => "artists".to_owned(),
                };
                self.push_settings();
                Task::none()
            }
            Message::SearchChanged(q) => {
                // The field follows every key; the results wait for a 150 ms pause (as the original)
                self.search_query = q;
                self.search_gen += 1;
                let generation = self.search_gen;
                Task::perform(tokio::time::sleep(Duration::from_millis(150)), move |_| {
                    Message::SearchDebounced(generation)
                })
            }
            Message::SelectLibrarySort(sort) => {
                self.sort_menu_open = false;
                self.lib_scroll_y = 0.0;
                self.library_sort = sort;
                self.settings.library_sort = sort.as_str().to_owned();
                self.rebuild_library_view();
                self.push_settings();
                Task::none()
            }
            Message::SelectAlbum(key) => {
                self.enter_page(&pages::Page::Album(key.clone()));
                self.open_album(key)
            }
            Message::SelectArtist(artist_key) => {
                self.selected_artist = Some(artist_key);
                Task::none()
            }
            Message::PlayAlbumTrack(key, tidx) => {
                if let Some(album) = self.find_album(&key)
                    && !album.tracks.is_empty()
                {
                    let _ = self.cmd_tx.send(ControllerCommand::PlayTrackList {
                        tracks: album.tracks.clone(),
                        start_index: tidx,
                    });
                }
                Task::none()
            }
            Message::QueueAlbum(key) => {
                if let Some(album) = self.find_album(&key) {
                    let _ = self
                        .cmd_tx
                        .send(ControllerCommand::EnqueueTracks(album.tracks.clone()));
                }
                Task::none()
            }
            Message::PlayTrackFromList(tracks, idx) => {
                let _ = self.cmd_tx.send(ControllerCommand::PlayTrackList {
                    tracks,
                    start_index: idx,
                });
                Task::none()
            }
            Message::PlayNext(track) => {
                let _ = self.cmd_tx.send(ControllerCommand::PlayNext(track));
                Task::none()
            }
            Message::EnqueueTracks(tracks) => {
                let _ = self.cmd_tx.send(ControllerCommand::EnqueueTracks(tracks));
                Task::none()
            }
            Message::JumpToQueueIndex(idx) => {
                let _ = self.cmd_tx.send(ControllerCommand::JumpToQueueIndex(idx));
                Task::none()
            }
            Message::RemoveQueueIndex(idx) => {
                let _ = self.cmd_tx.send(ControllerCommand::RemoveQueueIndex(idx));
                Task::none()
            }
            Message::ClearQueue => {
                let _ = self.cmd_tx.send(ControllerCommand::ClearQueue);
                Task::none()
            }
            Message::TogglePlayPause => {
                let _ = self.cmd_tx.send(ControllerCommand::TogglePlayPause);
                Task::none()
            }
            Message::Previous => {
                let _ = self.cmd_tx.send(ControllerCommand::Previous);
                Task::none()
            }
            Message::Next => {
                let _ = self.cmd_tx.send(ControllerCommand::Next);
                Task::none()
            }
            Message::ToggleShuffle => {
                let _ = self.cmd_tx.send(ControllerCommand::ToggleShuffle);
                Task::none()
            }
            Message::CycleRepeat => {
                let _ = self.cmd_tx.send(ControllerCommand::CycleRepeat);
                Task::none()
            }
            Message::SeekChanged(sec) => {
                self.seeking_pos = None;
                self.position_s = sec;
                let _ = self.cmd_tx.send(ControllerCommand::SeekSeconds(sec));
                Task::none()
            }
            Message::SeekDragging(sec) => {
                self.seeking_pos = Some(sec);
                Task::none()
            }
            Message::SeekCommit => {
                if let Some(sec) = self.seeking_pos.take() {
                    self.position_s = sec;
                    let _ = self.cmd_tx.send(ControllerCommand::SeekSeconds(sec));
                }
                Task::none()
            }
            Message::VolumeDragging(v) => {
                self.dragging_volume = Some(v);
                self.volume = v;
                let should_send = self
                    .last_volume_emit
                    .is_none_or(|t| t.elapsed() >= Duration::from_millis(40));
                if should_send {
                    self.last_volume_emit = Some(Instant::now());
                    let _ = self.cmd_tx.send(ControllerCommand::SetVolume(v));
                }
                Task::none()
            }
            Message::VolumeCommit => {
                if let Some(v) = self.dragging_volume.take() {
                    self.last_volume_emit = None;
                    let _ = self.cmd_tx.send(ControllerCommand::SetVolume(v));
                }
                Task::none()
            }
            Message::ToggleMute => {
                let _ = self.cmd_tx.send(ControllerCommand::ToggleMute);
                Task::none()
            }
            Message::SelectDevice(opt) => {
                self.audio_device = opt.id.clone();
                let _ = self.cmd_tx.send(ControllerCommand::SelectAudioDevice(opt.id));
                Task::none()
            }
            Message::RefreshDevices => {
                let _ = self.cmd_tx.send(ControllerCommand::RefreshDevices);
                Task::none()
            }
            Message::ToggleLike(id) => {
                let _ = self.cmd_tx.send(ControllerCommand::ToggleLike(id));
                Task::none()
            }
            Message::SelectPlaylistView(view) => {
                self.playlist_view = view;
                self.smart_draft = None;
                if let PlaylistView::Custom(id) = view
                    && let Some(p) = self.collections.playlists.iter().find(|p| p.id == id)
                {
                    self.rename_playlist_buf = p.name.clone();
                }
                self.rebuild_smart_cache();
                Task::none()
            }
            Message::NewPlaylistNameChanged(s) => {
                self.new_playlist_name = s;
                Task::none()
            }
            Message::CreatePlaylist => {
                self.create_playlist_open = false;
                let name = std::mem::take(&mut self.new_playlist_name);
                if !name.trim().is_empty() {
                    let _ = self.cmd_tx.send(ControllerCommand::CreatePlaylist(name));
                }
                Task::none()
            }
            Message::DeletePlaylist(id) => {
                if let Some(p) = self.collections.playlists.iter().find(|p| p.id == id) {
                    self.confirm = Some(dialogs::ConfirmDialog {
                        title: "WARNING: DELETE_PLAYLIST",
                        target_label: "TARGET: ",
                        target: p.name.clone(),
                        target_orange: false,
                        detail: None,
                        note: "ACTION CANNOT BE UNDONE.",
                        confirm: "CONFIRM",
                        action: dialogs::ConfirmAction::DeletePlaylist { id, cover_art: p.cover_art.clone() },
                    });
                }
                Task::none()
            }
            Message::ToggleLikeTracks(track_ids) => {
                let _ = self.cmd_tx.send(ControllerCommand::ToggleLikeTracks(track_ids));
                Task::none()
            }
            Message::EnterSelectMode => {
                self.select_mode = true;
                self.selected_albums.clear();
                self.album_detail_open = false;
                self.artist_detail_open = false;
                Task::none()
            }
            Message::ExitSelectMode => {
                self.select_mode = false;
                self.selected_albums.clear();
                Task::none()
            }
            Message::ToggleAlbumSelected(key) => {
                if !self.selected_albums.remove(&key) {
                    self.selected_albums.insert(key);
                }
                Task::none()
            }
            Message::SelectAllAlbums => {
                self.selected_albums = self.albums.iter().map(AlbumKey::from_album).collect();
                Task::none()
            }
            Message::DeleteSelectedAlbums => {
                let names: Vec<String> = self.selected_albums.iter().map(|k| k.name.clone()).collect();
                if names.is_empty() {
                    return Task::none();
                }
                let n = names.len();
                self.confirm = Some(dialogs::ConfirmDialog {
                    title: "WARNING: BATCH_DELETE",
                    target_label: "TARGETS: ",
                    target: format!("{n} ALBUM(S)"),
                    target_orange: true,
                    detail: None,
                    note: "DATA WILL BE PERMANENTLY ERASED.",
                    confirm: "PURGE",
                    action: dialogs::ConfirmAction::DeleteAlbums(names),
                });
                Task::none()
            }
            Message::PickFiles => {
                self.add_music_menu_open = false;
                Task::perform(pick_music_files(), Message::FilesPicked)
            }
            Message::FilesPicked(Ok(files)) => {
                if !files.is_empty() {
                    let _ = self.cmd_tx.send(ControllerCommand::ScanFiles(files));
                }
                Task::none()
            }
            Message::FilesPicked(Err(err)) => self.toasts.push(NoticeKind::Error, err),
            Message::PickPlaylistCover(id) => Task::perform(pick_cover_image(), move |res| {
                Message::PlaylistCoverPicked(id, res)
            }),
            Message::PlaylistCoverPicked(id, Ok(Some(image_path))) => {
                let _ = self.cmd_tx.send(ControllerCommand::SetPlaylistCover {
                    playlist_id: id,
                    image_path,
                });
                Task::none()
            }
            Message::PlaylistCoverPicked(_, Ok(None)) => Task::none(),
            Message::PlaylistCoverPicked(_, Err(err)) => self.toasts.push(NoticeKind::Error, err),
            Message::OpenTrackMenu {
                track,
                album_artist,
                album_year,
                removable_playlist,
                pos,
            } => {
                self.context_menu = Some(context_menu::ContextMenu {
                    pos,
                    kind: context_menu::ContextMenuKind::Track {
                        track: Box::new(track),
                        album_artist,
                        album_year,
                        removable_playlist,
                    },
                });
                Task::none()
            }
            Message::OpenAlbumMenu(key, pos) => {
                self.context_menu = Some(context_menu::ContextMenu {
                    pos,
                    kind: context_menu::ContextMenuKind::Album(key),
                });
                Task::none()
            }
            Message::OpenPlaylistMenu(id, pos) => {
                self.context_menu = Some(context_menu::ContextMenu {
                    pos,
                    kind: context_menu::ContextMenuKind::Playlist(id),
                });
                Task::none()
            }
            Message::OpenDetailMenu(ctx, pos) => {
                self.context_menu = Some(context_menu::ContextMenu {
                    pos,
                    kind: context_menu::ContextMenuKind::DetailMore(ctx),
                });
                Task::none()
            }
            Message::CloseContextMenu => {
                self.context_menu = None;
                Task::none()
            }
            Message::ContextToggleLike(id) => {
                self.context_menu = None;
                let _ = self.cmd_tx.send(ControllerCommand::ToggleLike(id));
                Task::none()
            }
            Message::ContextToggleLikeTracks(ids) => {
                self.context_menu = None;
                let _ = self.cmd_tx.send(ControllerCommand::ToggleLikeTracks(ids));
                Task::none()
            }
            Message::ContextPlayNext(tracks) => {
                self.context_menu = None;
                if tracks.len() == 1 {
                    if let Some(t) = tracks.into_iter().next() {
                        let _ = self.cmd_tx.send(ControllerCommand::PlayNext(t));
                    }
                } else {
                    let _ = self.cmd_tx.send(ControllerCommand::PlayNextTracks(tracks));
                }
                Task::none()
            }
            Message::ContextEnqueue(tracks) => {
                self.context_menu = None;
                let _ = self.cmd_tx.send(ControllerCommand::EnqueueTracks(tracks));
                Task::none()
            }
            Message::ContextOpenTrackTags {
                track,
                album_artist,
                album_year,
            } => {
                self.context_menu = None;
                Task::done(Message::OpenTagEditor(track, album_artist, album_year))
            }
            Message::ContextOpenAlbumTags(key) => {
                self.context_menu = None;
                Task::done(Message::OpenAlbumTagEditor(key))
            }
            Message::ContextTogglePlaylistTracks {
                playlist_id,
                track_ids,
            } => {
                self.context_menu = None;
                let _ = self.cmd_tx.send(ControllerCommand::TogglePlaylistTracks {
                    playlist_id,
                    track_ids,
                });
                Task::none()
            }
            Message::ContextRemoveFromPlaylist {
                playlist_id,
                track_id,
            } => {
                self.context_menu = None;
                let _ = self.cmd_tx.send(ControllerCommand::RemoveTrackFromPlaylist {
                    playlist_id,
                    track_id,
                });
                Task::none()
            }
            Message::ContextRemovePlaylistCover(id) => {
                self.context_menu = None;
                let _ = self.cmd_tx.send(ControllerCommand::RemovePlaylistCover(id));
                Task::none()
            }
            Message::ContextDeletePlaylist(id) => {
                self.context_menu = None;
                Task::done(Message::DeletePlaylist(id))
            }
            Message::ContextDeleteSmart(id) => {
                self.context_menu = None;
                Task::done(Message::DeleteSmartPlaylist(id))
            }
            Message::ContextEditSmart(sp) => {
                self.context_menu = None;
                Task::done(Message::EditSmartPlaylist(sp))
            }
            Message::ContextDeleteAlbumFromMenu(key) => {
                self.context_menu = None;
                self.confirm = Some(dialogs::ConfirmDialog {
                    title: "WARNING: BATCH_DELETE",
                    target_label: "TARGETS: ",
                    target: "1 ALBUM".to_owned(),
                    target_orange: true,
                    detail: Some(key.name.clone()),
                    note: "DATA WILL BE PERMANENTLY ERASED.",
                    confirm: "PURGE",
                    action: dialogs::ConfirmAction::DeleteAlbums(vec![key.name]),
                });
                Task::none()
            }
            Message::ContextDeleteAlbumFromDetail(key) => {
                self.context_menu = None;
                self.confirm = Some(dialogs::ConfirmDialog {
                    title: "WARNING: PURGE_DATA",
                    target_label: "TARGET: ",
                    target: key.name.clone(),
                    target_orange: false,
                    detail: None,
                    note: "DATA WILL BE PERMANENTLY ERASED.",
                    confirm: "PURGE",
                    action: dialogs::ConfirmAction::DeleteAlbums(vec![key.name]),
                });
                Task::none()
            }
            Message::RenamePlaylistInput(s) => {
                self.rename_playlist_buf = s;
                Task::none()
            }
            Message::RenamePlaylistSubmit(id) => {
                let name = self.rename_playlist_buf.clone();
                let _ = self
                    .cmd_tx
                    .send(ControllerCommand::RenamePlaylist { id, name });
                Task::none()
            }
            Message::AddToTargetPlaylist(track_id) => {
                if let Some(pid) = self.target_playlist_id() {
                    let _ = self.cmd_tx.send(ControllerCommand::AddTracksToPlaylist {
                        playlist_id: pid,
                        track_ids: vec![track_id],
                    });
                }
                Task::none()
            }
            Message::RemoveFromPlaylist {
                playlist_id,
                track_id,
            } => {
                let _ = self.cmd_tx.send(ControllerCommand::RemoveTrackFromPlaylist {
                    playlist_id,
                    track_id,
                });
                Task::none()
            }
            Message::NewSmartPlaylist => {
                self.bank_menu_open = false;
                self.tag_draft = None;
                self.smart_draft = Some(SmartDraft {
                    id: 0,
                    name: String::new(),
                    match_all: true,
                    rules: vec![SmartRule::default()],
                    sort: "album".to_owned(),
                    limit: String::new(),
                    extra: ExtraMap::new(),
                    match_count: 0,
                });
                self.recompute_smart_draft_count();
                Task::none()
            }
            Message::EditSmartPlaylist(sp) => {
                let rules = if sp.rules.is_empty() {
                    vec![SmartRule::default()]
                } else {
                    sp.rules
                };
                self.smart_draft = Some(SmartDraft {
                    id: sp.id,
                    name: sp.name,
                    match_all: sp.match_all,
                    rules,
                    sort: sp.sort,
                    limit: if sp.limit > 0 { sp.limit.to_string() } else { String::new() },
                    extra: sp.extra,
                    match_count: 0,
                });
                self.recompute_smart_draft_count();
                Task::none()
            }
            Message::SmartDraftNameChanged(s) => {
                if let Some(d) = &mut self.smart_draft {
                    d.name = s;
                }
                Task::none()
            }
            Message::SmartRuleFieldChanged(idx, f) => {
                if let Some(d) = &mut self.smart_draft
                    && let Some(rule) = d.rules.get_mut(idx)
                {
                    rule.field = f.key.to_owned();
                    let field_def = SmartPlaylists::field(f.key);
                    if !field_def.ops.contains(&rule.op.as_str()) {
                        rule.op = field_def.ops.first().copied().unwrap_or("is").to_owned();
                    }
                    self.recompute_smart_draft_count();
                }
                Task::none()
            }
            Message::SmartRuleOpChanged(idx, o) => {
                if let Some(d) = &mut self.smart_draft
                    && let Some(rule) = d.rules.get_mut(idx)
                {
                    rule.op = o.op.to_owned();
                    self.recompute_smart_draft_count();
                }
                Task::none()
            }
            Message::SmartRuleValueChanged(idx, v) => {
                if let Some(d) = &mut self.smart_draft
                    && let Some(rule) = d.rules.get_mut(idx)
                {
                    rule.value = v;
                    self.recompute_smart_draft_count();
                }
                Task::none()
            }
            Message::AddSmartRule => {
                if let Some(d) = &mut self.smart_draft {
                    d.rules.push(SmartRule::default());
                    self.recompute_smart_draft_count();
                }
                Task::none()
            }
            Message::RemoveSmartRule(idx) => {
                if let Some(d) = &mut self.smart_draft
                    && d.rules.len() > 1
                    && idx < d.rules.len()
                {
                    d.rules.remove(idx);
                    self.recompute_smart_draft_count();
                }
                Task::none()
            }
            Message::SmartDraftSortChanged(s) => {
                if let Some(d) = &mut self.smart_draft {
                    d.sort = s.key.to_owned();
                    self.recompute_smart_draft_count();
                }
                Task::none()
            }
            Message::SmartDraftLimitChanged(l) => {
                if let Some(d) = &mut self.smart_draft {
                    d.limit = l;
                    self.recompute_smart_draft_count();
                }
                Task::none()
            }
            Message::SmartDraftMatchAllChanged(b) => {
                if let Some(d) = &mut self.smart_draft {
                    d.match_all = b;
                    self.recompute_smart_draft_count();
                }
                Task::none()
            }
            Message::SaveSmartPlaylist => {
                if let Some(d) = self.smart_draft.take() {
                    let sp = SmartPlaylistData {
                        id: d.id,
                        name: if d.name.trim().is_empty() {
                            "Smart Playlist".to_owned()
                        } else {
                            d.name.trim().to_owned()
                        },
                        match_all: d.match_all,
                        rules: d.rules,
                        sort: d.sort,
                        limit: d.limit.trim().parse::<usize>().unwrap_or(0),
                        extra: d.extra,
                    };
                    let _ = self.cmd_tx.send(ControllerCommand::SaveSmartPlaylist(sp));
                }
                Task::none()
            }
            Message::CancelSmartPlaylist => {
                self.smart_draft = None;
                Task::none()
            }
            Message::DeleteSmartPlaylist(id) => {
                if let Some(sp) = self.collections.smart.iter().flatten().find(|s| s.id == id) {
                    self.confirm = Some(dialogs::ConfirmDialog {
                        title: "WARNING: DELETE_SMART_PLAYLIST",
                        target_label: "TARGET: ",
                        target: sp.name.clone(),
                        target_orange: false,
                        detail: None,
                        note: "THE TRACKS STAY IN THE LIBRARY.",
                        confirm: "CONFIRM",
                        action: dialogs::ConfirmAction::DeleteSmartPlaylist(id),
                    });
                }
                Task::none()
            }
            Message::SetEqEnabled(enabled) => {
                self.settings.eq_enabled = enabled;
                let _ = self.cmd_tx.send(ControllerCommand::SetEqEnabled(enabled));
                Task::none()
            }
            Message::SelectEqPreset(preset) => {
                let _ = self.cmd_tx.send(ControllerCommand::SelectEqPreset(preset));
                Task::none()
            }
            Message::SetEqBand { band, gain } => {
                if self.settings.eq_gains.len() < 10 {
                    self.settings.eq_gains.resize(10, 0.0);
                }
                if band < 10 {
                    self.settings.eq_gains[band] = gain;
                    self.settings.eq_preset = "CUSTOM".to_owned();
                }
                let _ = self
                    .cmd_tx
                    .send(ControllerCommand::SetEqBand { band, gain });
                Task::none()
            }
            Message::SelectReplayGain(opt) => {
                self.settings.replay_gain = opt.0.clone();
                let _ = self.cmd_tx.send(ControllerCommand::SetReplayGain(opt.0));
                Task::none()
            }
            Message::SetCrossfadeSeconds(secs) => {
                self.settings.crossfade_seconds = secs;
                let _ = self
                    .cmd_tx
                    .send(ControllerCommand::SetCrossfadeSeconds(secs));
                Task::none()
            }
            Message::OpenTagEditor(track, album_artist, year) => {
                let path = PathBuf::from(&track.file_path);
                let target_label = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("TRACK")
                    .to_owned();
                let cover_preview = track
                    .cover_full_path
                    .clone()
                    .or_else(|| track.cover_thumb_path.clone());
                if let Some(cp) = cover_preview.as_deref() {
                    self.covers.ensure_full(cp);
                }
                let year_s = if year > 0 { year.to_string() } else { String::new() };
                let track_s = if track.track_number > 0 {
                    track.track_number.to_string()
                } else {
                    String::new()
                };
                let disc_s = if track.disc_number > 0 {
                    track.disc_number.to_string()
                } else {
                    String::new()
                };
                let aa = if album_artist == "Unknown Artist" {
                    String::new()
                } else {
                    album_artist
                };
                self.tag_status = None;
                self.tag_draft = Some(TagDraft {
                    is_track: true,
                    paths: vec![path],
                    album_key: Some(track.album.clone()),
                    target_label,
                    orig_title: track.title.clone(),
                    orig_artist: track.artist.clone(),
                    orig_album: track.album.clone(),
                    orig_album_artist: aa.clone(),
                    orig_year: year_s.clone(),
                    orig_track: track_s.clone(),
                    orig_disc: disc_s.clone(),
                    title: track.title,
                    artist: track.artist,
                    album: track.album,
                    album_artist: aa,
                    year: year_s,
                    track: track_s,
                    disc: disc_s,
                    cover_preview,
                    new_cover: None,
                    remove_cover: false,
                    saving: false,
                });
                Task::none()
            }
            Message::OpenAlbumTagEditor(key) => {
                let Some(album) = self.find_album(&key).cloned() else {
                    return Task::none();
                };
                let paths: Vec<PathBuf> = album
                    .tracks
                    .iter()
                    .map(|t| PathBuf::from(&t.file_path))
                    .collect();
                if paths.is_empty() {
                    return Task::none();
                }
                let cover_preview = album
                    .cover_full_path
                    .clone()
                    .or_else(|| album.cover_thumb_path.clone());
                if let Some(cp) = cover_preview.as_deref() {
                    self.covers.ensure_full(cp);
                }
                let year_s = if album.year > 0 {
                    album.year.to_string()
                } else {
                    String::new()
                };
                let aa = if album.artist == "Unknown Artist" {
                    String::new()
                } else {
                    album.artist.clone()
                };
                self.tag_status = None;
                self.tag_draft = Some(TagDraft {
                    is_track: false,
                    target_label: format!("{} TRACKS", paths.len()),
                    paths,
                    album_key: Some(album.name.clone()),
                    orig_title: String::new(),
                    orig_artist: String::new(),
                    orig_album: album.name.clone(),
                    orig_album_artist: aa.clone(),
                    orig_year: year_s.clone(),
                    orig_track: String::new(),
                    orig_disc: String::new(),
                    title: String::new(),
                    artist: String::new(),
                    album: album.name,
                    album_artist: aa,
                    year: year_s,
                    track: String::new(),
                    disc: String::new(),
                    cover_preview,
                    new_cover: None,
                    remove_cover: false,
                    saving: false,
                });
                Task::none()
            }
            Message::CloseTagEditor => {
                if self.tag_draft.as_ref().is_some_and(|d| d.saving) {
                    return Task::none();
                }
                self.tag_draft = None;
                self.tag_status = None;
                Task::none()
            }
            Message::TagTitleChanged(s) => {
                if let Some(d) = &mut self.tag_draft {
                    d.title = s;
                }
                Task::none()
            }
            Message::TagArtistChanged(s) => {
                if let Some(d) = &mut self.tag_draft {
                    d.artist = s;
                }
                Task::none()
            }
            Message::TagAlbumChanged(s) => {
                if let Some(d) = &mut self.tag_draft {
                    d.album = s;
                }
                Task::none()
            }
            Message::TagAlbumArtistChanged(s) => {
                if let Some(d) = &mut self.tag_draft {
                    d.album_artist = s;
                }
                Task::none()
            }
            Message::TagYearChanged(s) => {
                if let Some(d) = &mut self.tag_draft {
                    d.year = s;
                }
                Task::none()
            }
            Message::TagTrackChanged(s) => {
                if let Some(d) = &mut self.tag_draft {
                    d.track = s;
                }
                Task::none()
            }
            Message::TagDiscChanged(s) => {
                if let Some(d) = &mut self.tag_draft {
                    d.disc = s;
                }
                Task::none()
            }
            Message::TagPickCover => {
                if self.tag_draft.as_ref().is_some_and(|d| d.saving) {
                    return Task::none();
                }
                Task::perform(pick_cover_image(), Message::TagCoverPicked)
            }
            Message::TagCoverPicked(Ok(Some(path))) => {
                if let Some(d) = &mut self.tag_draft {
                    d.new_cover = Some(path.clone());
                    d.remove_cover = false;
                    d.cover_preview = Some(path.clone());
                    self.covers.invalidate_paths(&[path]);
                }
                Task::none()
            }
            Message::TagCoverPicked(Ok(None)) => Task::none(),
            Message::TagCoverPicked(Err(err)) => {
                self.toasts.push(NoticeKind::Error, format!("COVER_PICK_FAILED // {err}"))
            }
            Message::TagRemoveCover => {
                if let Some(d) = &mut self.tag_draft
                    && !d.saving
                {
                    d.remove_cover = true;
                    d.new_cover = None;
                    d.cover_preview = None;
                }
                Task::none()
            }
            Message::SaveTrackTags => {
                if let Some(d) = &mut self.tag_draft {
                    if d.saving {
                        return Task::none();
                    }
                    if self.is_playing
                        && self.current_track.as_ref().is_some_and(|t| {
                            let cur = std::path::Path::new(&t.file_path);
                            d.paths.iter().any(|p| p == cur)
                        })
                    {
                        self.tag_status = Some(Err(
                            "Pause or switch tracks before editing a currently playing file."
                                .to_owned(),
                        ));
                        return Task::none();
                    }
                    let year = match parse_optional_u32(&d.year, &d.orig_year, "Year") {
                        Ok(v) => v,
                        Err(err) => {
                            self.tag_status = Some(Err(err));
                            return Task::none();
                        }
                    };
                    let (title, artist, track, disc) = if d.is_track {
                        let track = match parse_optional_u32(&d.track, &d.orig_track, "Track #") {
                            Ok(v) => v,
                            Err(err) => {
                                self.tag_status = Some(Err(err));
                                return Task::none();
                            }
                        };
                        let disc = match parse_optional_u32(&d.disc, &d.orig_disc, "Disc #") {
                            Ok(v) => v,
                            Err(err) => {
                                self.tag_status = Some(Err(err));
                                return Task::none();
                            }
                        };
                        (
                            (d.title != d.orig_title).then(|| d.title.clone()),
                            (d.artist != d.orig_artist).then(|| d.artist.clone()),
                            track,
                            disc,
                        )
                    } else {
                        (None, None, None, None)
                    };
                    let edit = TagEdit {
                        title,
                        artist,
                        album: (d.album != d.orig_album).then(|| d.album.clone()),
                        album_artist: (d.album_artist != d.orig_album_artist)
                            .then(|| d.album_artist.clone()),
                        year,
                        track,
                        disc,
                        cover_image: d.new_cover.clone(),
                        remove_cover: d.remove_cover,
                    };
                    if edit.title.is_none()
                        && edit.artist.is_none()
                        && edit.album.is_none()
                        && edit.album_artist.is_none()
                        && edit.year.is_none()
                        && edit.track.is_none()
                        && edit.disc.is_none()
                        && !edit.changes_cover()
                    {
                        self.tag_status = Some(Ok("No changes to save.".to_owned()));
                        return Task::none();
                    }
                    d.saving = true;
                    self.tag_status = None;
                    let _ = self.cmd_tx.send(ControllerCommand::SaveTags {
                        paths: d.paths.clone(),
                        album_key: d.album_key.clone(),
                        edit,
                    });
                }
                Task::none()
            }
            Message::PickFolders => {
                self.add_music_menu_open = false;
                Task::perform(pick_music_folders(), Message::FoldersPicked)
            }
            Message::FoldersPicked(Ok(folders)) => {
                if !folders.is_empty() {
                    let _ = self.cmd_tx.send(ControllerCommand::ScanFolders(folders));
                }
                Task::none()
            }
            Message::FoldersPicked(Err(err)) => self.toasts.push(NoticeKind::Error, err),
            Message::RemoveLibraryFolder(f) => {
                let _ = self
                    .cmd_tx
                    .send(ControllerCommand::RemoveLibraryFolder(f));
                Task::none()
            }
            Message::RescanLibrary => {
                let _ = self.cmd_tx.send(ControllerCommand::RescanLibrary);
                Task::none()
            }
            Message::SetGpuPreference(opt) => {
                self.settings.gpu_preference = opt.0;
                self.push_settings();
                Task::none()
            }
            Message::SetDisplayBackend(opt) => {
                self.settings.display_backend = opt.0;
                self.push_settings();
                Task::none()
            }
            Message::SetPerformanceMode(opt) => {
                self.settings.performance_mode = opt.0;
                self.push_settings();
                Task::none()
            }
            Message::SetDirectBridge(opt) => {
                self.settings.direct_volume_bridge = opt.0;
                self.push_settings();
                Task::none()
            }
            Message::ToggleTray(v) => {
                self.settings.enable_tray = v;
                self.push_settings();
                Task::none()
            }
            Message::ToggleMpris(v) => {
                self.settings.enable_mpris = v;
                self.push_settings();
                Task::none()
            }
            Message::ToggleDiscordRpc(v) => {
                self.settings.enable_discord_rpc = v;
                self.push_settings();
                Task::none()
            }
            Message::ToggleDiscordArtwork(v) => {
                self.settings.discord_fetch_artwork = v;
                self.push_settings();
                Task::none()
            }
            Message::DiscordClientIdChanged(s) => {
                self.discord_client_id_draft = s;
                Task::none()
            }
            Message::ApplyDiscordClientId => {
                let trimmed = self.discord_client_id_draft.trim().to_owned();
                if trimmed.is_empty() {
                    self.discord_client_id_draft.clear();
                    self.settings.discord_client_id.clear();
                    self.push_settings();
                    Task::none()
                } else if retrograde_linux::is_valid_client_id(&trimmed) {
                    self.discord_client_id_draft = trimmed.clone();
                    self.settings.discord_client_id = trimmed;
                    self.push_settings();
                    Task::none()
                } else {
                    self.toasts.push(
                        NoticeKind::Warn,
                        "INVALID_DISCORD_ID // EXPECTED 17-20 DIGITS".to_owned(),
                    )
                }
            }
            Message::ResetDiscordClientId => {
                self.discord_client_id_draft.clear();
                self.settings.discord_client_id.clear();
                self.push_settings();
                Task::none()
            }
            Message::ToggleWatchFolders(v) => {
                self.settings.watch_folders = v;
                self.push_settings();
                Task::none()
            }
            Message::ToggleAccentFromCover(v) => {
                self.settings.accent_from_cover = v;
                self.push_settings();
                Task::none()
            }
            Message::ToggleMinimizeToTray(v) => {
                self.settings.minimize_to_tray = Some(v);
                self.push_settings();
                Task::none()
            }
            Message::VisualizerBarPressed(idx) => {
                let (token, reset_gen, matched) = self.egg_morse.press(idx);
                self.visualizer_cache.clear();
                let release_task =
                    Task::perform(tokio::time::sleep(Duration::from_millis(150)), move |_| {
                        Message::VisualizerBarRelease(idx, token)
                    });
                let extra_task = if matched {
                    self.open_egg()
                } else {
                    Task::perform(tokio::time::sleep(Duration::from_secs(3)), move |_| {
                        Message::EggMorseTimeout(reset_gen)
                    })
                };
                Task::batch([release_task, extra_task])
            }
            Message::VisualizerBarRelease(idx, token) => {
                if self.egg_morse.release_bar(idx, token) {
                    self.visualizer_cache.clear();
                }
                Task::none()
            }
            Message::EggMorseTimeout(generation) => {
                self.egg_morse.timeout(generation);
                Task::none()
            }
            Message::EggAssetsReady(assets) => {
                self.egg_loading = false;
                if self.egg_open {
                    self.egg_assets = assets;
                }
                Task::none()
            }
            Message::EggImageHover(hovered) => {
                self.egg_hovered = hovered;
                if !self.render_tier.enable_animations {
                    self.egg_color_alpha = if hovered { 1.0 } else { 0.0 };
                }
                Task::none()
            }
            Message::CloseEgg => {
                self.close_egg();
                Task::none()
            }
        }
    }

    pub fn open_egg(&mut self) -> Task<Message> {
        self.egg_open = true;
        self.egg_hovered = false;
        self.egg_color_alpha = 0.0;
        self.egg_pulse_phase = 0.0;
        if self.egg_assets.is_none() && !self.egg_loading {
            self.egg_loading = true;
            return Task::perform(tokio::task::spawn_blocking(egg::decode_egg_assets), |r| {
                Message::EggAssetsReady(r.ok().flatten())
            });
        }
        Task::none()
    }

    fn close_egg(&mut self) {
        self.egg_open = false;
        self.egg_hovered = false;
        self.egg_color_alpha = 0.0;
        self.egg_assets = None;
    }

    fn handle_key_shortcut(
        &mut self,
        key: Key,
        modifiers: Modifiers,
        ignored: bool,
    ) -> Task<Message> {
        if matches!(key.as_ref(), Key::Named(keyboard::key::Named::Escape)) {
            // Exactly one layer closes per Escape press.
            if self.bg_prompt {
                self.bg_prompt = false;
            } else if self.context_menu.is_some() {
                self.context_menu = None;
            } else if self.confirm.is_some() {
                self.confirm = None;
            } else if self.sort_menu_open || self.bank_menu_open || self.add_music_menu_open {
                self.sort_menu_open = false;
                self.bank_menu_open = false;
                self.add_music_menu_open = false;
            } else if self.show_shortcuts_modal {
                self.show_shortcuts_modal = false;
            } else if self.egg_open {
                self.close_egg();
            } else if self.create_playlist_open {
                self.create_playlist_open = false;
            } else if self.queue_open || self.audio_open {
                self.queue_open = false;
                self.audio_open = false;
            } else if self.select_mode {
                self.select_mode = false;
                self.selected_albums.clear();
            } else if !ignored {
                // A focused text_input consumed this Escape to unfocus itself; stop here.
            } else if self.tag_draft.is_some() {
                if !self.tag_draft.as_ref().is_some_and(|d| d.saving) {
                    self.tag_draft = None;
                    self.tag_status = None;
                }
            } else if self.smart_draft.is_some() {
                self.smart_draft = None;
            } else if self.active_tab == Tab::NowPlaying {
                self.active_tab = self.previous_tab;
            } else if self.current_page().is_some() {
                return self.go_back();
            } else if self.active_tab == Tab::Search {
                self.active_tab = self.previous_tab;
            } else if self.mini_window.is_some() {
                return self.restore_main_from_mini();
            }
            return Task::none();
        }

        // Captured keys were typed into a widget; drafts (tag/smart editors) own the keyboard.
        if !ignored || self.tag_draft.is_some() || self.smart_draft.is_some() {
            return Task::none();
        }

        // Keys like Ctrl+M, arrows Up/Down and F5 are not captured by a focused
        // text_input, so confirm no text field has focus before acting on them.
        iced::advanced::widget::operate(AnyTextInputFocused::default()).map(move |focused| {
            Message::FocusCheckedShortcut {
                key: key.clone(),
                modifiers,
                text_input_focused: focused,
            }
        })
    }

    fn dispatch_shortcut(&mut self, key: Key, modifiers: Modifiers) -> Task<Message> {
        if self.egg_open {
            return Task::none();
        }
        // While the shortcuts card is open only "?" (close) works; the player underneath stays put.
        if self.show_shortcuts_modal && !matches!(key.as_ref(), Key::Character("?")) {
            return Task::none();
        }
        if modifiers.control() {
            match key.as_ref() {
                Key::Character("m") | Key::Character("M") => return self.toggle_mini_player(),
                Key::Character("s") | Key::Character("S") => {
                    let _ = self.cmd_tx.send(ControllerCommand::ToggleShuffle);
                }
                Key::Character("r") | Key::Character("R") => {
                    let _ = self.cmd_tx.send(ControllerCommand::CycleRepeat);
                }
                Key::Character("b") | Key::Character("B") => {
                    self.toggle_sidebar();
                }
                Key::Character("f") | Key::Character("F") => {
                    self.active_tab = Tab::Search;
                    return operation::focus(self.search_input_id.clone());
                }
                _ => {}
            }
            return Task::none();
        }

        match key.as_ref() {
            Key::Named(keyboard::key::Named::Space) if modifiers.is_empty() => {
                let _ = self.cmd_tx.send(ControllerCommand::TogglePlayPause);
            }
            Key::Named(keyboard::key::Named::ArrowRight) if modifiers.shift() => {
                let target = (self.position_s + 10.0).min(self.duration_s.max(0.0));
                let _ = self.cmd_tx.send(ControllerCommand::SeekSeconds(target));
            }
            Key::Named(keyboard::key::Named::ArrowLeft) if modifiers.shift() => {
                let target = (self.position_s - 10.0).max(0.0);
                let _ = self.cmd_tx.send(ControllerCommand::SeekSeconds(target));
            }
            Key::Named(keyboard::key::Named::ArrowRight) if modifiers.is_empty() => {
                let _ = self.cmd_tx.send(ControllerCommand::Next);
            }
            Key::Named(keyboard::key::Named::ArrowLeft) if modifiers.is_empty() => {
                let _ = self.cmd_tx.send(ControllerCommand::Previous);
            }
            Key::Named(keyboard::key::Named::ArrowUp) if modifiers.is_empty() => self.step_volume(1.0),
            Key::Named(keyboard::key::Named::ArrowDown) if modifiers.is_empty() => self.step_volume(-1.0),
            Key::Named(keyboard::key::Named::F5) => {
                let _ = self.cmd_tx.send(ControllerCommand::RescanLibrary);
            }
            Key::Character("/") if !modifiers.alt() => {
                self.active_tab = Tab::Search;
                return operation::focus(self.search_input_id.clone());
            }
            Key::Character("?") => {
                self.show_shortcuts_modal = !self.show_shortcuts_modal;
            }
            Key::Character("m") | Key::Character("M") if modifiers.is_empty() => {
                let _ = self.cmd_tx.send(ControllerCommand::ToggleMute);
            }
            Key::Character("f") | Key::Character("F") if modifiers.is_empty() => {
                if self.active_tab == Tab::NowPlaying {
                    self.active_tab = self.previous_tab;
                } else {
                    self.previous_tab = self.active_tab;
                    self.active_tab = Tab::NowPlaying;
                }
            }
            Key::Character("k") | Key::Character("K") if modifiers.is_empty() => self.step_volume(1.0),
            Key::Character("j") | Key::Character("J") if modifiers.is_empty() => self.step_volume(-1.0),
            _ => {}
        }
        Task::none()
    }

    fn toggle_sidebar(&mut self) {
        self.settings.sidebar_visible = !self.settings.sidebar_visible;
        self.bank_viewport_h = None;
        let target = if self.settings.sidebar_visible { 244.0 } else { 60.0 };
        if self.render_tier.enable_animations && self.shot.is_none() {
            self.sidebar_anim = Some((Instant::now(), self.sidebar_width, target));
        } else {
            self.sidebar_width = target;
            self.sidebar_anim = None;
        }
        self.push_settings();
    }

    fn request_close_main(&mut self) -> Task<Message> {
        // As the original: never chosen asks (BACKGROUND_PLAY), then hide or quit. Without a tray
        // there is no way back to a hidden window, so the app quits instead.
        match self.settings.minimize_to_tray {
            None if self.tray_available => {
                self.bg_remember = false;
                self.bg_prompt = true;
                Task::none()
            }
            Some(true) if self.tray_available => self.hide_main_window(),
            _ => self.begin_shutdown(),
        }
    }

    /// Closes the main window to the tray (playback goes on).
    fn hide_main_window(&mut self) -> Task<Message> {
        self.window_visible = false;
        match self.main_window.take() {
            Some(id) => window::close(id),
            None => Task::none(),
        }
    }

    /// A playlist is being deleted: its page closes to the library grid and leaves the page history.
    fn leave_deleted_page(&mut self, view: PlaylistView) {
        self.page_stack.retain(|p| *p != pages::Page::Playlist(view));
        if self.playlist_view == view {
            self.playlist_view = PlaylistView::Liked;
            if self.shown_tab() == Tab::Playlists {
                self.album_detail_open = false;
                self.artist_detail_open = false;
                self.page_stack.clear();
                if self.active_tab == Tab::NowPlaying {
                    self.previous_tab = Tab::Library;
                } else {
                    self.active_tab = Tab::Library;
                }
            }
        }
    }

    /// The WARNING dialog was confirmed: do what it asked.
    fn accept_confirm(&mut self) -> Task<Message> {
        let Some(d) = self.confirm.take() else { return Task::none() };
        match d.action {
            dialogs::ConfirmAction::PruneMissing(roots) => {
                let _ = self.cmd_tx.send(ControllerCommand::PruneMissing { roots });
                Task::none()
            }
            dialogs::ConfirmAction::DeletePlaylist { id, cover_art } => {
                self.leave_deleted_page(PlaylistView::Custom(id));
                let _ = self.cmd_tx.send(ControllerCommand::DeletePlaylist(id));
                let toast = self.toasts.push(NoticeKind::Info, format!("PLAYLIST_DELETED // {}", d.target));
                // The playlist's own cover files go too (files named native_pl_*, nothing else is touched)
                let covers = Task::future(tokio::task::spawn_blocking(move || {
                    retrograde_core::covers::PlaylistCovers::delete(cover_art.as_deref())
                }))
                .discard();
                Task::batch([toast, covers])
            }
            dialogs::ConfirmAction::DeleteSmartPlaylist(id) => {
                self.leave_deleted_page(PlaylistView::Smart(id));
                let _ = self.cmd_tx.send(ControllerCommand::DeleteSmartPlaylist(id));
                Task::none()
            }
            dialogs::ConfirmAction::DeleteAlbums(names) => {
                if self
                    .selected_album
                    .as_ref()
                    .is_some_and(|sel| names.contains(&sel.name))
                {
                    self.album_detail_open = false;
                    self.selected_album = None;
                    self.page_stack.retain(|p| !matches!(p, pages::Page::Album(k) if names.contains(&k.name)));
                    if self.shown_tab() == Tab::Library {
                        self.library_mode = LibraryMode::Albums;
                    }
                }
                self.select_mode = false;
                self.selected_albums.clear();
                self.deleting_albums = true;
                let _ = self.cmd_tx.send(ControllerCommand::DeleteAlbums(names));
                Task::none()
            }
        }
    }

    /// J / K / arrows: one of the meter's 15 segments, snapped like the original.
    fn step_volume(&mut self, dir: f64) {
        let v = (((self.volume * 15.0).round() + dir) / 15.0).clamp(0.0, 1.0);
        self.volume = v;
        let _ = self.cmd_tx.send(ControllerCommand::SetVolume(v));
    }

    fn toggle_mini_player(&mut self) -> Task<Message> {
        if self.mini_window.is_some() {
            return self.restore_main_from_mini();
        }
        self.mini_hovered = false;
        self.mini_marquee_phase = 0.0;
        let (mid, open_mini) = open_mini_window(self.settings.mini_x, self.settings.mini_y);
        self.mini_window = Some(mid);
        let mut tasks = vec![open_mini.map(Message::MiniWindowOpened)];
        if let Some(main_id) = self.main_window.take() {
            self.window_visible = false;
            tasks.push(window::close(main_id));
        }
        Task::batch(tasks)
    }

    fn restore_main_from_mini(&mut self) -> Task<Message> {
        self.mini_hovered = false;
        self.mini_marquee_phase = 0.0;
        self.push_settings();
        let mut tasks = Vec::new();
        if let Some(mid) = self.mini_window.take() {
            tasks.push(window::close(mid));
        }
        if let Some(main_id) = self.main_window {
            self.window_visible = true;
            tasks.push(window::gain_focus(main_id));
        } else {
            let (id, open_task) = open_main_window();
            self.main_window = Some(id);
            self.window_visible = true;
            tasks.push(open_task.map(Message::WindowOpened));
        }
        Task::batch(tasks)
    }

    fn begin_shutdown(&mut self) -> Task<Message> {
        if self.shutting_down {
            return Task::none();
        }
        self.shutting_down = true;
        if self.mini_pos_dirty {
            self.push_settings();
        }
        let _ = self.cmd_tx.send(ControllerCommand::Shutdown);
        // Generous watchdog: the coordinator flushes persistence (~2s) and tears down
        // mpv / direct audio before reporting ShutdownComplete.
        Task::perform(tokio::time::sleep(Duration::from_millis(4000)), |_| {
            Message::ForceExit
        })
    }

    fn push_settings(&mut self) {
        self.mini_pos_dirty = false;
        let _ = self
            .cmd_tx
            .send(ControllerCommand::UpdateSettings(self.settings.clone()));
    }

    fn handle_controller_event(&mut self, evt: ControllerEvent) -> Task<Message> {
        match evt {
            ControllerEvent::LibraryLoaded(albums) => {
                self.covers.sync_albums(&albums);
                self.covers.sync_collections(&self.collections);
                self.artists = group_artists(&albums);
                self.albums = albums;
                self.rebuild_track_map();
                self.rebuild_library_view();
                self.rebuild_search();
                self.rebuild_smart_counts();
                self.rebuild_smart_cache();
                self.recompute_smart_draft_count();
                if let Some(t) = &self.current_track {
                    if let Some(p) = t.cover_full_path.as_deref() {
                        self.covers.ensure_full(p);
                    }
                    if let Some(p) = t.cover_thumb_path.as_deref() {
                        self.covers.ensure_thumb(p);
                    }
                }
            }
            ControllerEvent::CollectionsUpdated(col) => {
                self.covers.sync_collections(&col);
                self.collections = col;
                self.rebuild_search();
                self.rebuild_liked_set();
                self.rebuild_smart_counts();
                self.rebuild_smart_cache();
                self.recompute_smart_draft_count();
            }
            ControllerEvent::StatsUpdated(stats) => {
                self.stats = stats;
                // The grid is not reordered on every play (as the original), but the first stats count
                if !self.stats_seen {
                    self.stats_seen = true;
                    if self.library_sort.uses_stats() {
                        self.rebuild_library_view();
                    }
                }
                self.rebuild_smart_counts();
                if matches!(self.playlist_view, PlaylistView::Smart(_)) {
                    self.rebuild_smart_cache();
                }
                self.recompute_smart_draft_count();
            }
            ControllerEvent::QueueUpdated {
                tracks,
                index,
                shuffle,
                repeat,
                hand_len,
            } => {
                self.queue_tracks = tracks;
                self.queue_index = index;
                self.shuffle = shuffle;
                self.repeat_mode = repeat;
                self.queue_hand_len = hand_len;
            }
            ControllerEvent::DevicesUpdated(devs) => {
                self.devices = devs;
            }
            ControllerEvent::TrackChanged(track) => {
                let prev_id = self.current_track.as_ref().map(|t| t.id.as_str());
                let next_id = track.as_ref().map(|t| t.id.as_str());
                if prev_id != next_id {
                    self.lyrics = None;
                    self.seeking_pos = None;
                }
                if let Some(t) = &track {
                    if let Some(p) = t.cover_full_path.as_deref() {
                        self.covers.ensure_full(p);
                    }
                    if let Some(p) = t.cover_thumb_path.as_deref() {
                        self.covers.ensure_thumb(p);
                    }
                }
                self.current_track = track;
            }
            ControllerEvent::AccentPaletteChanged(palette) => {
                let (pal, from_cover) = match palette {
                    Some(p) => (p, true),
                    None => (CoverPalette::ORBIT_DEFAULT, false),
                };
                let animate = self.render_tier.enable_animations;
                self.accent_anim.set_target(pal, from_cover, animate);
                if !animate {
                    self.orbit_theme = theme::orbit_theme(self.accent_anim.current_accent);
                }
            }
            ControllerEvent::LyricsUpdated { track_id, lyrics } => {
                if track_id.as_ref() == self.current_track.as_ref().map(|t| &t.id) {
                    self.lyrics = lyrics;
                }
            }
            ControllerEvent::PlaybackState {
                is_playing,
                position_s,
                duration_s,
            } => {
                let was_playing = self.is_playing;
                if was_playing != is_playing {
                    self.visualizer_cache.clear();
                }
                self.is_playing = is_playing;
                self.position_s = position_s;
                self.duration_s = duration_s;

                if self.is_unfocused_open_main() && was_playing && is_playing {
                    let seq = PRESENT_SEQ.load(Ordering::SeqCst);
                    if let WatchdogAction::ArmAndSuspend { expected_seq } =
                        self.watchdog.on_tick(seq, Instant::now())
                    {
                        // Dekker store-load check with `map_present_event` so a frame arriving right
                        // as we arm `WAKE_ON_PRESENT` can never cause a lost wakeup.
                        WAKE_ON_PRESENT.store(true, Ordering::SeqCst);
                        let after = PRESENT_SEQ.load(Ordering::SeqCst);
                        if !self.watchdog.confirm_suspend(after, expected_seq) {
                            WAKE_ON_PRESENT.store(false, Ordering::SeqCst);
                        }
                    }
                }
            }
            ControllerEvent::VolumeState { volume, muted } => {
                self.muted = muted;
                if self.dragging_volume.is_none() {
                    self.volume = volume;
                    self.settings.volume = volume;
                }
            }
            ControllerEvent::DirectStatus {
                audio_device,
                bit_perfect_status,
                direct_active,
                direct_volume_hint,
                audio_path,
                pw_bridge_node_id,
                direct_mixer_control,
            } => {
                self.audio_device = audio_device;
                self.bit_perfect_status = bit_perfect_status;
                self.direct_active = direct_active;
                self.direct_volume_hint = direct_volume_hint;
                self.audio_path = audio_path;
                self.pw_bridge_node_id = pw_bridge_node_id;
                self.direct_mixer_control = direct_mixer_control;
            }
            ControllerEvent::PowerTierChanged { profile, tier } => {
                self.power_profile = profile;
                self.render_tier = tier;
            }
            ControllerEvent::SettingsSaved(s) => {
                let (mini_x, mini_y) = (self.settings.mini_x, self.settings.mini_y);
                let prev_id = self.settings.discord_client_id.clone();
                self.settings = s;
                if prev_id != self.settings.discord_client_id && self.active_tab != Tab::Settings {
                    self.discord_client_id_draft = self.settings.discord_client_id.clone();
                }
                if self.mini_pos_dirty {
                    // Keep the unsaved local drag position until it is pushed.
                    self.settings.mini_x = mini_x;
                    self.settings.mini_y = mini_y;
                }
            }
            ControllerEvent::Notice { kind, text } => {
                return self.toasts.push(kind, text);
            }
            ControllerEvent::ScanProgress {
                phase,
                scanned,
                total,
            } => {
                let started = self.scan.as_ref().map_or_else(Instant::now, |s| s.started);
                self.scan = Some(toasts::ScanState { phase, scanned, total, started });
            }
            ControllerEvent::ScanFinished => {
                self.scan = None;
            }
            ControllerEvent::MissingTracks { count, roots } => {
                // Files gone from disk are only dropped once the user agrees: the folder may be moved or offline
                self.confirm = Some(dialogs::ConfirmDialog {
                    title: "WARNING: MISSING_FILES",
                    target_label: "TARGETS: ",
                    target: format!("{count} TRACK{}", if count == 1 { "" } else { "S" }),
                    target_orange: true,
                    detail: None,
                    note: "NOT ON DISK. REMOVE FROM LIBRARY?",
                    confirm: "REMOVE",
                    action: dialogs::ConfirmAction::PruneMissing(roots),
                });
            }
            ControllerEvent::AlbumsDeleteFinished => {
                self.deleting_albums = false;
            }
            ControllerEvent::CoversInvalidated(paths) => {
                self.covers.invalidate_paths(&paths);
                if self.current_track.as_ref().is_some_and(|t| {
                    t.cover_thumb_path.as_ref().is_some_and(|p| paths.contains(p))
                        || t.cover_full_path.as_ref().is_some_and(|p| paths.contains(p))
                }) {
                    self.fs_blur_tried = None;
                    self.fs_blur = None;
                    return self.fs_refresh_backdrop();
                }
            }
            ControllerEvent::TagsSaved {
                first_path,
                written,
                failed,
            } => {
                if let Some(draft) = &mut self.tag_draft
                    && (draft.saving || draft.paths.first() == first_path.as_ref())
                {
                    if !failed.is_empty() {
                        draft.saving = false;
                        self.tag_status = Some(Err(failed.join("; ")));
                    } else if written > 0 {
                        self.tag_draft = None;
                        self.tag_status = None;
                    } else {
                        draft.saving = false;
                    }
                }
            }
            ControllerEvent::TrayAvailable(available) => {
                self.tray_available = available;
            }
            ControllerEvent::DiscordRpcStatus(status) => {
                self.discord_rpc_status = status;
            }
            ControllerEvent::WindowCommand(action) => match action {
                WindowAction::Raise => {
                    if let Some(id) = self.main_window {
                        return window::gain_focus(id);
                    } else if self.mini_window.is_some() {
                        return self.restore_main_from_mini();
                    } else {
                        let (id, open_task) = open_main_window();
                        self.main_window = Some(id);
                        self.window_visible = true;
                        return open_task.map(Message::WindowOpened);
                    }
                }
                WindowAction::Toggle => {
                    if self.window_visible
                        && let Some(id) = self.main_window.take()
                    {
                        self.window_visible = false;
                        return window::close(id);
                    } else if self.mini_window.is_some() {
                        return self.restore_main_from_mini();
                    } else {
                        let (id, open_task) = open_main_window();
                        self.main_window = Some(id);
                        self.window_visible = true;
                        return open_task.map(Message::WindowOpened);
                    }
                }
                WindowAction::ToggleMiniPlayer => {
                    return self.toggle_mini_player();
                }
                WindowAction::Quit => {
                    return self.begin_shutdown();
                }
            },
            ControllerEvent::ShutdownComplete => {
                return iced::exit();
            }
        }
        Task::none()
    }

    fn view(&self, window_id: window::Id) -> Element<'_, Message> {
        if self.mini_window == Some(window_id) {
            views::render_mini_player(self)
        } else {
            views::render_app(self)
        }
    }
}

/// Receives raw `RedrawRequested` events (which `event::listen_with` filters out in iced 0.14).
/// Returns `None` on normal frames so it triggers no extra redraws; only emits `WindowPresented`
/// once when `WAKE_ON_PRESENT` was armed during compositor suspension.
fn map_present_event(
    event: Event,
    _status: event::Status,
    window_id: window::Id,
) -> Option<Message> {
    let Event::Window(window::Event::RedrawRequested(_)) = event else {
        return None;
    };
    PRESENT_SEQ.fetch_add(1, Ordering::SeqCst);
    if WAKE_ON_PRESENT.load(Ordering::SeqCst)
        && WAKE_ON_PRESENT.swap(false, Ordering::SeqCst)
    {
        Some(Message::WindowPresented(window_id))
    } else {
        None
    }
}

fn map_runtime_event(
    event: Event,
    status: event::Status,
    window_id: window::Id,
) -> Option<Message> {
    match event {
        Event::Window(window::Event::Focused) => Some(Message::WindowFocused(window_id, true)),
        Event::Window(window::Event::Unfocused) => Some(Message::WindowFocused(window_id, false)),
        Event::Window(window::Event::Moved(pt)) => Some(Message::WindowMoved(window_id, pt)),
        Event::Window(window::Event::Resized(size)) => Some(Message::WindowResized(window_id, size)),
        Event::Window(window::Event::FileHovered(_)) => Some(Message::FileHovered),
        Event::Window(window::Event::FilesHoveredLeft) => Some(Message::FilesHoveredLeft),
        Event::Window(window::Event::FileDropped(path)) => Some(Message::FileDropped(path)),
        Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            modified_key,
            modifiers,
            repeat,
            ..
        }) => {
            // `key` ignores Shift, so Shift+/ would arrive as "/" and never as "?".
            let key = match (&key, &modified_key) {
                (Key::Character(_), Key::Character(_))
                    if !modifiers.control() && !modifiers.alt() && !modifiers.logo() =>
                {
                    modified_key
                }
                _ => key,
            };
            // Held keys repeat only where that is wanted: volume and seeking.
            let repeatable = match key.as_ref() {
                Key::Named(
                    keyboard::key::Named::ArrowUp
                    | keyboard::key::Named::ArrowDown
                    | keyboard::key::Named::ArrowLeft
                    | keyboard::key::Named::ArrowRight,
                ) => true,
                Key::Character(c) => modifiers.is_empty() && matches!(c, "j" | "J" | "k" | "K"),
                _ => false,
            };
            if repeat && !repeatable {
                return None;
            }
            Some(Message::KeyShortcut {
                key,
                modifiers,
                ignored: status == event::Status::Ignored,
            })
        }
        _ => None,
    }
}

fn open_main_window() -> (window::Id, Task<window::Id>) {
    window::open(window::Settings {
        size: iced::Size::new(1200.0, 800.0),
        min_size: Some(iced::Size::new(900.0, 600.0)),
        decorations: false,
        resizable: true,
        exit_on_close_request: false,
        platform_specific: window::settings::PlatformSpecific {
            application_id: "retrograde".to_owned(),
            ..Default::default()
        },
        ..Default::default()
    })
}

fn open_mini_window(mini_x: Option<i32>, mini_y: Option<i32>) -> (window::Id, Task<window::Id>) {
    let position = match (mini_x, mini_y) {
        (Some(x), Some(y)) if (-4000..=16000).contains(&x) && (-4000..=16000).contains(&y) => {
            window::Position::Specific(Point::new(x as f32, y as f32))
        }
        _ => window::Position::Default,
    };
    window::open(window::Settings {
        size: iced::Size::new(400.0, 122.0),
        position,
        resizable: false,
        decorations: false,
        level: window::Level::AlwaysOnTop,
        exit_on_close_request: false,
        platform_specific: window::settings::PlatformSpecific {
            application_id: "retrograde-mini".to_owned(),
            ..Default::default()
        },
        ..Default::default()
    })
}

fn controller_event_stream() -> impl Stream<Item = ControllerEvent> {
    iced::stream::channel(
        64,
        |mut output: iced::futures::channel::mpsc::Sender<ControllerEvent>| async move {
            let handoff = BOOTSTRAP_HANDOFF
                .get()
                .and_then(|m| m.lock().ok())
                .and_then(|mut g| g.take());

            let Some(handoff) = handoff else {
                return;
            };

            let (ui_tx, mut ui_rx) = mpsc::unbounded_channel();
            tokio::spawn(run_coordinator(handoff.cmd_rx, ui_tx, handoff.startup));

            while let Some(evt) = ui_rx.recv().await {
                if output.send(evt).await.is_err() {
                    break;
                }
            }
        },
    )
}

/// Track time as the original shows it: `m:ss`, or `h:mm:ss` from an hour on.
pub fn format_mm_ss(seconds: f64) -> String {
    let total = if seconds.is_finite() { seconds.max(0.0).floor() as u64 } else { 0 };
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{}:{s:02}", total / 60) }
}

fn parse_optional_u32(current: &str, orig: &str, label: &str) -> Result<Option<u32>, String> {
    if current == orig {
        return Ok(None);
    }
    let trimmed = current.trim();
    if trimmed.is_empty() {
        return Ok(Some(0));
    }
    trimmed
        .parse::<u32>()
        .map(Some)
        .map_err(|_| format!("Invalid {label}: enter a whole number (got '{trimmed}')"))
}

/// Canonicalizes a local CLI path so a secondary instance launched from another
/// working directory still forwards an absolute path; `file://` URIs pass through.
fn normalize_open_arg(arg: &str) -> String {
    if arg.len() >= 7 && arg[..7].eq_ignore_ascii_case("file://") {
        return arg.to_owned();
    }
    std::fs::canonicalize(arg)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| arg.to_owned())
}

/// Widget operation reporting whether any focusable widget (a `text_input`) has focus
/// in any window. Always yields a value so the shortcut task never stalls.
#[derive(Default)]
struct AnyTextInputFocused {
    focused: bool,
}

impl iced::advanced::widget::Operation<bool> for AnyTextInputFocused {
    fn traverse(
        &mut self,
        operate: &mut dyn FnMut(&mut dyn iced::advanced::widget::Operation<bool>),
    ) {
        if !self.focused {
            operate(self);
        }
    }

    fn focusable(
        &mut self,
        _id: Option<&Id>,
        _bounds: iced::Rectangle,
        state: &mut dyn iced::advanced::widget::operation::Focusable,
    ) {
        if state.is_focused() {
            self.focused = true;
        }
    }

    fn finish(&self) -> iced::advanced::widget::operation::Outcome<bool> {
        iced::advanced::widget::operation::Outcome::Some(self.focused)
    }
}

/// Tracks whether the compositor is still delivering `RedrawRequested` frames to an unfocused
/// open main window. After two consecutive playback ticks (>= 500 ms apart) with no presented
/// frame, requests arming `WAKE_ON_PRESENT` and pausing periodic ticks until a frame presents.
#[derive(Debug, Default)]
struct PresentWatchdog {
    suspended: bool,
    unpresented_ticks: u8,
    last_present_seq: u64,
    last_check_at: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WatchdogAction {
    None,
    ArmAndSuspend { expected_seq: u64 },
}

impl PresentWatchdog {
    fn is_suspended(&self) -> bool {
        self.suspended
    }

    fn reset(&mut self) {
        self.suspended = false;
        self.unpresented_ticks = 0;
        self.last_check_at = None;
    }

    fn on_unfocus(&mut self, seq: u64, now: Instant) {
        self.suspended = false;
        self.unpresented_ticks = 0;
        self.last_present_seq = seq;
        self.last_check_at = Some(now);
    }

    fn on_tick(&mut self, seq: u64, now: Instant) -> WatchdogAction {
        if self.suspended {
            return WatchdogAction::None;
        }
        if seq != self.last_present_seq {
            self.last_present_seq = seq;
            self.unpresented_ticks = 0;
            self.last_check_at = Some(now);
            return WatchdogAction::None;
        }
        if self
            .last_check_at
            .is_none_or(|t| now.duration_since(t) >= Duration::from_millis(500))
        {
            self.last_check_at = Some(now);
            self.unpresented_ticks = self.unpresented_ticks.saturating_add(1);
            if self.unpresented_ticks >= 2 {
                return WatchdogAction::ArmAndSuspend { expected_seq: seq };
            }
        }
        WatchdogAction::None
    }

    fn confirm_suspend(&mut self, observed_after_arm: u64, expected_seq: u64) -> bool {
        if observed_after_arm != expected_seq {
            self.last_present_seq = observed_after_arm;
            self.unpresented_ticks = 0;
            self.suspended = false;
            false
        } else {
            self.suspended = true;
            true
        }
    }

    fn on_presented(&mut self, seq: u64) {
        self.suspended = false;
        self.unpresented_ticks = 0;
        self.last_present_seq = seq;
        self.last_check_at = None;
    }
}

/// Resolves the periodic `TimePos` emission cadence for the current window presentation state.
/// Note: the mini-player intentionally stays `Active` at its tiny fixed overlay size (1 cover).
fn resolve_playback_tick_mode(
    has_mini_window: bool,
    window_visible: bool,
    has_main_window: bool,
    main_window_minimized: bool,
    main_window_focused: bool,
    compositor_suspended: bool,
) -> PlaybackTickMode {
    if has_mini_window {
        PlaybackTickMode::Active
    } else if !window_visible || !has_main_window || main_window_minimized {
        PlaybackTickMode::Paused
    } else if main_window_focused {
        PlaybackTickMode::Active
    } else if compositor_suspended {
        PlaybackTickMode::Paused
    } else {
        PlaybackTickMode::Unfocused
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Duration, Instant, PlaybackTickMode, PresentWatchdog, WatchdogAction, format_mm_ss,
        resolve_playback_tick_mode,
    };

    #[test]
    fn track_time_is_m_ss_like_the_original() {
        assert_eq!(format_mm_ss(0.0), "0:00");
        assert_eq!(format_mm_ss(163.9), "2:43");
        assert_eq!(format_mm_ss(600.0), "10:00");
        assert_eq!(format_mm_ss(3723.0), "1:02:03");
        assert_eq!(format_mm_ss(-5.0), "0:00");
        assert_eq!(format_mm_ss(f64::NAN), "0:00");
    }

    #[test]
    fn resolves_playback_tick_mode_for_all_window_states() {
        // Focused main window is always active.
        assert_eq!(
            resolve_playback_tick_mode(false, true, true, false, true, false),
            PlaybackTickMode::Active
        );
        // Open mini-player keeps active ticks even when main window is hidden or unfocused.
        assert_eq!(
            resolve_playback_tick_mode(true, false, false, true, false, true),
            PlaybackTickMode::Active
        );
        // Visible unfocused main window (side-by-side or another monitor) uses 1 Hz ticks.
        assert_eq!(
            resolve_playback_tick_mode(false, true, true, false, false, false),
            PlaybackTickMode::Unfocused
        );
        // Unfocused main window whose frames are withheld by the compositor pauses ticks.
        assert_eq!(
            resolve_playback_tick_mode(false, true, true, false, false, true),
            PlaybackTickMode::Paused
        );
        // Minimized or closed-to-tray main window pauses ticks.
        assert_eq!(
            resolve_playback_tick_mode(false, true, true, true, false, false),
            PlaybackTickMode::Paused
        );
        assert_eq!(
            resolve_playback_tick_mode(false, false, false, false, false, false),
            PlaybackTickMode::Paused
        );
    }

    #[test]
    fn watchdog_does_not_suspend_when_frames_advance() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut wd = PresentWatchdog::default();
        wd.on_unfocus(10, t0);

        // One stale tick does not suspend; an advancing frame resets the streak.
        assert_eq!(wd.on_tick(10, at(1000)), WatchdogAction::None);
        assert_eq!(wd.on_tick(11, at(2000)), WatchdogAction::None);
        assert_eq!(wd.on_tick(11, at(3000)), WatchdogAction::None);
        assert_eq!(wd.on_tick(12, at(4000)), WatchdogAction::None);
        assert!(!wd.is_suspended());
    }

    #[test]
    fn watchdog_ignores_burst_ticks_under_500ms() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut wd = PresentWatchdog::default();
        wd.on_unfocus(10, t0);

        assert_eq!(wd.on_tick(10, at(100)), WatchdogAction::None);
        assert_eq!(wd.on_tick(10, at(300)), WatchdogAction::None);
        assert_eq!(wd.on_tick(10, at(499)), WatchdogAction::None);
        assert!(!wd.is_suspended());
    }

    #[test]
    fn watchdog_suspends_after_two_stale_ticks_and_wakes_on_present() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut wd = PresentWatchdog::default();
        wd.on_unfocus(10, t0);

        assert_eq!(wd.on_tick(10, at(600)), WatchdogAction::None);
        assert_eq!(
            wd.on_tick(10, at(1200)),
            WatchdogAction::ArmAndSuspend { expected_seq: 10 }
        );
        assert!(wd.confirm_suspend(10, 10));
        assert!(wd.is_suspended());

        // Subsequent ticks stay no-ops until a frame presents.
        assert_eq!(wd.on_tick(10, at(5000)), WatchdogAction::None);
        wd.on_presented(11);
        assert!(!wd.is_suspended());
    }

    #[test]
    fn watchdog_cancels_suspend_when_frame_races_arm() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut wd = PresentWatchdog::default();
        wd.on_unfocus(10, t0);

        assert_eq!(wd.on_tick(10, at(600)), WatchdogAction::None);
        assert_eq!(
            wd.on_tick(10, at(1200)),
            WatchdogAction::ArmAndSuspend { expected_seq: 10 }
        );
        assert!(!wd.confirm_suspend(11, 10));
        assert!(!wd.is_suspended());
        // Sequence updated to 11 so the next stale tick at seq=11 counts cleanly as #1, not #2.
        assert_eq!(wd.on_tick(11, at(1800)), WatchdogAction::None);
        assert!(!wd.is_suspended());
    }
}
