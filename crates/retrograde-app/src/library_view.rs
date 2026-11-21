use std::cmp::Reverse;
use std::collections::HashMap;

use retrograde_core::{
    Album, ArtistSummary, CollectionsData, LIBRARY_SORT_KEYS, PlayStats, SmartPlaylists,
    canonical_library_sort,
};

/// The library sort of the original (`MainViewModel.SortOptions`): the key it stores and the menu label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LibrarySort {
    #[default]
    AddedDesc,
    AddedAsc,
    NameAsc,
    NameDesc,
    ArtistAsc,
    ArtistDesc,
    YearDesc,
    YearAsc,
    PlayedDesc,
    PlaysDesc,
}

impl LibrarySort {
    /// Menu order, as the C# build lists them.
    pub const ALL: [Self; 10] = [
        Self::AddedDesc,
        Self::AddedAsc,
        Self::NameAsc,
        Self::NameDesc,
        Self::ArtistAsc,
        Self::ArtistDesc,
        Self::YearDesc,
        Self::YearAsc,
        Self::PlayedDesc,
        Self::PlaysDesc,
    ];

    pub fn as_str(self) -> &'static str {
        LIBRARY_SORT_KEYS[Self::ALL.iter().position(|s| *s == self).unwrap_or(0)]
    }

    /// Reads a stored key (old Rust keys and unknown values included, see `canonical_library_sort`).
    pub fn from_key(s: &str) -> Self {
        let key = canonical_library_sort(s);
        Self::ALL
            .iter()
            .copied()
            .find(|v| v.as_str() == key)
            .unwrap_or_default()
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::AddedDesc => "RECENTLY ADDED",
            Self::AddedAsc => "OLDEST ADDED",
            Self::NameAsc => "NAME [A-Z]",
            Self::NameDesc => "NAME [Z-A]",
            Self::ArtistAsc => "ARTIST [A-Z]",
            Self::ArtistDesc => "ARTIST [Z-A]",
            Self::YearDesc => "NEWEST",
            Self::YearAsc => "OLDEST",
            Self::PlayedDesc => "RECENTLY PLAYED",
            Self::PlaysDesc => "MOST PLAYED",
        }
    }

    pub fn uses_stats(self) -> bool {
        matches!(self, Self::PlayedDesc | Self::PlaysDesc)
    }
}

/// Case-insensitive text key (Unicode lowercase), used for every comparison and match in the library.
pub fn fold(s: &str) -> String {
    s.to_lowercase()
}

/// The artist key an artist page is opened with.
pub fn artist_key(name: &str) -> String {
    let trimmed = name.trim();
    fold(if trimmed.is_empty() { "Unknown Artist" } else { trimmed })
}

/// Last play time and total plays of an album's tracks.
fn album_plays(album: &Album, stats: &PlayStats) -> (i64, i64) {
    let mut last = 0;
    let mut count = 0;
    for t in &album.tracks {
        if let Some(s) = stats.get(&t.id) {
            last = last.max(s.last);
            count += i64::from(s.count);
        }
    }
    (last, count)
}

/// The library grid order: every album, sorted like the original's `RebuildLibrary`. The sort is stable, so
/// albums that tie keep the order they were loaded in.
pub fn sort_albums(albums: &[Album], sort: LibrarySort, stats: &PlayStats) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..albums.len()).collect();
    match sort {
        LibrarySort::AddedDesc => idx.sort_by_key(|&i| Reverse(albums[i].added_at)),
        LibrarySort::AddedAsc => idx.sort_by_key(|&i| albums[i].added_at),
        LibrarySort::NameAsc => idx.sort_by_cached_key(|&i| fold(&albums[i].name)),
        LibrarySort::NameDesc => idx.sort_by_cached_key(|&i| Reverse(fold(&albums[i].name))),
        LibrarySort::ArtistAsc => idx.sort_by_cached_key(|&i| fold(&albums[i].artist)),
        LibrarySort::ArtistDesc => idx.sort_by_cached_key(|&i| Reverse(fold(&albums[i].artist))),
        LibrarySort::YearDesc => idx.sort_by_key(|&i| Reverse(albums[i].year)),
        LibrarySort::YearAsc => idx.sort_by_key(|&i| albums[i].year),
        // Albums never played keep their added order at the end
        LibrarySort::PlayedDesc => idx.sort_by_cached_key(|&i| {
            let (last, _) = album_plays(&albums[i], stats);
            (Reverse(last), Reverse(albums[i].added_at))
        }),
        LibrarySort::PlaysDesc => idx.sort_by_cached_key(|&i| {
            let (last, count) = album_plays(&albums[i], stats);
            (Reverse(count), Reverse(last), Reverse(albums[i].added_at))
        }),
    }
    idx
}

/// The artists in the library sort (`SortedArtists` of the original): names alphabetically, dates by the
/// artist's newest album.
pub fn sort_artists(artists: &[ArtistSummary], sort: LibrarySort, stats: &PlayStats) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..artists.len()).collect();
    let plays = |a: &ArtistSummary| {
        a.albums.iter().map(|al| album_plays(al, stats)).fold((0i64, 0i64), |(l, c), (al, ac)| (l.max(al), c + ac))
    };
    match sort {
        LibrarySort::NameAsc | LibrarySort::ArtistAsc => idx.sort_by_cached_key(|&i| fold(&artists[i].name)),
        LibrarySort::NameDesc | LibrarySort::ArtistDesc => {
            idx.sort_by_cached_key(|&i| Reverse(fold(&artists[i].name)))
        }
        LibrarySort::AddedAsc => {
            idx.sort_by_key(|&i| artists[i].albums.iter().map(|a| a.added_at).min().unwrap_or(0))
        }
        LibrarySort::YearDesc => {
            idx.sort_by_key(|&i| Reverse(artists[i].albums.iter().map(|a| a.year).max().unwrap_or(0)))
        }
        LibrarySort::YearAsc => idx.sort_by_key(|&i| {
            artists[i].albums.iter().map(|a| a.year).filter(|y| *y > 0).min().unwrap_or(i32::MAX)
        }),
        LibrarySort::PlayedDesc => {
            idx.sort_by_cached_key(|&i| (Reverse(plays(&artists[i]).0), fold(&artists[i].name)))
        }
        LibrarySort::PlaysDesc => {
            idx.sort_by_cached_key(|&i| (Reverse(plays(&artists[i]).1), fold(&artists[i].name)))
        }
        LibrarySort::AddedDesc => idx.sort_by_key(|&i| Reverse(artists[i].newest_added_at)),
    }
    idx
}

/// Groups the albums by album artist, in the order the artists first appear in the library. Each artist's
/// albums are newest first (year, then added).
pub fn group_artists(albums: &[Album]) -> Vec<ArtistSummary> {
    let mut by_key: HashMap<String, usize> = HashMap::new();
    let mut artists: Vec<ArtistSummary> = Vec::new();
    for album in albums {
        let display_name = if album.artist.trim().is_empty() {
            "Unknown Artist".to_owned()
        } else {
            album.artist.trim().to_owned()
        };
        let i = *by_key.entry(fold(&display_name)).or_insert_with(|| {
            artists.push(ArtistSummary {
                name: display_name,
                albums: Vec::new(),
                track_count: 0,
                newest_added_at: 0,
            });
            artists.len() - 1
        });
        let entry = &mut artists[i];
        entry.track_count += album.tracks.len();
        entry.newest_added_at = entry.newest_added_at.max(album.added_at);
        entry.albums.push(album.clone());
    }
    for artist in &mut artists {
        artist
            .albums
            .sort_by_key(|a| (Reverse(a.year), Reverse(a.added_at)));
    }
    artists
}

/// SEARCH, ALBUMS: albums whose name or artist contains the query (already folded).
pub fn search_albums(albums: &[Album], q: &str) -> Vec<usize> {
    albums
        .iter()
        .enumerate()
        .filter(|(_, a)| fold(&a.name).contains(q) || fold(&a.artist).contains(q))
        .map(|(i, _)| i)
        .collect()
}

/// SEARCH, ARTISTS: artists whose name contains the query (already folded), by name.
pub fn search_artists(artists: &[ArtistSummary], q: &str) -> Vec<usize> {
    let mut idx: Vec<usize> = artists
        .iter()
        .enumerate()
        .filter(|(_, a)| fold(&a.name).contains(q))
        .map(|(i, _)| i)
        .collect();
    idx.sort_by_cached_key(|&i| fold(&artists[i].name));
    idx
}

pub fn ensure_default_smart_playlists(collections: &mut CollectionsData) -> bool {
    if collections.smart.is_none() {
        collections.smart = Some(SmartPlaylists::examples());
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use retrograde_core::Track;

    fn mk_album(name: &str, artist: &str, year: i32, added_at: i64, track_title: &str) -> Album {
        Album {
            name: name.to_owned(),
            artist: artist.to_owned(),
            cover_thumb_path: None,
            cover_full_path: None,
            year,
            added_at,
            tracks: vec![Track {
                id: format!("/m/{name}/01.flac"),
                title: track_title.to_owned(),
                artist: artist.to_owned(),
                album: name.to_owned(),
                duration: 180.0,
                file_path: format!("/m/{name}/01.flac"),
                track_number: 1,
                disc_number: 1,
                codec: "FLAC".to_owned(),
                sample_rate: 44100,
                bits_per_sample: 16,
                channels: 2,
                cover_thumb_path: None,
                cover_full_path: None,
            }],
        }
    }

    #[test]
    fn sorts_searches_and_groups_artists() {
        let albums = vec![
            mk_album("Zebra", "Ado", 2024, 10, "Show"),
            mk_album("alpha", "ado", 2022, 30, "Usseewa"),
            mk_album("Jazz", "Miles", 1959, 20, "So What"),
        ];
        let stats = PlayStats::default();

        assert_eq!(sort_albums(&albums, LibrarySort::AddedDesc, &stats), vec![1, 2, 0]);
        assert_eq!(sort_albums(&albums, LibrarySort::NameAsc, &stats), vec![1, 2, 0]);
        assert_eq!(sort_albums(&albums, LibrarySort::NameDesc, &stats), vec![0, 2, 1]);
        assert_eq!(search_albums(&albums, "jaz"), vec![2]);

        let artists = group_artists(&albums);
        assert_eq!(artists.len(), 2);
        assert_eq!(artists[0].name, "Ado");
        assert_eq!(artists[0].track_count, 2);
        assert_eq!(artists[0].newest_added_at, 30);
        assert_eq!(search_artists(&artists, "miles"), vec![1]);
        assert_eq!(sort_artists(&artists, LibrarySort::NameDesc, &stats), vec![1, 0]);
    }

    #[test]
    fn ties_keep_load_order() {
        let albums = vec![
            mk_album("C", "X", 2000, 5, "a"),
            mk_album("A", "Y", 2000, 5, "b"),
            mk_album("B", "Z", 2000, 5, "c"),
        ];
        let stats = PlayStats::default();
        for sort in LibrarySort::ALL {
            if matches!(sort, LibrarySort::NameAsc | LibrarySort::NameDesc | LibrarySort::ArtistAsc | LibrarySort::ArtistDesc) {
                continue;
            }
            assert_eq!(sort_albums(&albums, sort, &stats), vec![0, 1, 2], "{sort:?}");
        }
    }

    #[test]
    fn unicode_case_folds_and_sort_keys_roundtrip() {
        let albums = vec![mk_album("Élan", "Äther", 2001, 1, "x"), mk_album("b", "äther", 2002, 2, "y")];
        assert_eq!(group_artists(&albums).len(), 1);
        assert_eq!(search_albums(&albums, &fold("ÉLAN")), vec![0]);
        for s in LibrarySort::ALL {
            assert_eq!(LibrarySort::from_key(s.as_str()), s);
        }
        assert_eq!(LibrarySort::from_key("title_asc"), LibrarySort::NameAsc);
        assert_eq!(LibrarySort::from_key("bogus"), LibrarySort::AddedDesc);
    }

    #[test]
    fn initializes_default_smart_playlists_only_once() {
        let mut c = CollectionsData::default();
        assert!(ensure_default_smart_playlists(&mut c));
        assert_eq!(c.smart.as_ref().unwrap().len(), 4);
        assert!(!ensure_default_smart_playlists(&mut c));
    }
}
