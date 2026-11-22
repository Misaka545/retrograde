//! Developer screenshots without input devices: with `RETRO_SHOT=<dir>` and `RETRO_SHOT_STEPS=lib,detail,...`
//! the app walks through the named views a few seconds after the window opens, saves the window's own picture
//! (not the screen) as `<dir>/<n>-<step>.png` for each, and exits. Used to compare the UI against the
//! original app view by view. Run it against a copy of the data folder (`RETRO_APPDIR`).

use std::path::PathBuf;
use std::time::Duration;

use iced::{Task, window};

use crate::controller::{ControllerEvent, NoticeKind};
use crate::{App, Message, PlaylistView, Tab};

#[derive(Debug, Clone)]
pub struct Plan {
    dir: PathBuf,
    steps: Vec<String>,
}

impl Plan {
    pub fn from_env() -> Option<Self> {
        let dir = PathBuf::from(std::env::var_os("RETRO_SHOT")?);
        let steps = std::env::var("RETRO_SHOT_STEPS")
            .unwrap_or_else(|_| "lib,detail,queue,eq,settings,fs".to_owned())
            .split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect();
        let _ = std::fs::create_dir_all(&dir);
        Some(Self { dir, steps })
    }
}

fn after(ms: u64, msg: Message) -> Task<Message> {
    Task::perform(tokio::time::sleep(Duration::from_millis(ms)), move |_| msg.clone())
}

impl App {
    /// Called when the main window has opened.
    pub fn shot_begin(&self) -> Task<Message> {
        if self.shot.is_some() { after(6000, Message::ShotStep(0)) } else { Task::none() }
    }

    /// Puts the app into the state the step names, then waits for it to be drawn before the picture is taken.
    pub fn shot_step(&mut self, i: usize) -> Task<Message> {
        let Some(plan) = self.shot.clone() else { return Task::none() };
        let Some(raw) = plan.steps.get(i) else { return iced::exit() };
        // "name@ms": take the picture ms after the change instead of waiting for it to settle
        let (step, delay) = match raw.split_once('@') {
            Some((n, ms)) => (n.to_owned(), ms.parse().unwrap_or(2200)),
            None => (raw.clone(), 2200),
        };
        let mut tasks: Vec<Task<Message>> = Vec::new();
        let mut go = |m: Message| tasks.push(Task::done(m));
        match step.as_str() {
            "close" => go(Message::SelectTab(self.previous_tab)),
            "lib" => {
                go(Message::GoHome);
                go(Message::SetLibraryMode(crate::LibraryMode::Albums));
            }
            "detail" => {
                if let Some(a) = self.albums.first() {
                    go(Message::SelectAlbum(crate::AlbumKey::from_album(a)));
                }
            }
            "queue" => go(Message::ToggleQueuePanel),
            "eq" => go(Message::ToggleAudioPanel),
            "output" => {
                go(Message::ToggleAudioPanel);
                go(Message::SetAudioOutputTab(true));
            }
            "settings" => go(Message::SelectTab(Tab::Settings)),
            "favorites" => go(Message::OpenPlaylistPage(PlaylistView::Liked)),
            "artists" => {
                go(Message::SelectTab(Tab::Library));
                go(Message::SetLibraryMode(crate::LibraryMode::Albums));
                go(Message::SetLibraryMode(crate::LibraryMode::Artists));
            }
            "search" => {
                go(Message::FocusSearch);
                go(Message::SearchChanged("ado".to_owned()));
            }
            "artist" => go(Message::OpenArtist("ado".to_owned())),
            // Overlays set straight from here: no close request (it could quit the run) and nothing is confirmed
            "toast" => {
                go(Message::Controller(ControllerEvent::Notice { kind: NoticeKind::Success, text: "SCAN_COMPLETE // 3 NEW".to_owned() }));
                go(Message::Controller(ControllerEvent::Notice { kind: NoticeKind::Error, text: "PLAYBACK_FAILED // file not found".to_owned() }));
                go(Message::Controller(ControllerEvent::ScanProgress { phase: "scanning", scanned: 120, total: 363 }));
            }
            "toastclose" => go(Message::Controller(ControllerEvent::ScanFinished)),
            // Direct (bit-perfect) output shown as active, without taking a card: then `eq` / `output`
            "direct" => go(Message::Controller(ControllerEvent::DirectStatus {
                audio_device: self
                    .devices
                    .iter()
                    .find(|d| d.is_direct)
                    .map_or_else(|| "alsa/hw:CARD=Pro,DEV=0".to_owned(), |d| d.id.clone()),
                bit_perfect_status: "BIT-PERFECT // 44.1 KHZ // 16-BIT // S16_LE".to_owned(),
                direct_active: true,
                direct_volume_hint: "VOLUME SETS THE CARD'S OWN PCM CONTROL, THE FILE'S SAMPLES ARE NOT TOUCHED. THE SYSTEM VOLUME REACHES IT THROUGH THE (DIRECT) BRIDGE SINK.".to_owned(),
                audio_path: retrograde_audio::AudioPath { input: None, output: None },
                pw_bridge_node_id: None,
                direct_mixer_control: Some("PCM".to_owned()),
            })),
            "confirm" => go(Message::Controller(ControllerEvent::MissingTracks { count: 12, roots: Vec::new() })),
            "bgprompt" => self.bg_prompt = true,
            "dismiss" => {
                self.confirm = None;
                self.bg_prompt = false;
                self.sort_menu_open = false;
                self.bank_menu_open = false;
                self.context_menu = None;
                self.add_music_menu_open = false;
                self.select_mode = false;
                self.selected_albums.clear();
                self.smart_draft = None;
                self.tag_draft = None;
                self.tag_status = None;
                self.create_playlist_open = false;
                self.show_shortcuts_modal = false;
                go(Message::CloseEgg);
            }
            "tagalbum" => {
                if let Some(a) = self.albums.first() {
                    go(Message::OpenAlbumTagEditor(crate::AlbumKey::from_album(a)));
                }
            }
            "tagtrack" => {
                if let Some(a) = self.albums.first()
                    && let Some(t) = a.tracks.first()
                {
                    go(Message::OpenTagEditor(t.clone(), a.artist.clone(), a.year));
                }
            }
            "sortmenu" => {
                go(Message::GoHome);
                go(Message::ToggleSortMenu);
            }
            "banknew" => go(Message::ToggleBankMenu),
            "select" => {
                go(Message::GoHome);
                go(Message::EnterSelectMode);
                go(Message::SelectAllAlbums);
            }
            "addmenu" => {
                go(Message::GoHome);
                go(Message::ToggleAddMusicMenu);
            }
            "ctxalbum" => {
                if let Some(a) = self.albums.first() {
                    go(Message::OpenAlbumMenu(crate::AlbumKey::from_album(a), iced::Point::new(320.0, 200.0)));
                }
            }
            "ctxtrack" => {
                if let Some(a) = self.albums.first()
                    && let Some(t) = a.tracks.first()
                {
                    go(Message::OpenTrackMenu {
                        track: t.clone(),
                        album_artist: a.artist.clone(),
                        album_year: a.year,
                        removable_playlist: None,
                        pos: iced::Point::new(500.0, 420.0),
                    });
                }
            }
            "detailmenu" => {
                if let Some(a) = self.albums.first() {
                    go(Message::OpenDetailMenu(
                        crate::pages::DetailCtx::Album(crate::AlbumKey::from_album(a)),
                        iced::Point::new(420.0, 320.0),
                    ));
                }
            }
            "empty" => {
                go(Message::GoHome);
                self.library_albums.clear();
            }
            "rail" => {
                if self.settings.sidebar_visible {
                    go(Message::ToggleSidebar);
                }
            }
            "expand" => {
                if !self.settings.sidebar_visible {
                    go(Message::ToggleSidebar);
                }
            }
            // The mini player replaces the main window: put it last, the picture is then of the mini window
            "mini" => go(Message::ToggleMiniPlayer),
            "smart" => go(Message::NewSmartPlaylist),
            "create" => go(Message::ToggleCreatePlaylist(true)),
            "playlists" => {
                go(Message::SetBankPlaylists(true));
                go(Message::ToggleCreatePlaylist(false));
            }
            "fs" => go(Message::SelectTab(Tab::NowPlaying)),
            "lyrics" => go(Message::SetFsLyricsTab(true)),
            "shortcuts" => go(Message::ToggleShortcutsModal),
            "egg" => tasks.push(self.open_egg()),
            other => tracing::warn!(step = other, "unknown RETRO_SHOT step"),
        }
        tasks.push(after(delay, Message::ShotGo(i)));
        Task::batch(tasks)
    }

    pub fn shot_go(&self, i: usize) -> Task<Message> {
        match self.main_window.or(self.mini_window) {
            Some(id) => window::screenshot(id).map(move |s| Message::ShotTaken(i, s)),
            None => Task::none(),
        }
    }

    pub fn shot_taken(&mut self, i: usize, shot: window::Screenshot) -> Task<Message> {
        if let Some(plan) = &self.shot {
            let name = plan.steps.get(i).map_or("?", String::as_str);
            if let Some(img) = image::RgbaImage::from_raw(shot.size.width, shot.size.height, shot.rgba.to_vec()) {
                let _ = img.save(plan.dir.join(format!("{i}-{name}.png")));
            }
        }
        after(600, Message::ShotStep(i + 1))
    }
}
