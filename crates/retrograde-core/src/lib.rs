pub mod collections;
pub mod covers;
pub mod electron_import;
pub mod error_log;
pub mod library;
pub mod lyrics;
pub mod models;
pub mod paths;
pub mod scanner;
pub mod session;
pub mod settings;
pub mod smart;
pub mod stats;
pub mod tag_writer;
pub mod watcher;

pub use collections::{
    CollectionsData, CollectionsStore, ExtraMap, PlaylistData, SmartPlaylistData, SmartRule,
};
pub use covers::{
    CoverPalette, ImageLimits, MIN_ACCENT_LUMINANCE, PlaylistCovers, extract_cover_palette,
    extract_cover_palette_from_rgb, relative_luminance,
};
pub use library::{AlbumDto, LibraryStore, TrackDto};
pub use lyrics::{LyricLine, LyricsDoc, LyricsLoader};
pub use models::{Album, ArtistSummary, Track};
pub use paths::CoverPaths;
pub use scanner::{ScanProgress, ScanResult, Scanner};
pub use session::{SessionState, SessionStore};
pub use settings::{
    AppSettings, DirectVolumeBridge, DisplayBackend, GpuPreference, LIBRARY_SORT_KEYS,
    PerformanceMode, canonical_library_sort,
};
pub use smart::{Codecs, SmartField, SmartPlaylists};
pub use stats::{PlayStat, PlayStats};
pub use tag_writer::{TagEdit, TagWriter};
pub use watcher::FolderWatcher;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "Run explicitly with RETRO_APPDIR pointing to a scratch copy of ~/.retrograde"]
    fn real_data_compatibility_from_retro_appdir() {
        let app_dir = CoverPaths::app_dir();
        assert!(app_dir.exists(), "app_dir missing: {}", app_dir.display());

        let settings = AppSettings::load();
        assert!(settings.volume >= 0.0 && settings.volume <= 1.0);

        let albums = LibraryStore::load().expect("load library.json");
        assert!(!albums.is_empty(), "expected non-empty library.json");

        let collections = CollectionsStore::load();
        let (_session_state, session_queue) = SessionStore::load().unwrap_or_default();
        let stats = PlayStats::load();

        let _all_tracks = library::build_track_index(&albums);
        let liked_set: std::collections::HashSet<&str> =
            collections.liked.iter().map(String::as_str).collect();
        let smart_list = collections.smart.as_deref().unwrap_or(&[]);
        let smart_matches = smart_list
            .first()
            .map(|sp| {
                SmartPlaylists::evaluate(
                    sp,
                    albums
                        .iter()
                        .flat_map(|a| a.tracks.iter().map(move |t| (t, a))),
                    |id| stats.get(id),
                    |id| liked_set.contains(id),
                    0,
                )
            })
            .unwrap_or_default();

        // Check cover hashes match actual cover files on disk when present.
        let mut matched_covers = 0usize;
        for album in &albums {
            if CoverPaths::thumb_path(&album.name).exists()
                || CoverPaths::full_path(&album.name).exists()
            {
                matched_covers += 1;
            }
        }
        assert!(matched_covers > 0, "expected cover files matching hash_name");

        println!(
            "Loaded: albums={}, playlists={}, smart_playlists={}, smart_first_matches={}, queue={}, stats={}, matched_covers={}",
            albums.len(),
            collections.playlists.len(),
            smart_list.len(),
            smart_matches.len(),
            session_queue.len(),
            stats.tracks.len(),
            matched_covers,
        );
    }
}
