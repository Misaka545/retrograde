/// Tray menu actions, as in the C# build (`App.axaml.cs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    /// Left click: hide the window when it is shown, else bring it back (from the mini player too).
    ToggleWindow,
    /// "Show Player".
    ShowWindow,
    /// "Mini player (Ctrl+M)".
    ToggleMiniPlayer,
    /// "Rescan library" (same as F5).
    RescanLibrary,
    /// "Minimize to tray on close" checkbox.
    ToggleMinimizeToTray,
    /// "Watch library folders for new files" checkbox.
    ToggleWatchFolders,
    /// "Accent colour from the cover" checkbox.
    ToggleAccentFromCover,
    /// "Exit".
    Quit,
}

/// The checkbox states shown in the tray menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TrayToggles {
    pub minimize_to_tray: bool,
    pub watch_folders: bool,
    pub accent_from_cover: bool,
}

#[cfg(feature = "tray")]
mod imp {
    use ksni::TrayMethods;
    use ksni::menu::{CheckmarkItem, MenuItem, StandardItem};
    use tokio::sync::mpsc;
    use tracing::debug;

    use super::{TrayCommand, TrayToggles};

    /// The app icon at the sizes trays ask for, as ARGB32 in network byte order.
    fn icons() -> &'static Vec<ksni::Icon> {
        static ICONS: std::sync::OnceLock<Vec<ksni::Icon>> = std::sync::OnceLock::new();
        ICONS.get_or_init(|| {
            let Ok(img) = image::load_from_memory(include_bytes!("../../../packaging/linux/retrograde.png")) else {
                return Vec::new();
            };
            [64u32, 32, 22]
                .into_iter()
                .map(|side| {
                    let small = img.resize_exact(side, side, image::imageops::FilterType::Lanczos3).to_rgba8();
                    let mut data = Vec::with_capacity((side * side * 4) as usize);
                    for px in small.pixels() {
                        data.extend_from_slice(&[px[3], px[0], px[1], px[2]]);
                    }
                    ksni::Icon { width: side as i32, height: side as i32, data }
                })
                .collect()
        })
    }

    struct RetrogradeTray {
        title: String,
        toggles: TrayToggles,
        tx: mpsc::UnboundedSender<TrayCommand>,
    }

    fn action(label: &str, cmd: TrayCommand) -> MenuItem<RetrogradeTray> {
        StandardItem {
            label: label.to_owned(),
            activate: Box::new(move |this: &mut RetrogradeTray| {
                let _ = this.tx.send(cmd);
            }),
            ..Default::default()
        }
        .into()
    }

    fn checkbox(label: &str, checked: bool, cmd: TrayCommand) -> MenuItem<RetrogradeTray> {
        CheckmarkItem {
            label: label.to_owned(),
            checked,
            activate: Box::new(move |this: &mut RetrogradeTray| {
                let _ = this.tx.send(cmd);
            }),
            ..Default::default()
        }
        .into()
    }

    impl ksni::Tray for RetrogradeTray {
        fn id(&self) -> String {
            "retrograde".to_owned()
        }

        fn title(&self) -> String {
            self.title.clone()
        }

        fn icon_name(&self) -> String {
            // The installer puts retrograde.png in the icon theme; the pixmap below covers hosts that cannot find it
            "retrograde".to_owned()
        }

        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            icons().clone()
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            let _ = self.tx.send(TrayCommand::ToggleWindow);
        }

        fn menu(&self) -> Vec<MenuItem<Self>> {
            let t = self.toggles;
            vec![
                action("Show Player", TrayCommand::ShowWindow),
                action("Mini player (Ctrl+M)", TrayCommand::ToggleMiniPlayer),
                action("Rescan library", TrayCommand::RescanLibrary),
                MenuItem::Separator,
                checkbox(
                    "Minimize to tray on close",
                    t.minimize_to_tray,
                    TrayCommand::ToggleMinimizeToTray,
                ),
                checkbox(
                    "Watch library folders for new files",
                    t.watch_folders,
                    TrayCommand::ToggleWatchFolders,
                ),
                checkbox(
                    "Accent colour from the cover",
                    t.accent_from_cover,
                    TrayCommand::ToggleAccentFromCover,
                ),
                MenuItem::Separator,
                action("Exit", TrayCommand::Quit),
            ]
        }
    }

    pub struct TrayService {
        handle: ksni::Handle<RetrogradeTray>,
    }

    impl TrayService {
        pub async fn start() -> Result<(Self, mpsc::UnboundedReceiver<TrayCommand>), String> {
            let (tx, rx) = mpsc::unbounded_channel();
            let tray = RetrogradeTray {
                title: "Retrograde".to_owned(),
                toggles: TrayToggles::default(),
                tx,
            };
            let handle = tray
                .spawn()
                .await
                .map_err(|e| format!("StatusNotifierItem tray unavailable: {e}"))?;
            debug!("StatusNotifierItem tray registered");
            Ok((Self { handle }, rx))
        }

        /// The tooltip title ("Retrograde — artist - title").
        pub async fn update_title(&self, title: String) {
            let _ = self.handle.update(|tray| tray.title = title).await;
        }

        pub async fn update_toggles(&self, toggles: TrayToggles) {
            let _ = self.handle.update(|tray| tray.toggles = toggles).await;
        }

        pub async fn shutdown(self) {
            let _ = self.handle.shutdown().await;
        }
    }
}

#[cfg(not(feature = "tray"))]
mod imp {
    use tokio::sync::mpsc;

    use super::{TrayCommand, TrayToggles};

    pub struct TrayService;

    impl TrayService {
        pub async fn start() -> Result<(Self, mpsc::UnboundedReceiver<TrayCommand>), String> {
            Err("System tray support was disabled at compile time".to_owned())
        }

        pub async fn update_title(&self, _title: String) {}

        pub async fn update_toggles(&self, _toggles: TrayToggles) {}

        pub async fn shutdown(self) {}
    }
}

pub use imp::TrayService;
