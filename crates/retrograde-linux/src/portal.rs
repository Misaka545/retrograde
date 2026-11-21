use std::path::PathBuf;

/// Opens a native Linux folder chooser via XDG Desktop Portal (`ashpd`) when enabled,
/// keeping Wayland dialogs sandbox-compatible without GTK or X11 dependencies.
#[cfg(feature = "portal")]
pub async fn pick_music_folders() -> Result<Vec<PathBuf>, String> {
    use ashpd::desktop::file_chooser::SelectedFiles;
    use ashpd::desktop::ResponseError;

    let req = SelectedFiles::open_file()
        .title("Add Music Folders")
        .directory(true)
        .multiple(true)
        .send()
        .await
        .map_err(|e| format!("XDG Portal file chooser request failed: {e}"))?;

    match req.response() {
        Ok(response) => Ok(uris_to_local_paths(response.uris())),
        Err(ashpd::Error::Response(ResponseError::Cancelled)) => Ok(Vec::new()),
        Err(e) => Err(format!("Folder selection failed: {e}")),
    }
}

/// Opens a native Linux file chooser for audio tracks via XDG Desktop Portal.
#[cfg(feature = "portal")]
pub async fn pick_music_files() -> Result<Vec<PathBuf>, String> {
    use ashpd::desktop::file_chooser::{FileFilter, SelectedFiles};
    use ashpd::desktop::ResponseError;

    const EXTS: &[&str] = &[
        "flac", "mp3", "wav", "ogg", "opus", "m4a", "aac", "wma", "aiff", "alac", "wv", "ape",
        "dsf", "dff",
    ];
    let mut filter = FileFilter::new("Audio Files");
    for ext in EXTS {
        filter = filter.glob(&format!("*.{ext}"));
    }

    let req = SelectedFiles::open_file()
        .title("Add Music Files")
        .multiple(true)
        .filter(filter)
        .send()
        .await
        .map_err(|e| format!("XDG Portal file chooser request failed: {e}"))?;

    match req.response() {
        Ok(response) => Ok(uris_to_local_paths(response.uris())),
        Err(ashpd::Error::Response(ResponseError::Cancelled)) => Ok(Vec::new()),
        Err(e) => Err(format!("Audio file selection failed: {e}")),
    }
}

/// Opens a native Linux file chooser for a single cover image via XDG Desktop Portal.
#[cfg(feature = "portal")]
pub async fn pick_cover_image() -> Result<Option<PathBuf>, String> {
    use ashpd::desktop::file_chooser::{FileFilter, SelectedFiles};
    use ashpd::desktop::ResponseError;

    let filter = FileFilter::new("Images")
        .glob("*.jpg")
        .glob("*.jpeg")
        .glob("*.png")
        .glob("*.webp");

    let req = SelectedFiles::open_file()
        .title("Choose Cover Image")
        .multiple(false)
        .filter(filter)
        .send()
        .await
        .map_err(|e| format!("XDG Portal file chooser request failed: {e}"))?;

    match req.response() {
        Ok(response) => Ok(uris_to_local_paths(response.uris()).into_iter().next()),
        Err(ashpd::Error::Response(ResponseError::Cancelled)) => Ok(None),
        Err(e) => Err(format!("Image selection failed: {e}")),
    }
}

#[cfg(feature = "portal")]
fn uris_to_local_paths(uris: &[ashpd::Uri]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for uri in uris {
        if let Some(path) = retrograde_core::CoverPaths::to_local_path(Some(uri.as_str())) {
            paths.push(path);
        }
    }
    paths
}

#[cfg(not(feature = "portal"))]
pub async fn pick_music_folders() -> Result<Vec<PathBuf>, String> {
    Err("XDG Desktop Portal support was disabled at compile time".to_owned())
}

#[cfg(not(feature = "portal"))]
pub async fn pick_music_files() -> Result<Vec<PathBuf>, String> {
    Err("XDG Desktop Portal support was disabled at compile time".to_owned())
}

#[cfg(not(feature = "portal"))]
pub async fn pick_cover_image() -> Result<Option<PathBuf>, String> {
    Err("XDG Desktop Portal support was disabled at compile time".to_owned())
}
