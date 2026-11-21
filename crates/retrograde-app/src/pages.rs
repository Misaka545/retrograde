//! The pages of the original app that share one look: the detail page (album, artist, playlist, favorites),
//! the card grids (albums, artists, search results), the search page and the new-playlist modal.

use iced::widget::text::Wrapping;
use iced::widget::{
    button, column, container, grid, hover, responsive, row, scrollable, space, stack, text,
    text_input,
};
use iced::{Alignment, Background, Border, Color, Element, Length, Padding, Radians, Task};
use retrograde_core::{CoverPaths, PlaylistCovers, Track};

use crate::context_menu;
use crate::covers_ui::render_cover;
use crate::icons;
use crate::theme;
use crate::views::bold;
use crate::{App, Message, PlaylistView, SearchTab, Tab, format_mm_ss};

/// What a detail page lists. Messages carry this instead of the tracks, so a row does not copy the whole list.
#[derive(Debug, Clone, PartialEq)]
pub enum DetailCtx {
    Album(crate::AlbumKey),
    Artist(String),
    Playlist(PlaylistView),
}

/// A detail page, as remembered by the page history (BACK).
#[derive(Debug, Clone, PartialEq)]
pub enum Page {
    Album(crate::AlbumKey),
    Artist(String),
    Playlist(PlaylistView),
}

const MAX_HISTORY: usize = 30;

impl App {
    /// The tab on screen under the full-screen player.
    pub fn shown_tab(&self) -> Tab {
        if self.active_tab == Tab::NowPlaying { self.previous_tab } else { self.active_tab }
    }

    /// The detail page on screen, if any.
    pub fn current_page(&self) -> Option<Page> {
        match self.shown_tab() {
            Tab::Library => match self.library_mode {
                crate::LibraryMode::Albums if self.album_detail_open => self.selected_album.clone().map(Page::Album),
                crate::LibraryMode::Artists if self.artist_detail_open => self.selected_artist.clone().map(Page::Artist),
                _ => None,
            },
            Tab::Playlists if self.smart_draft.is_none() => Some(Page::Playlist(self.playlist_view)),
            _ => None,
        }
    }

    /// A detail page is about to open: one opened from another page remembers that page for BACK, the first
    /// one remembers where it was opened from (SEARCH or LIBRARY).
    pub fn enter_page(&mut self, next: &Page) {
        match self.current_page() {
            Some(cur) if cur != *next => {
                self.page_stack.push(cur);
                if self.page_stack.len() > MAX_HISTORY {
                    self.page_stack.remove(0);
                }
            }
            Some(_) => {}
            None => {
                self.page_stack.clear();
                self.view_before_detail = if self.shown_tab() == Tab::Search { Tab::Search } else { Tab::Library };
            }
        }
    }

    /// The label of the back button: BACK while pages were opened from pages, else where they started.
    pub fn back_label(&self) -> &'static str {
        if !self.page_stack.is_empty() {
            "BACK"
        } else if self.view_before_detail == Tab::Search {
            "SEARCH"
        } else {
            "LIBRARY"
        }
    }

    /// One page back, or out of the pages to where they were opened from.
    pub fn go_back(&mut self) -> Task<Message> {
        if let Some(page) = self.page_stack.pop() {
            return match page {
                Page::Album(key) => self.open_album(key),
                Page::Artist(key) => self.open_artist(key),
                Page::Playlist(view) => self.open_playlist(view),
            };
        }
        self.album_detail_open = false;
        self.artist_detail_open = false;
        self.detail_scroll_y = 0.0;
        let to = self.view_before_detail;
        if self.active_tab == Tab::NowPlaying {
            self.previous_tab = to;
        } else {
            self.active_tab = to;
        }
        Task::none()
    }

    fn show_tab(&mut self, tab: Tab) {
        if self.active_tab != tab {
            if self.active_tab != Tab::NowPlaying {
                self.previous_tab = self.active_tab;
            }
            self.active_tab = tab;
        }
        self.tag_draft = None;
    }

    pub fn open_album(&mut self, key: crate::AlbumKey) -> Task<Message> {
        let mut cover = None;
        if let Some(album) = self.find_album(&key) {
            let full = album.cover_full_path.clone();
            let thumb = album.cover_thumb_path.clone();
            cover = thumb.clone().or(full.clone());
            if let Some(p) = full {
                self.covers.ensure_full(&p);
            }
            if let Some(p) = thumb {
                self.covers.ensure_thumb(&p);
            }
        }
        self.detail_scroll_y = 0.0;
        let tint = self.request_tint(format!("album:{}\u{0}{}", key.name, key.artist), cover);
        self.selected_album = Some(key);
        self.album_detail_open = true;
        // An album picked anywhere (sidebar, search, artist page) opens on the library's album page
        self.library_mode = crate::LibraryMode::Albums;
        self.artist_detail_open = false;
        self.show_tab(Tab::Library);
        tint
    }

    pub fn open_artist(&mut self, key: String) -> Task<Message> {
        self.detail_scroll_y = 0.0;
        let cover = self.find_artist(&key).and_then(|a| {
            a.albums.iter().find_map(|al| al.cover_thumb_path.clone().or(al.cover_full_path.clone()))
        });
        let tint = self.request_tint(format!("artist:{key}"), cover);
        self.selected_artist = Some(key);
        self.artist_detail_open = true;
        self.library_mode = crate::LibraryMode::Artists;
        self.album_detail_open = false;
        self.show_tab(Tab::Library);
        tint
    }

    pub fn open_playlist(&mut self, view: PlaylistView) -> Task<Message> {
        self.detail_scroll_y = 0.0;
        self.show_tab(Tab::Playlists);
        let (tint_key, cover) = match view {
            PlaylistView::Liked => ("liked".to_owned(), None),
            PlaylistView::Custom(id) => {
                let custom = self
                    .collections
                    .playlists
                    .iter()
                    .find(|p| p.id == id)
                    .and_then(|p| CoverPaths::to_local_path(p.cover_art.as_deref()));
                if let Some(ref c) = custom {
                    let thumb = PlaylistCovers::thumb_for(c);
                    self.covers.ensure_full(c);
                    if thumb != *c {
                        self.covers.ensure_thumb(&thumb);
                    }
                }
                let fallback = custom.or_else(|| {
                    self.collections
                        .playlists
                        .iter()
                        .find(|p| p.id == id)
                        .and_then(|p| p.track_ids.first())
                        .and_then(|tid| self.lookup_track(tid))
                        .and_then(|t| t.cover_thumb_path.clone().or(t.cover_full_path.clone()))
                });
                (format!("pl:{id}"), fallback)
            }
            PlaylistView::Smart(id) => (format!("smart:{id}"), None),
        };
        let tint = self.request_tint(tint_key, cover);
        Task::batch([tint, Task::done(Message::SelectPlaylistView(view))])
    }

    /// Works out the header color of a page from its cover off the UI thread.
    fn request_tint(&mut self, key: String, cover: Option<std::path::PathBuf>) -> Task<Message> {
        if self.tint_for.as_deref() == Some(key.as_str()) {
            return Task::none();
        }
        self.tint_for = Some(key.clone());
        self.detail_tint = None;
        let Some(path) = cover else { return Task::none() };
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    retrograde_core::extract_cover_palette(&path).map(|p| theme::color_from_rgb8(p.glow))
                })
                .await
                .ok()
                .flatten()
            },
            move |c| Message::DetailTintReady(key.clone(), c),
        )
    }

    /// The tracks a detail page shows, in order.
    pub fn detail_tracks(&self, ctx: &DetailCtx) -> Vec<Track> {
        match ctx {
            DetailCtx::Album(key) => self.find_album(key).map(|a| a.tracks.clone()).unwrap_or_default(),
            // PLAY on an artist: albums oldest first (undated last), as the original's
            DetailCtx::Artist(key) => self
                .find_artist(key)
                .map(|a| {
                    let mut albums: Vec<&retrograde_core::Album> = a.albums.iter().collect();
                    albums.sort_by_key(|al| (if al.year == 0 { i32::MAX } else { al.year }, al.added_at));
                    albums.iter().flat_map(|al| al.tracks.iter().cloned()).collect()
                })
                .unwrap_or_default(),
            DetailCtx::Playlist(PlaylistView::Liked) => self
                .collections
                .liked
                .iter()
                .filter_map(|id| self.lookup_track(id).cloned())
                .collect(),
            DetailCtx::Playlist(PlaylistView::Custom(id)) => self
                .collections
                .playlists
                .iter()
                .find(|p| p.id == *id)
                .map(|p| p.track_ids.iter().filter_map(|t| self.lookup_track(t).cloned()).collect())
                .unwrap_or_default(),
            DetailCtx::Playlist(PlaylistView::Smart(_)) => self.cached_smart_tracks.clone(),
        }
    }
}

/// Width-aware shortening: cuts the text with an ellipsis so it fits `max_px` at `size`, counting CJK glyphs wide.
pub fn fit(s: &str, max_px: f32, size: f32) -> String {
    let w = |c: char| -> f32 {
        if (c as u32) >= 0x2E80 {
            size
        } else if c.is_uppercase() || c.is_ascii_digit() {
            size * 0.68
        } else if c == ' ' || c == 'i' || c == 'l' || c == 'j' || c == '.' || c == ',' {
            size * 0.32
        } else {
            size * 0.58
        }
    };
    let total: f32 = s.chars().map(w).sum();
    if total <= max_px {
        return s.to_owned();
    }
    let mut out = String::new();
    let mut acc = 0.0;
    let ell = size * 0.6;
    for c in s.chars() {
        if acc + w(c) + ell > max_px {
            break;
        }
        acc += w(c);
        out.push(c);
    }
    out.push('…');
    out
}

fn badge<'a>(label: &'a str) -> Element<'a, Message> {
    container(text(label).size(8).color(theme::TEXT_DIM))
        .padding([2, 4])
        .style(theme::fill_style(theme::LINE_SOFT))
        .into()
}

/// Height of a card's padding and its two text lines under the cover.
const CARD_TEXT_H: f32 = 70.0;

/// Where a card grid sits: the space around it differs.
#[derive(Debug, Clone, Copy)]
pub enum GridArea {
    Library,
    Search,
    Detail,
}

impl App {
    /// Columns and card height of the card grids, from the window width with the original's breakpoints
    /// (`MainViewModel.UpdateColumns`): at most 5 columns beside the sidebar, 7 without it. A card is as tall as
    /// its cover plus the two lines under it.
    pub fn grid_layout(&self, area: GridArea) -> (usize, f32) {
        let w = self.window_size.width;
        let expanded = self.sidebar_width >= 150.0;
        let sidebar_footprint = self.sidebar_width + 12.0;
        let content = w - 24.0 - sidebar_footprint;
        let mut columns = if content >= 1000.0 {
            5
        } else if content >= 744.0 {
            4
        } else if content >= 488.0 {
            3
        } else {
            2
        };
        if !expanded {
            columns = if content >= 1512.0 { 7 } else if content >= 1256.0 { 6 } else { columns };
        }
        // The grid's own width: window, outer margins, sidebar, page padding, scroll gutter and bar
        let around = match area {
            GridArea::Library => 32.0 + 14.0 + 10.0,
            GridArea::Search => 32.0 + 14.0 + 10.0,
            GridArea::Detail => 64.0 + 10.0,
        };
        let grid_w = w - 24.0 - sidebar_footprint - around;
        let card_w = ((grid_w - 12.0 * (columns as f32 - 1.0)) / columns as f32).max(80.0);
        (columns, (card_w - 24.0 + CARD_TEXT_H).floor())
    }
}

/// One grid card of the original: cover, a kind badge with the name on one line, a small caption under it.
/// Every card is `card_h` tall: the cover is as large as fits the card's width and that height.
#[allow(clippy::too_many_arguments)]
pub fn media_card<'a>(
    kind: &'a str,
    title: String,
    caption: String,
    cover: Option<&'a iced::widget::image::Handle>,
    mono: &str,
    card_h: f32,
    accent: Color,
    msg: Message,
) -> Element<'a, Message> {
    media_card_opts(kind, title, caption, cover, mono, card_h, accent, msg, None)
}

#[allow(clippy::too_many_arguments)]
pub fn media_card_opts<'a>(
    kind: &'a str,
    title: String,
    caption: String,
    cover: Option<&'a iced::widget::image::Handle>,
    mono: &str,
    card_h: f32,
    accent: Color,
    msg: Message,
    selectable_state: Option<bool>,
) -> Element<'a, Message> {
    let selected = selectable_state.unwrap_or(false);
    let mono = mono.to_owned();
    let cover = cover.cloned();
    let inner = responsive(move |size| {
        let side = (size.width - 24.0).min(card_h - CARD_TEXT_H).max(60.0);
        let name = fit(&title, side - 40.0, 13.0);
        let cap = fit(&caption, side, 10.0);
        let card_col = column![
            render_cover(cover.as_ref(), side, 0.0, accent, &mono),
            row![
                badge(kind_static(kind)),
                text(name).size(13).font(bold()).color(theme::TEXT).wrapping(Wrapping::None),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            text(cap).size(10).color(theme::TEXT_MUTE).wrapping(Wrapping::None),
        ]
        .spacing(8)
        .padding(12);

        let corner_tl = container(
            column![
                container(space::horizontal()).width(8).height(1).style(theme::fill_style(if selected { theme::ORANGE } else { theme::LINE })),
                container(space::horizontal()).width(1).height(7).style(theme::fill_style(if selected { theme::ORANGE } else { theme::LINE })),
            ],
        );

        let mut layers: Vec<Element<'_, Message>> = vec![card_col.into(), corner_tl.into()];
        if let Some(is_sel) = selectable_state {
            let check = container(icons::icon(
                if is_sel { "checksquare" } else { "square" },
                18.0,
                if is_sel { theme::ORANGE } else { theme::TEXT_DIM },
            ))
            .width(Length::Fill)
            .align_x(Alignment::End)
            .padding(8);
            layers.push(check.into());
        }
        stack(layers).width(Length::Fill).height(Length::Fill).into()
    });
    button(inner)
        .style(theme::album_card_style(selected, accent))
        .width(Length::Fill)
        .height(Length::Fixed(card_h))
        .padding(0)
        .on_press(msg)
        .into()
}

fn kind_static(k: &str) -> &'static str {
    match k {
        "ALB" => "ALB",
        "ART" => "ART",
        "TRK" => "TRK",
        _ => "PL",
    }
}

pub fn card_grid<'a>(cards: Vec<Element<'a, Message>>, columns: usize) -> Element<'a, Message> {
    grid(cards).spacing(12).columns(columns.max(1)).height(Length::Shrink).into()
}

/// Computes `[start..end)` visible row indices for a uniformly spaced vertical list or grid.
pub fn visible_row_range(
    scroll_y: f32,
    viewport_h: f32,
    row_h: f32,
    total_rows: usize,
    overscan: usize,
) -> std::ops::Range<usize> {
    if total_rows == 0 || row_h <= 0.0 {
        return 0..0;
    }
    let y0 = scroll_y.max(0.0);
    let y1 = y0 + viewport_h.max(0.0);
    let first = ((y0 / row_h).floor() as usize).saturating_sub(overscan);
    let last = ((y1 / row_h).ceil() as usize).saturating_add(overscan).min(total_rows);
    first.min(last)..last
}

/// The artists of the library as cards (the ARTISTS chip of the library page).
pub fn artists_grid<'a>(app: &'a App, accent: Color) -> Element<'a, Message> {
    let total = app.library_artists.len();
    if total == 0 {
        return empty_state("DATABASE_EMPTY", "USE ADD MUSIC TO SCAN A FOLDER");
    }
    let (columns, card_h) = app.grid_layout(GridArea::Library);
    let cols = columns.max(1);
    let row_stride = card_h + 12.0;
    let total_rows = total.div_ceil(cols);
    let range = visible_row_range(
        app.lib_scroll_y,
        (app.window_size.height - 160.0).max(200.0),
        row_stride,
        total_rows,
        2,
    );
    let start_idx = range.start * cols;
    let end_idx = (range.end * cols).min(total);
    let cards: Vec<Element<'a, Message>> = app.library_artists[start_idx..end_idx]
        .iter()
        .filter_map(|&i| app.artists.get(i))
        .map(|ar| {
            // The newest album that has a cover
            let cover = ar.albums.iter().find_map(|a| {
                app.covers.get_first(&[a.cover_thumb_path.as_deref(), a.cover_full_path.as_deref()])
            });
            media_card(
                "ART",
                ar.name.clone(),
                artist_caption(ar),
                cover,
                &ar.name,
                card_h,
                accent,
                Message::OpenArtist(crate::library_view::artist_key(&ar.name)),
            )
        })
        .collect();
    let top_h = range.start as f32 * row_stride;
    let bot_h = (total_rows.saturating_sub(range.end) as f32 * row_stride - 12.0).max(0.0);
    let mut stack_col = column![];
    if top_h > 0.0 {
        stack_col = stack_col.push(space::vertical().height(top_h));
    }
    stack_col = stack_col.push(card_grid(cards, cols));
    if bot_h > 0.0 {
        stack_col = stack_col.push(space::vertical().height(bot_h));
    }
    scrollable(container(stack_col).padding(Padding::ZERO.right(14.0)))
        .on_scroll(|vp| Message::LibScrolled(vp.absolute_offset().y))
        .height(Length::Fill)
        .into()
}

pub fn empty_state<'a>(title: &'a str, sub: &'a str) -> Element<'a, Message> {
    container(
        column![
            text(title).size(24).font(bold()).color(Color::WHITE),
            text(sub).size(12).color(theme::TEXT_MUTE),
        ]
        .spacing(8)
        .align_x(Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(Alignment::Center)
    .align_y(Alignment::Center)
    .into()
}

/// C# `Database Empty` box (`256 px` tall bordered container with folderplus icon + `INITIATE_SCAN`).
pub fn empty_library_box(hint: &str) -> Element<'_, Message> {
    let box_inner = column![
        icons::icon("folderplus", 48.0, theme::LINE),
        text("DATABASE_EMPTY").size(18).font(bold()).color(theme::TEXT_MUTE),
        text(hint.to_owned()).size(11).color(theme::TEXT_MUTE),
        button(text("INITIATE_SCAN").size(10).font(bold()))
            .padding([8, 20])
            .style(|_t, s| {
                let hover = matches!(s, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: Some(Background::Color(if hover {
                        theme::ORANGE
                    } else {
                        theme::with_alpha(theme::ORANGE, 0.14)
                    })),
                    text_color: if hover { Color::BLACK } else { theme::ORANGE },
                    border: Border {
                        color: theme::ORANGE,
                        width: 1.0,
                        radius: 0.0.into(),
                    },
                    ..Default::default()
                }
            })
            .on_press(Message::PickFolders),
    ]
    .spacing(14)
    .align_x(Alignment::Center);

    container(
        container(box_inner)
            .width(Length::Fill)
            .height(256)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .style(|_t| container::Style {
                background: Some(Background::Color(theme::with_alpha(theme::PANEL, 0.5))),
                border: Border {
                    color: theme::LINE,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..Default::default()
            }),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(Padding { top: 48.0, right: 24.0, bottom: 0.0, left: 24.0 })
    .align_y(Alignment::Start)
    .into()
}

/// Everything the detail page needs besides the track list.
pub struct DetailSpec<'a> {
    pub ctx: DetailCtx,
    pub chip: &'a str,
    pub title: String,
    pub artist: Option<String>,
    pub year: Option<i32>,
    pub quality: Option<String>,
    pub cover: Option<&'a iced::widget::image::Handle>,
    pub tint: Color,
    pub show_cover_col: bool,
    pub extra: Option<Element<'a, Message>>,
    /// The artist line opens this artist page (album pages).
    pub artist_link: Option<String>,
    /// Replaces the track table (the artist page: most played and the albums).
    pub body: Option<Element<'a, Message>>,
    /// A bold line under the title instead of artist / year / quality (the artist page's "40 ALBUMS").
    pub meta_text: Option<String>,
    /// Render the violet gradient + white heart cover for Liked Songs.
    pub is_liked_cover: bool,
    /// When set, hovering the cover reveals `CHANGE_IMG` to pick a cover for that playlist.
    pub change_cover_playlist: Option<i64>,
}

fn mini_eq_glyph<'a>(color: Color) -> Element<'a, Message> {
    row![
        container(space::horizontal()).width(2).height(7).style(theme::fill_style(color)),
        container(space::horizontal()).width(2).height(12).style(theme::fill_style(color)),
        container(space::horizontal()).width(2).height(9).style(theme::fill_style(color)),
    ]
    .spacing(2)
    .align_y(Alignment::End)
    .into()
}

fn disc_header<'a>(disc: i32) -> Element<'a, Message> {
    row![
        icons::icon("disc", 16.0, Color::from_rgb8(0x6B, 0x69, 0x63)),
        text(format!("DISC {disc}")).size(13).font(bold()).color(Color::from_rgb8(0xCF, 0xCC, 0xC4)),
        container(space::horizontal()).width(Length::Fill).height(1).style(theme::fill_style(Color::from_rgb8(0x27, 0x27, 0x2B))),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .padding(Padding { top: 18.0, right: 0.0, bottom: 8.0, left: 0.0 })
    .into()
}

#[allow(clippy::too_many_arguments)]
fn track_row<'a>(
    app: &'a App,
    track: Track,
    number: usize,
    ctx: &DetailCtx,
    show_cover: bool,
    removable_playlist: Option<i64>,
    play: Message,
    accent: Color,
) -> Element<'a, Message> {
    let active = app.current_track.as_ref().is_some_and(|cur| cur.id == track.id);
    let is_liked = app.liked_set.contains(&track.id);
    let (album_artist, album_year) = match ctx {
        DetailCtx::Album(k) => app.find_album(k).map(|a| (a.artist.clone(), a.year)).unwrap_or_default(),
        _ => (String::new(), 0),
    };

    let number_cell: Element<'a, Message> = if active {
        mini_eq_glyph(accent)
    } else {
        text(format!("{number:02}")).size(15).color(theme::TEXT_MUTE).into()
    };

    let edge = container(space::horizontal())
        .width(2)
        .height(34)
        .style(theme::fill_style(if active { accent } else { Color::TRANSPARENT }));

    let mut body = row![
        edge,
        container(number_cell).width(46).align_x(Alignment::Center),
    ]
    .spacing(12)
    .align_y(Alignment::Center);

    if show_cover {
        let cover = app.covers.get_first(&[track.cover_thumb_path.as_deref(), track.cover_full_path.as_deref()]);
        body = body.push(render_cover(cover, 32.0, 0.0, accent, &track.title));
    }

    let track_id_like = track.id.clone();
    let track_for_btn = track.clone();
    let aa_for_btn = album_artist.clone();
    let track_for_rclick = track.clone();
    let aa_for_rclick = album_artist;

    let heart_slot: Element<'a, Message> = if is_liked {
        button(icons::icon("heart", 14.0, theme::ORANGE))
            .padding(5)
            .style(theme::bare_button_style())
            .on_press(Message::ToggleLike(track_id_like.clone()))
            .into()
    } else {
        space::horizontal().width(24).into()
    };

    body = body
        .push(
            column![
                text(fit(&track.title, 760.0, 14.0))
                    .size(14)
                    .font(bold())
                    .color(if active { accent } else { theme::TEXT })
                    .wrapping(Wrapping::None),
                text(fit(&track.artist.to_uppercase(), 760.0, 10.0))
                    .size(10)
                    .color(theme::TEXT_MUTE)
                    .wrapping(Wrapping::None),
            ]
            .spacing(2)
            .width(Length::Fill),
        )
        .push(container(heart_slot).width(30).align_x(Alignment::Center))
        .push(space::horizontal().width(34))
        .push(
            container(text(format_mm_ss(track.duration)).size(12).color(theme::TEXT_MUTE))
                .width(72)
                .align_x(Alignment::Center),
        );

    let base_btn = button(container(body).padding([9, 0]))
        .width(Length::Fill)
        .padding(0)
        .style(theme::nav_item_style(active, accent))
        .on_press(play);

    let hover_controls = container(
        row![
            button(icons::icon("heart", 14.0, if is_liked { theme::ORANGE } else { theme::TEXT_DIM }))
                .padding(5)
                .style(theme::bare_button_style())
                .on_press(Message::ToggleLike(track_id_like)),
            context_menu::left_trigger(
                button(icons::icon("more", 16.0, theme::TEXT_DIM))
                    .padding(5)
                    .style(theme::bare_button_style())
                    .on_press(Message::Noop),
                move |pos| Message::OpenTrackMenu {
                    track: track_for_btn.clone(),
                    album_artist: aa_for_btn.clone(),
                    album_year,
                    removable_playlist,
                    pos,
                },
            ),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(Alignment::End)
    .align_y(Alignment::Center)
    .padding(Padding::ZERO.right(76.0));

    let row_hovered = hover(base_btn, hover_controls);
    context_menu::trigger(row_hovered, move |pos| Message::OpenTrackMenu {
        track: track_for_rclick.clone(),
        album_artist: aa_for_rclick.clone(),
        album_year,
        removable_playlist,
        pos,
    })
}

/// The page of the original for one album, artist or playlist: a header tinted with the cover color that fades
/// into the panel, the framed cover, the big title with the three-color rule, play and like, and the OPERATION_LOG table.
pub fn detail_page<'a>(app: &'a App, spec: DetailSpec<'a>, accent: Color) -> Element<'a, Message> {
    let tracks = app.detail_tracks(&spec.ctx);
    let tint = spec.tint;
    let name_for_cover = spec.title.clone();

    let frame_offset = container(space::horizontal()).width(224).height(224).style(|_t| container::Style {
        border: Border { color: Color::from_rgb8(0x46, 0x46, 0x4C), width: 1.0, radius: 0.0.into() },
        ..Default::default()
    });
    let cover_inner: Element<'a, Message> = if spec.is_liked_cover {
        container(icons::icon("heart", 72.0, Color::WHITE))
            .width(222)
            .height(222)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .style(|_t| container::Style {
                background: Some(Background::Gradient(
                    iced::gradient::Linear::new(Radians(std::f32::consts::FRAC_PI_4 * 3.0))
                        .add_stop(0.0, Color::from_rgb8(0x45, 0x0A, 0xF5))
                        .add_stop(0.6, Color::from_rgb8(0x8E, 0x8E, 0xE5))
                        .add_stop(1.0, Color::from_rgb8(0xC4, 0xEF, 0xD9))
                        .into(),
                )),
                ..Default::default()
            })
            .into()
    } else if let Some(pid) = spec.change_cover_playlist {
        let base_cov = render_cover(spec.cover, 222.0, 0.0, accent, &name_for_cover);
        let overlay_btn = button(
            container(
                column![
                    icons::icon("image", 32.0, Color::WHITE),
                    text("CHANGE_IMG").size(12).font(bold()).color(Color::WHITE),
                ]
                .spacing(8)
                .align_x(Alignment::Center),
            )
            .width(222)
            .height(222)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center),
        )
        .padding(0)
        .style(|_t, _s| button::Style {
            background: Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, 0.6))),
            text_color: Color::WHITE,
            border: Border::default(),
            ..Default::default()
        })
        .on_press(Message::PickPlaylistCover(pid));
        hover(base_cov, overlay_btn)
    } else {
        render_cover(spec.cover, 222.0, 0.0, accent, &name_for_cover)
    };

    let dot4 = |ax: Alignment, ay: Alignment| {
        container(container(space::horizontal()).width(4).height(4).style(theme::fill_style(Color::from_rgba(1.0, 1.0, 1.0, 0.5))))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(ax)
            .align_y(ay)
    };
    let cover_reticle = container(
        container(stack![
            dot4(Alignment::Start, Alignment::Start),
            dot4(Alignment::End, Alignment::Start),
            dot4(Alignment::Start, Alignment::End),
            dot4(Alignment::End, Alignment::End),
        ])
        .padding(4)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_t| container::Style {
            border: Border {
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.2),
                width: 0.5,
                radius: 0.0.into(),
            },
            ..Default::default()
        }),
    )
    .padding(8)
    .width(222)
    .height(222);

    let picture = container(stack![cover_inner, cover_reticle].width(222).height(222)).style(|_t| container::Style {
        background: Some(Background::Color(Color::from_rgb8(0x22, 0x22, 0x25))),
        border: Border { color: Color::from_rgb8(0x6B, 0x69, 0x63), width: 1.0, radius: 0.0.into() },
        ..Default::default()
    });
    let cover = stack![
        container(frame_offset).padding(Padding { top: 8.0, left: 8.0, right: 0.0, bottom: 0.0 }),
        container(picture).padding(Padding { top: 0.0, left: 0.0, right: 8.0, bottom: 8.0 }),
    ]
    .width(232)
    .height(232);

    let title = spec.title.to_uppercase();
    let title_el = responsive(move |size| {
        // 60 on two lines, else 46 or 36 on three, cut after that: the header stays as tall as the cover
        let n = title.chars().count() as f32;
        let fits = |sz: f32, lines: f32| n * sz * 0.66 <= size.width * lines;
        let (sz, lines) = if fits(60.0, 2.0) {
            (60.0, 2.0)
        } else if fits(46.0, 3.0) {
            (46.0, 3.0)
        } else {
            (36.0, 3.0)
        };
        container(
            container(
                text(title.clone())
                    .size(sz)
                    .font(bold())
                    .color(Color::WHITE)
                    .line_height(iced::widget::text::LineHeight::Absolute((sz * 1.28).into())),
            )
            .max_height(sz * 1.28 * lines + 8.0)
            .clip(true),
        )
        .height(Length::Fill)
        .align_y(Alignment::End)
        .into()
    })
    .height(Length::Fixed(180.0));

    let dot = || container(space::horizontal()).width(4).height(4).style(theme::fill_style(theme::TEXT_MUTE));
    let mut meta = row![].spacing(16).align_y(Alignment::Center);
    if let Some(a) = &spec.artist {
        let label = row![icons::icon("user", 12.0, Color::WHITE), text(fit(a, 420.0, 12.0)).size(12)]
            .spacing(6)
            .align_y(Alignment::Center);
        meta = meta.push(match spec.artist_link.clone() {
            Some(key) => Element::from(
                button(label).padding(0).style(crate::views::link_style(Color::WHITE)).on_press(Message::OpenArtist(key)),
            ),
            None => Element::from(container(label).style(|_t| container::Style { text_color: Some(Color::WHITE), ..Default::default() })),
        });
    }
    if let Some(y) = spec.year.filter(|y| *y > 0) {
        meta = meta.push(dot()).push(text(y.to_string()).size(12).color(theme::TEXT_DIM));
    }
    if let Some(q) = &spec.quality {
        meta = meta.push(dot()).push(text(q.clone()).size(12).color(theme::TEXT_DIM));
    }
    if let Some(m) = &spec.meta_text {
        meta = meta.push(text(m.clone()).size(12).font(bold()).color(Color::WHITE));
    }

    let rule = row![
        container(space::horizontal()).width(64).height(3).style(theme::fill_style(theme::ORANGE)),
        container(space::horizontal()).width(64).height(3).style(theme::fill_style(theme::GOLD)),
        container(space::horizontal()).width(64).height(3).style(theme::fill_style(theme::TEAL)),
    ];
    let chip_row = row![
        container(text(spec.chip.to_owned()).size(9).font(bold()).color(theme::TEAL)).padding([2, 8]).style(|_t| container::Style {
            background: Some(Background::Color(theme::with_alpha(theme::TEAL, 0.10))),
            border: Border { color: theme::TEAL, width: 1.0, radius: 0.0.into() },
            ..Default::default()
        }),
        container(container(space::horizontal()).height(1).width(Length::Fill).style(theme::fill_style(theme::LINE)))
            .width(Length::Fill)
            .padding([0, 12]),
        text(format!("{} TRACKS", tracks.len())).size(10).color(theme::GOLD),
    ]
    .align_y(Alignment::Center);
    let head_text = column![chip_row, title_el, rule, meta].spacing(10).width(Length::Fill);

    let has_tracks = !tracks.is_empty();
    let play = button(
        container(icons::icon("play", 20.0, theme::VOID)).width(48).height(40).align_x(Alignment::Center).align_y(Alignment::Center),
    )
    .padding(0)
    .style(|_t, status| button::Style {
        background: Some(Background::Color(if matches!(status, button::Status::Hovered) { Color::WHITE } else { theme::TEXT })),
        text_color: theme::VOID,
        border: Border { radius: 0.0.into(), ..Default::default() },
        ..Default::default()
    })
    .on_press_maybe(has_tracks.then(|| Message::PlayDetail(spec.ctx.clone(), 0)));
    let mut actions = row![
        play,
        button(icons::icon("listmusic", 26.0, theme::TEXT_DIM))
            .padding(6)
            .style(theme::bare_button_style())
            .on_press_maybe(has_tracks.then(|| Message::QueueDetail(spec.ctx.clone()))),
    ]
    .spacing(24)
    .align_y(Alignment::Center);
    if let Some(extra) = spec.extra {
        actions = actions.push(extra);
    }

    let back = button(
        row![icons::icon("arrowleft", 14.0, theme::TEXT_DIM), text(app.back_label()).size(10).font(bold()).color(theme::TEXT_DIM)]
            .spacing(8)
            .align_y(Alignment::Center),
    )
    .padding([4, 8])
    .style(theme::bare_button_style())
    .on_press(Message::Back);

    let header = container(
        column![
            container(back).padding([10, 24]),
            container(row![cover, head_text].spacing(32).align_y(Alignment::End)).padding([8, 32]),
            container(actions).padding([8, 32]),
        ]
        .spacing(8),
    )
    .width(Length::Fill)
    .style(move |_t| container::Style {
        background: Some(Background::Gradient(
            iced::gradient::Linear::new(Radians(std::f32::consts::PI))
                .add_stop(0.0, theme::with_alpha(tint, 0.55))
                .add_stop(1.0, theme::PANEL)
                .into(),
        )),
        ..Default::default()
    });

    let mut head_cols = row![container(text("#").size(9).color(theme::TEXT_MUTE)).width(50).align_x(Alignment::Center)].spacing(16);
    if spec.show_cover_col {
        head_cols = head_cols.push(space::horizontal().width(32));
    }
    head_cols = head_cols
        .push(text("OPERATION_LOG").size(9).color(theme::TEXT_MUTE).width(Length::Fill))
        .push(container(text("DURATION").size(9).color(theme::TEXT_MUTE)).width(80).align_x(Alignment::Center));
    let columns = container(head_cols).padding([8, 0]).style(|_t| container::Style {
        border: Border { color: theme::LINE, width: 1.0, radius: 0.0.into() },
        ..Default::default()
    });

    let removable = match &spec.ctx {
        DetailCtx::Playlist(PlaylistView::Custom(id)) => Some(*id),
        _ => None,
    };
    let body: Element<'a, Message> = if let Some(body) = spec.body {
        body
    } else if tracks.is_empty() {
        empty_state("NO DATA FOUND", "ADD TRACKS TO POPULATE")
    } else {
        let multi_disc = matches!(spec.ctx, DetailCtx::Album(_))
            && tracks
                .first()
                .is_some_and(|first| tracks.iter().any(|t| t.disc_number != first.disc_number));
        enum DetailItem {
            Disc(i32),
            Track(usize),
        }
        let mut items: Vec<DetailItem> = Vec::with_capacity(tracks.len() + 4);
        let mut last_disc = -1;
        for (i, t) in tracks.iter().enumerate() {
            if multi_disc && t.disc_number != last_disc {
                last_disc = t.disc_number;
                items.push(DetailItem::Disc(t.disc_number.max(1)));
            }
            items.push(DetailItem::Track(i));
        }

        const ROW_STRIDE: f32 = 54.0;
        let virtualize = tracks.len() > 80;
        let range = if virtualize {
            let list_scroll = (app.detail_scroll_y - 320.0).max(0.0);
            visible_row_range(
                list_scroll,
                (app.window_size.height - 160.0).max(240.0),
                ROW_STRIDE,
                items.len(),
                6,
            )
        } else {
            0..items.len()
        };
        let mut rows: Vec<Element<'a, Message>> = Vec::with_capacity(range.len());
        for item in &items[range.clone()] {
            match *item {
                DetailItem::Disc(d) => rows.push(disc_header(d)),
                DetailItem::Track(i) => {
                    let play = Message::PlayDetail(spec.ctx.clone(), i);
                    rows.push(track_row(
                        app,
                        tracks[i].clone(),
                        i + 1,
                        &spec.ctx,
                        spec.show_cover_col,
                        removable,
                        play,
                        accent,
                    ));
                }
            }
        }
        let mut list_col = column![];
        if virtualize && range.start > 0 {
            list_col = list_col.push(space::vertical().height(range.start as f32 * ROW_STRIDE));
        }
        list_col = list_col.push(column(rows).spacing(2));
        if virtualize && range.end < items.len() {
            list_col = list_col.push(space::vertical().height(
                ((items.len() - range.end) as f32 * ROW_STRIDE - 2.0).max(0.0),
            ));
        }
        column![columns, list_col].spacing(8).into()
    };

    let mut scroll = scrollable(
        column![header, container(body).padding(Padding { top: 16.0, right: 32.0, bottom: 40.0, left: 32.0 })].width(Length::Fill),
    );
    if tracks.len() > 80 {
        scroll = scroll.on_scroll(|vp| Message::DetailScrolled(vp.absolute_offset().y));
    }
    container(scroll)
    .width(Length::Fill)
    .height(Length::Fill)
    .style(|_t| container::Style {
        background: Some(Background::Color(theme::PANEL)),
        border: Border { color: theme::LINE_SOFT, width: 1.0, radius: 0.0.into() },
        ..Default::default()
    })
    .into()
}

pub fn album_detail<'a>(app: &'a App, accent: Color) -> Option<Element<'a, Message>> {
    let key = app.selected_album.as_ref()?;
    let album = app.find_album(key)?;
    let cover = app.covers.get_first(&[album.cover_full_path.as_deref(), album.cover_thumb_path.as_deref()]);
    let all_liked = !album.tracks.is_empty() && album.tracks.iter().all(|t| app.liked_set.contains(&t.id));
    let k_like = key.clone();
    let k_menu = key.clone();
    let extra = row![
        button(icons::icon("heart", 26.0, if all_liked { theme::ORANGE } else { theme::TEXT_DIM }))
            .padding(6)
            .style(theme::bare_button_style())
            .on_press(Message::ToggleLikeTracks(
                album.tracks.iter().map(|t| t.id.clone()).collect(),
            )),
        context_menu::left_trigger(
            button(icons::icon("more", 26.0, theme::TEXT_DIM))
                .padding(6)
                .style(theme::bare_button_style())
                .on_press(Message::Noop),
            move |pos| Message::OpenDetailMenu(DetailCtx::Album(k_menu.clone()), pos),
        ),
    ]
    .spacing(16)
    .align_y(Alignment::Center);
    let _ = k_like;
    let spec = DetailSpec {
        ctx: DetailCtx::Album(key.clone()),
        chip: "ALBUM_DATA",
        title: album.name.clone(),
        artist: Some(album.artist.clone()),
        year: Some(album.year),
        quality: Track::album_quality_label(&album.tracks),
        cover,
        tint: app.detail_tint.unwrap_or(accent),
        show_cover_col: false,
        extra: Some(extra.into()),
        artist_link: Some(crate::library_view::artist_key(&album.artist)),
        body: None,
        meta_text: None,
        is_liked_cover: false,
        change_cover_playlist: None,
    };
    Some(detail_page(app, spec, accent))
}

/// The artist page, as the original's: ARTIST, the name, "N ALBUMS", the most played tracks (when any were
/// played) and the albums as cards, newest first.
pub fn artist_detail<'a>(app: &'a App, accent: Color) -> Option<Element<'a, Message>> {
    let key = app.selected_artist.as_deref()?;
    let artist = app.find_artist(key)?;
    let cover = artist
        .albums
        .iter()
        .find_map(|a| app.covers.get_first(&[a.cover_full_path.as_deref(), a.cover_thumb_path.as_deref()]));
    let ctx = DetailCtx::Artist(key.to_owned());

    // MOST PLAYED: the five tracks played most (then most recently), with their covers
    let mut played: Vec<(&Track, i32, i64)> = artist
        .albums
        .iter()
        .flat_map(|a| a.tracks.iter())
        .filter_map(|t| app.stats.get(&t.id).filter(|s| s.count > 0).map(|s| (t, s.count, s.last)))
        .collect();
    played.sort_by_key(|&(_, count, last)| (std::cmp::Reverse(count), std::cmp::Reverse(last)));
    let popular: Vec<Track> = played.into_iter().take(5).map(|(t, _, _)| t.clone()).collect();

    let section = |label: String| -> Element<'a, Message> {
        container(text(label).size(14).font(bold()).color(theme::TEXT_DIM))
            .padding(Padding { top: 16.0, right: 0.0, bottom: 12.0, left: 0.0 })
            .into()
    };
    let mut body = column![].spacing(2);
    if !popular.is_empty() {
        body = body.push(section("MOST PLAYED".to_owned()));
        for (i, t) in popular.iter().enumerate() {
            let play = Message::PlayTrackFromList(popular.clone(), i);
            body = body.push(track_row(app, t.clone(), i + 1, &ctx, true, None, play, accent));
        }
    }
    body = body.push(section(format!("ALBUMS ({})", artist.albums.len())));
    let (columns, card_h) = app.grid_layout(GridArea::Detail);
    let cards: Vec<Element<'a, Message>> = artist
        .albums
        .iter()
        .map(|a| {
            let k = crate::AlbumKey::from_album(a);
            let k_menu = k.clone();
            let c = app.covers.get_first(&[a.cover_thumb_path.as_deref(), a.cover_full_path.as_deref()]);
            let sub = if a.year > 0 { a.year.to_string() } else { a.artist.to_uppercase() };
            let card = media_card("ALB", a.name.clone(), sub, c, &a.name, card_h, accent, Message::SelectAlbum(k));
            context_menu::trigger(card, move |pos| Message::OpenAlbumMenu(k_menu.clone(), pos))
        })
        .collect();
    body = body.push(card_grid(cards, columns));

    let n = artist.albums.len();
    let spec = DetailSpec {
        ctx,
        chip: "ARTIST",
        title: artist.name.clone(),
        artist: None,
        year: None,
        quality: None,
        cover,
        tint: app.detail_tint.unwrap_or(accent),
        show_cover_col: true,
        extra: None,
        artist_link: None,
        body: Some(body.into()),
        meta_text: Some(if n == 1 { "1 ALBUM".to_owned() } else { format!("{n} ALBUMS") }),
        is_liked_cover: false,
        change_cover_playlist: None,
    };
    Some(detail_page(app, spec, accent))
}

/// Favorites, a custom playlist or a smart playlist.
pub fn playlist_page<'a>(app: &'a App, accent: Color) -> Element<'a, Message> {
    let view = app.playlist_view;
    let liked_tint = Color::from_rgb8(0x50, 0x38, 0xA0);
    let tracks = app.detail_tracks(&DetailCtx::Playlist(view));
    if tracks.is_empty() && matches!(view, PlaylistView::Liked) {
        return container(empty_state("NO DATA FOUND", "MARK TRACKS AS 'FAVORITE' TO POPULATE"))
            .style(|_t| container::Style {
                background: Some(Background::Color(theme::PANEL)),
                border: Border { color: theme::LINE_SOFT, width: 1.0, radius: 0.0.into() },
                ..Default::default()
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    }

    let (chip, title, tint, is_liked_cover, change_cover_playlist, extra): (
        &str,
        String,
        Color,
        bool,
        Option<i64>,
        Option<Element<'a, Message>>,
    ) = match view {
        PlaylistView::Liked => ("PLAYLIST_DATA", "FAVORITES".to_owned(), liked_tint, true, None, None),
        PlaylistView::Custom(id) => {
            let name = app.collections.playlists.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_default();
            let all_liked = !tracks.is_empty() && tracks.iter().all(|t| app.liked_set.contains(&t.id));
            let track_ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
            let extra: Element<'a, Message> = row![
                button(icons::icon("heart", 26.0, if all_liked { theme::ORANGE } else { theme::TEXT_DIM }))
                    .padding(6)
                    .style(theme::bare_button_style())
                    .on_press_maybe((!track_ids.is_empty()).then_some(Message::ToggleLikeTracks(track_ids))),
                context_menu::left_trigger(
                    button(icons::icon("more", 26.0, theme::TEXT_DIM))
                        .padding(6)
                        .style(theme::bare_button_style())
                        .on_press(Message::Noop),
                    move |pos| Message::OpenDetailMenu(DetailCtx::Playlist(PlaylistView::Custom(id)), pos),
                ),
                space::horizontal().width(8),
                text_input("RENAME...", &app.rename_playlist_buf).style(theme::input_style)
                    .on_input(Message::RenamePlaylistInput)
                    .on_submit(Message::RenamePlaylistSubmit(id))
                    .size(12)
                    .padding([6, 8])
                    .width(160),
                button(text("RENAME").size(10)).padding([7, 10]).style(theme::ghost_button_style(false, accent)).on_press(Message::RenamePlaylistSubmit(id)),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into();
            ("PLAYLIST_DATA", name, app.detail_tint.unwrap_or(accent), false, Some(id), Some(extra))
        }
        PlaylistView::Smart(id) => {
            let sp = app.collections.smart.as_deref().unwrap_or(&[]).iter().find(|s| s.id == id);
            let extra: Option<Element<'a, Message>> = sp.map(|_| {
                context_menu::left_trigger(
                    button(icons::icon("more", 26.0, theme::TEXT_DIM))
                        .padding(6)
                        .style(theme::bare_button_style())
                        .on_press(Message::Noop),
                    move |pos| Message::OpenDetailMenu(DetailCtx::Playlist(PlaylistView::Smart(id)), pos),
                )
            });
            ("SMART_DATA", sp.map(|s| s.name.clone()).unwrap_or_default(), app.detail_tint.unwrap_or(accent), false, None, extra)
        }
    };

    let custom_cover = if let PlaylistView::Custom(id) = view {
        app.collections
            .playlists
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| CoverPaths::to_local_path(p.cover_art.as_deref()))
    } else {
        None
    };
    let cover = custom_cover
        .as_deref()
        .and_then(|p| {
            let thumb = PlaylistCovers::thumb_for(p);
            app.covers.get_first(&[Some(p), Some(thumb.as_path())])
        })
        .or_else(|| {
            tracks.first().and_then(|t| app.covers.get_first(&[t.cover_full_path.as_deref(), t.cover_thumb_path.as_deref()]))
        });
    let quality = Track::album_quality_label(&tracks);
    let spec = DetailSpec {
        ctx: DetailCtx::Playlist(view),
        chip,
        title,
        artist: if matches!(view, PlaylistView::Liked) {
            Some("USER DATA".to_owned())
        } else {
            None
        },
        year: None,
        quality,
        cover,
        tint,
        show_cover_col: true,
        extra,
        artist_link: None,
        body: None,
        meta_text: None,
        is_liked_cover,
        change_cover_playlist,
    };
    detail_page(app, spec, accent)
}

fn tab_btn<'a>(label: &'a str, on: bool, msg: Message) -> Element<'a, Message> {
    button(
        column![
            text(label).size(11).font(bold()).color(if on { theme::ORANGE } else { theme::TEXT_MUTE }),
            container(space::horizontal()).width(label.len() as f32 * 8.0).height(2).style(theme::fill_style(if on { theme::ORANGE } else { Color::TRANSPARENT })),
        ]
        .spacing(6),
    )
    .padding([4, 0])
    .style(|_t, _s| button::Style { background: None, ..Default::default() })
    .on_press(msg)
    .into()
}

/// Cards built at most per search section (the heading still counts every match).
const SEARCH_CARDS: usize = 120;

/// "N ALBUM(S) · M TRACKS" under an artist card.
pub fn artist_caption(ar: &retrograde_core::ArtistSummary) -> String {
    let n = ar.albums.len();
    if n == 1 {
        format!("1 ALBUM · {} TRACKS", ar.track_count)
    } else {
        format!("{n} ALBUMS · {} TRACKS", ar.track_count)
    }
}

/// SEARCH: the field, the tabs and the matches as cards.
pub fn search_page<'a>(app: &'a App, accent: Color) -> Element<'a, Message> {
    let field = container(
        row![
            icons::icon("search", 16.0, theme::TEXT_DIM),
            text_input("SEARCH_DATABASE...", &app.search_query)
                .id(app.search_input_id.clone())
                .on_input(Message::SearchChanged)
                .size(14)
                .padding([10, 6])
                .style(|_t, _s| text_input::Style {
                    background: Background::Color(Color::TRANSPARENT),
                    border: Border::default(),
                    icon: theme::TEXT_DIM,
                    placeholder: theme::TEXT_MUTE,
                    value: theme::TEXT,
                    selection: theme::with_alpha(theme::TEAL, 0.3),
                }),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .padding([2, 14])
    .width(Length::Fixed(448.0))
    .style(|_t| container::Style {
        background: Some(Background::Color(theme::VOID)),
        border: Border { color: theme::TEAL, width: 1.0, radius: 0.0.into() },
        ..Default::default()
    });

    let has_query = !app.search_query.trim().is_empty();
    let tabs = row![
        tab_btn("ALL", app.search_tab == SearchTab::All, Message::SetSearchTab(SearchTab::All)),
        tab_btn("TRACKS", app.search_tab == SearchTab::Tracks, Message::SetSearchTab(SearchTab::Tracks)),
        tab_btn("ALBUMS", app.search_tab == SearchTab::Albums, Message::SetSearchTab(SearchTab::Albums)),
        tab_btn("ARTISTS", app.search_tab == SearchTab::Artists, Message::SetSearchTab(SearchTab::Artists)),
        tab_btn("PLAYLISTS", app.search_tab == SearchTab::Playlists, Message::SetSearchTab(SearchTab::Playlists)),
    ]
    .spacing(24);

    let mut sections: Vec<Element<'a, Message>> = Vec::new();
    let heading = |label: String| -> Element<'a, Message> {
        container(text(label).size(14).font(bold()).color(theme::TEXT_DIM))
            .padding(Padding { top: 20.0, right: 0.0, bottom: 8.0, left: 0.0 })
            .width(Length::Fill)
            .style(|_t| container::Style { border: Border { color: theme::LINE, width: 0.0, radius: 0.0.into() }, ..Default::default() })
            .into()
    };

    let want = |t: SearchTab| app.search_tab == SearchTab::All || app.search_tab == t;
    let (columns, card_h) = app.grid_layout(GridArea::Search);
    // Every match is counted; only the first cards of a long section are built
    let more = |shown: usize, total: usize| -> Option<Element<'a, Message>> {
        (total > shown).then(|| {
            container(text(format!("SHOWING {shown} OF {total} // REFINE THE SEARCH")).size(10).color(theme::TEXT_MUTE))
                .padding([12, 0])
                .width(Length::Fill)
                .align_x(Alignment::Center)
                .into()
        })
    };
    if !has_query {
        sections.push(empty_state("SEARCH_READY", "TYPE TO SEARCH THE LIBRARY"));
    } else {
        if want(SearchTab::Tracks) && !app.search_tracks.is_empty() {
            let total = app.search_tracks.len();
            sections.push(heading(format!("TRACKS ({total})")));
            let cards: Vec<Element<'a, Message>> = app
                .search_tracks
                .iter()
                .take(SEARCH_CARDS)
                .enumerate()
                .map(|(i, t)| {
                    let cover = app.covers.get_first(&[t.cover_thumb_path.as_deref(), t.cover_full_path.as_deref()]);
                    let card = media_card("TRK", t.title.clone(), t.artist.to_uppercase(), cover, &t.title, card_h, accent, Message::PlaySearchTrack(i));
                    let t_clone = t.clone();
                    context_menu::trigger(card, move |pos| Message::OpenTrackMenu {
                        track: t_clone.clone(),
                        album_artist: String::new(),
                        album_year: 0,
                        removable_playlist: None,
                        pos,
                    })
                })
                .collect();
            sections.push(card_grid(cards, columns));
            sections.extend(more(SEARCH_CARDS, total));
        }
        if want(SearchTab::Albums) && !app.search_albums.is_empty() {
            let total = app.search_albums.len();
            sections.push(heading(format!("ALBUMS ({total})")));
            let cards: Vec<Element<'a, Message>> = app
                .search_albums
                .iter()
                .take(SEARCH_CARDS)
                .filter_map(|&i| app.albums.get(i))
                .map(|a| {
                    let key = crate::AlbumKey::from_album(a);
                    let k_menu = key.clone();
                    let cover = app.covers.get_first(&[a.cover_thumb_path.as_deref(), a.cover_full_path.as_deref()]);
                    let card = media_card("ALB", a.name.clone(), a.artist.to_uppercase(), cover, &a.name, card_h, accent, Message::SelectAlbum(key));
                    context_menu::trigger(card, move |pos| Message::OpenAlbumMenu(k_menu.clone(), pos))
                })
                .collect();
            sections.push(card_grid(cards, columns));
            sections.extend(more(SEARCH_CARDS, total));
        }
        if want(SearchTab::Artists) && !app.search_artists.is_empty() {
            let total = app.search_artists.len();
            sections.push(heading(format!("ARTISTS ({total})")));
            let cards: Vec<Element<'a, Message>> = app
                .search_artists
                .iter()
                .take(SEARCH_CARDS)
                .filter_map(|&i| app.artists.get(i))
                .map(|ar| {
                    let cover = ar.albums.iter().find_map(|a| app.covers.get_first(&[a.cover_thumb_path.as_deref(), a.cover_full_path.as_deref()]));
                    media_card("ART", ar.name.clone(), artist_caption(ar), cover, &ar.name, card_h, accent, Message::OpenArtist(crate::library_view::artist_key(&ar.name)))
                })
                .collect();
            sections.push(card_grid(cards, columns));
            sections.extend(more(SEARCH_CARDS, total));
        }
        if want(SearchTab::Playlists) && !app.search_playlists.is_empty() {
            sections.push(heading(format!("PLAYLISTS ({})", app.search_playlists.len())));
            let cards: Vec<Element<'a, Message>> = app
                .search_playlists
                .iter()
                .map(|(view, name, n)| {
                    let cover = if let PlaylistView::Custom(pid) = *view {
                        app.collections.playlists.iter().find(|p| p.id == pid).and_then(|p| {
                            CoverPaths::to_local_path(p.cover_art.as_deref()).and_then(|full| {
                                let thumb = PlaylistCovers::thumb_for(&full);
                                app.covers.get_first(&[Some(thumb.as_path()), Some(full.as_path())])
                            }).or_else(|| {
                                p.track_ids.first()
                                    .and_then(|tid| app.lookup_track(tid))
                                    .and_then(|t| app.covers.get_first(&[t.cover_thumb_path.as_deref(), t.cover_full_path.as_deref()]))
                            })
                        })
                    } else {
                        None
                    };
                    let card = media_card("PL", name.clone(), format!("{n} TRACKS"), cover, name, card_h, accent, Message::OpenPlaylistPage(*view));
                    if let PlaylistView::Custom(pid) = *view {
                        context_menu::trigger(card, move |pos| Message::OpenPlaylistMenu(pid, pos))
                    } else {
                        card
                    }
                })
                .collect();
            sections.push(card_grid(cards, columns));
        }
        if sections.is_empty() {
            sections.push(empty_state("NO DATA FOUND", "NOTHING MATCHES THE SEARCH"));
        }
    }

    let header_strip = container(
        column![
            field,
            container(tabs).padding(Padding { top: 16.0, ..Padding::ZERO }),
        ]
        .width(Length::Fill),
    )
    .padding([16, 24])
    .width(Length::Fill)
    .style(|_t| container::Style {
        background: Some(Background::Color(theme::PANEL)),
        ..Default::default()
    });

    column![
        header_strip,
        container(space::horizontal()).width(Length::Fill).height(1).style(theme::fill_style(theme::LINE)),
        scrollable(container(column(sections).width(Length::Fill)).padding(Padding { top: 8.0, right: 24.0, bottom: 24.0, left: 24.0 })).height(Length::Fill),
    ]
    .spacing(0)
    .height(Length::Fill)
    .into()
}

/// NEW_PLAYLIST_ENTRY: asks for the name of a new playlist.
pub fn create_playlist_modal<'a>(app: &'a App) -> Element<'a, Message> {
    let body = container(
        text_input("ENTER_DESIGNATION...", &app.new_playlist_name)
            .id(app.create_input_id.clone())
            .on_input(Message::NewPlaylistNameChanged)
            .on_submit(Message::CreatePlaylist)
            .size(13)
            .padding([10, 10])
            .style(|_t, _s| text_input::Style {
                background: Background::Color(Color::TRANSPARENT),
                border: Border::default(),
                icon: theme::TEXT_DIM,
                placeholder: theme::TEXT_MUTE,
                value: theme::TEXT,
                selection: theme::with_alpha(theme::GOLD, 0.3),
            }),
    )
    .style(|_t| container::Style {
        background: Some(Background::Color(theme::VOID)),
        border: Border { color: theme::GOLD, width: 1.0, radius: 0.0.into() },
        ..Default::default()
    })
    .into();
    let footer = row![
        space::horizontal(),
        button(text("CANCEL").size(10).font(bold()).color(theme::TEXT_DIM))
            .padding([8, 16])
            .style(theme::bare_button_style())
            .on_press(Message::ToggleCreatePlaylist(false)),
        crate::dialogs::ok_button("INITIALIZE", Message::CreatePlaylist),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .into();
    crate::dialogs::modal_frame("NEW_PLAYLIST_ENTRY", body, footer, Some(Message::ToggleCreatePlaylist(false)))
}

#[cfg(test)]
mod tests {
    use super::visible_row_range;

    #[test]
    fn visible_row_range_handles_empty_middle_bottom_and_overscan() {
        assert_eq!(visible_row_range(0.0, 600.0, 200.0, 0, 2), 0..0);
        assert_eq!(visible_row_range(0.0, 600.0, 0.0, 10, 2), 0..0);
        assert_eq!(visible_row_range(0.0, 600.0, 200.0, 20, 1), 0..4);
        assert_eq!(visible_row_range(1000.0, 600.0, 200.0, 20, 1), 4..9);
        assert_eq!(visible_row_range(3800.0, 600.0, 200.0, 20, 2), 17..20);
    }
}
