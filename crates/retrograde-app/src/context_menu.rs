//! Right-click and "⋯" action menus (`COMMAND_MENU` and `ALBUM_OPERATIONS`), matching C#'s
//! `MenuTemplate` in `MainWindow.axaml`.

use std::collections::HashSet;

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree};
use iced::advanced::{Clipboard, Shell, Widget, overlay};
use iced::widget::{button, column, container, mouse_area, opaque, row, scrollable, space, text};
use iced::{
    Alignment, Background, Border, Color, Element, Event, Length, Padding, Point, Rectangle,
    Shadow, Size, Vector, mouse,
};
use retrograde_core::Track;

use crate::smooth_scroll::smooth;
use crate::icons;
use crate::pages::DetailCtx;
use crate::theme;
use crate::views::bold;
use crate::{AlbumKey, App, Message, PlaylistView};

const MENU_WIDTH: f32 = 240.0;

#[derive(Debug, Clone)]
pub enum ContextMenuKind {
    Track {
        track: Box<Track>,
        album_artist: String,
        album_year: i32,
        removable_playlist: Option<i64>,
    },
    Album(AlbumKey),
    Playlist(i64),
    PlaylistPicker {
        tracks: Vec<Track>,
    },
    DetailMore(DetailCtx),
}

#[derive(Debug, Clone)]
pub struct ContextMenu {
    pub pos: Point,
    pub kind: ContextMenuKind,
}

/// Wraps any element so a right-click over its bounds publishes `on_right_click(cursor_pos)`
/// without polling cursor position globally.
pub fn trigger<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    on_right_click: impl Fn(Point) -> Message + 'a,
) -> Element<'a, Message> {
    Element::new(ContextTrigger {
        content: content.into(),
        on_right_click: Some(Box::new(on_right_click)),
        on_left_click: None,
    })
}

/// Wraps a left-clickable control (such as a "⋯" button) so clicking it publishes `on_left_click(cursor_pos)`.
pub fn left_trigger<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    on_left_click: impl Fn(Point) -> Message + 'a,
) -> Element<'a, Message> {
    Element::new(ContextTrigger {
        content: content.into(),
        on_right_click: None,
        on_left_click: Some(Box::new(on_left_click)),
    })
}

struct ContextTrigger<'a, Message> {
    content: Element<'a, Message>,
    on_right_click: Option<Box<dyn Fn(Point) -> Message + 'a>>,
    on_left_click: Option<Box<dyn Fn(Point) -> Message + 'a>>,
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for ContextTrigger<'_, Message> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if let Some(on_left) = &self.on_left_click
            && let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event
            && cursor.is_over(layout.bounds())
        {
            let pos = cursor.position().unwrap_or_else(|| layout.bounds().position());
            shell.publish(on_left(pos));
            shell.capture_event();
            return;
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        if shell.is_event_captured() {
            return;
        }
        if let Some(on_right) = &self.on_right_click
            && let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) = event
            && cursor.is_over(layout.bounds())
        {
            let pos = cursor.position().unwrap_or_else(|| layout.bounds().position());
            shell.publish(on_right(pos));
            shell.capture_event();
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        self.content
            .as_widget_mut()
            .overlay(&mut tree.children[0], layout, renderer, viewport, translation)
    }
}

struct Entry<'a> {
    kind: EntryKind<'a>,
}

enum EntryKind<'a> {
    Separator,
    Caption(&'a str),
    Action {
        icon: &'static str,
        icon_color: Color,
        label: String,
        danger: bool,
        msg: Message,
    },
    PlaylistRow {
        label: String,
        checked: bool,
        msg: Message,
    },
}

fn action<'a>(
    icon: &'static str,
    icon_color: Color,
    label: impl Into<String>,
    danger: bool,
    msg: Message,
) -> Entry<'a> {
    Entry {
        kind: EntryKind::Action {
            icon,
            icon_color,
            label: label.into(),
            danger,
            msg,
        },
    }
}

fn push_playlist_entries<'a>(entries: &mut Vec<Entry<'a>>, app: &App, tracks: &[Track], with_sep: bool) {
    if with_sep {
        entries.push(Entry {
            kind: EntryKind::Separator,
        });
        entries.push(Entry {
            kind: EntryKind::Caption("ADD_TO_PLAYLIST"),
        });
    }
    if app.collections.playlists.is_empty() {
        entries.push(Entry {
            kind: EntryKind::Caption("NO_DATA"),
        });
        return;
    }
    let track_ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
    for p in &app.collections.playlists {
        let set: HashSet<&str> = p.track_ids.iter().map(String::as_str).collect();
        let all = !track_ids.is_empty() && track_ids.iter().all(|id| set.contains(id.as_str()));
        entries.push(Entry {
            kind: EntryKind::PlaylistRow {
                label: p.name.clone(),
                checked: all,
                msg: Message::ContextTogglePlaylistTracks {
                    playlist_id: p.id,
                    track_ids: track_ids.clone(),
                },
            },
        });
    }
}

/// Builds the context menu overlay clamped inside `app.window_size`.
pub fn view<'a>(app: &'a App, menu: &'a ContextMenu) -> Element<'a, Message> {
    let dim = theme::TEXT_DIM;
    let soft = Color::from_rgb8(0xCF, 0xCC, 0xC4);
    let mut entries: Vec<Entry<'a>> = Vec::new();
    let (title, title_color) = match &menu.kind {
        ContextMenuKind::Track {
            track,
            album_artist,
            album_year,
            removable_playlist,
        } => {
            let liked = app.liked_set.contains(&track.id);
            entries.push(action(
                "heart",
                if liked { theme::ORANGE } else { dim },
                if liked { "REMOVE_FAVORITE" } else { "ADD_TO_FAVORITE" },
                false,
                Message::ContextToggleLike(track.id.clone()),
            ));
            entries.push(action(
                "pencil",
                dim,
                "EDIT_TAGS",
                false,
                Message::ContextOpenTrackTags {
                    track: (**track).clone(),
                    album_artist: album_artist.clone(),
                    album_year: *album_year,
                },
            ));
            entries.push(action(
                "playnext",
                dim,
                "PLAY_NEXT",
                false,
                Message::ContextPlayNext(vec![(**track).clone()]),
            ));
            entries.push(action(
                "listmusic",
                dim,
                "ADD_TO_QUEUE",
                false,
                Message::ContextEnqueue(vec![(**track).clone()]),
            ));
            if let Some(pid) = removable_playlist {
                entries.push(action(
                    "trash",
                    theme::ORANGE,
                    "REMOVE_FROM_PLAYLIST",
                    true,
                    Message::ContextRemoveFromPlaylist {
                        playlist_id: *pid,
                        track_id: track.id.clone(),
                    },
                ));
            }
            push_playlist_entries(&mut entries, app, std::slice::from_ref(track.as_ref()), true);
            ("COMMAND_MENU", theme::TEAL)
        }
        ContextMenuKind::Album(key) => {
            let tracks = app
                .find_album(key)
                .map(|a| a.tracks.clone())
                .unwrap_or_default();
            let track_ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
            let all_liked = !track_ids.is_empty() && track_ids.iter().all(|id| app.liked_set.contains(id));
            entries.push(action(
                "playnext",
                dim,
                "PLAY_NEXT",
                false,
                Message::ContextPlayNext(tracks.clone()),
            ));
            entries.push(action(
                "listmusic",
                dim,
                "ADD_ALL_TO_QUEUE",
                false,
                Message::ContextEnqueue(tracks.clone()),
            ));
            entries.push(action(
                "heart",
                if all_liked { theme::ORANGE } else { dim },
                if all_liked {
                    "UNLIKE_ALL_TRACKS"
                } else {
                    "LIKE_ALL_TRACKS"
                },
                false,
                Message::ContextToggleLikeTracks(track_ids),
            ));
            entries.push(action(
                "pencil",
                dim,
                "EDIT_ALBUM_TAGS",
                false,
                Message::ContextOpenAlbumTags(key.clone()),
            ));
            entries.push(action(
                "trash",
                theme::ORANGE,
                "DELETE_ALBUM",
                true,
                Message::ContextDeleteAlbumFromMenu(key.clone()),
            ));
            push_playlist_entries(&mut entries, app, &tracks, true);
            ("ALBUM_OPERATIONS", theme::ORANGE)
        }
        ContextMenuKind::Playlist(id) => {
            let tracks = app.detail_tracks(&DetailCtx::Playlist(PlaylistView::Custom(*id)));
            let track_ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
            let all_liked = !track_ids.is_empty() && track_ids.iter().all(|id| app.liked_set.contains(id));
            let has_cover = app
                .collections
                .playlists
                .iter()
                .find(|p| p.id == *id)
                .is_some_and(|p| p.cover_art.is_some());
            entries.push(action(
                "playnext",
                dim,
                "PLAY_NEXT",
                false,
                Message::ContextPlayNext(tracks.clone()),
            ));
            entries.push(action(
                "listmusic",
                dim,
                "ADD_ALL_TO_QUEUE",
                false,
                Message::ContextEnqueue(tracks.clone()),
            ));
            entries.push(action(
                "heart",
                if all_liked { theme::ORANGE } else { dim },
                if all_liked {
                    "UNLIKE_ALL_TRACKS"
                } else {
                    "LIKE_ALL_TRACKS"
                },
                false,
                Message::ContextToggleLikeTracks(track_ids),
            ));
            entries.push(action(
                "upload",
                dim,
                "CHANGE_COVER",
                false,
                Message::PickPlaylistCover(*id),
            ));
            if has_cover {
                entries.push(action(
                    "trash",
                    theme::ORANGE,
                    "REMOVE_COVER",
                    true,
                    Message::ContextRemovePlaylistCover(*id),
                ));
            }
            entries.push(action(
                "trash",
                theme::ORANGE,
                "DELETE_PLAYLIST",
                true,
                Message::ContextDeletePlaylist(*id),
            ));
            push_playlist_entries(&mut entries, app, &tracks, true);
            ("ALBUM_OPERATIONS", theme::ORANGE)
        }
        ContextMenuKind::PlaylistPicker { tracks } => {
            push_playlist_entries(&mut entries, app, tracks, false);
            ("ADD_TO_PLAYLIST", theme::TEAL)
        }
        ContextMenuKind::DetailMore(ctx) => match ctx {
            DetailCtx::Album(key) => {
                let tracks = app
                    .find_album(key)
                    .map(|a| a.tracks.clone())
                    .unwrap_or_default();
                entries.push(action(
                    "playnext",
                    dim,
                    "PLAY_NEXT",
                    false,
                    Message::ContextPlayNext(tracks.clone()),
                ));
                entries.push(action(
                    "listmusic",
                    dim,
                    "ADD_ALL_TO_QUEUE",
                    false,
                    Message::ContextEnqueue(tracks),
                ));
                entries.push(action(
                    "pencil",
                    dim,
                    "EDIT_ALBUM_TAGS",
                    false,
                    Message::ContextOpenAlbumTags(key.clone()),
                ));
                entries.push(action(
                    "trash",
                    theme::ORANGE,
                    "DELETE_ALBUM",
                    true,
                    Message::ContextDeleteAlbumFromDetail(key.clone()),
                ));
                ("ALBUM_OPERATIONS", theme::ORANGE)
            }
            DetailCtx::Playlist(PlaylistView::Custom(id)) => {
                let has_cover = app
                    .collections
                    .playlists
                    .iter()
                    .find(|p| p.id == *id)
                    .is_some_and(|p| p.cover_art.is_some());
                entries.push(action(
                    "upload",
                    dim,
                    "CHANGE_COVER",
                    false,
                    Message::PickPlaylistCover(*id),
                ));
                if has_cover {
                    entries.push(action(
                        "trash",
                        theme::ORANGE,
                        "REMOVE_COVER",
                        true,
                        Message::ContextRemovePlaylistCover(*id),
                    ));
                }
                entries.push(action(
                    "trash",
                    theme::ORANGE,
                    "DELETE_PLAYLIST",
                    true,
                    Message::ContextDeletePlaylist(*id),
                ));
                ("PLAYLIST_OPERATIONS", theme::ORANGE)
            }
            DetailCtx::Playlist(PlaylistView::Smart(id)) => {
                if let Some(sp) = app
                    .collections
                    .smart
                    .as_deref()
                    .unwrap_or(&[])
                    .iter()
                    .find(|s| s.id == *id)
                {
                    entries.push(action(
                        "filter",
                        theme::TEAL,
                        "EDIT_RULES",
                        false,
                        Message::ContextEditSmart(sp.clone()),
                    ));
                    entries.push(action(
                        "trash",
                        theme::ORANGE,
                        "DELETE_SMART_PLAYLIST",
                        true,
                        Message::ContextDeleteSmart(*id),
                    ));
                }
                ("SMART_OPERATIONS", theme::TEAL)
            }
            _ => ("OPERATIONS", theme::TEAL),
        },
    };

    let mut est_h: f32 = 36.0;
    let mut rows: Vec<Element<'a, Message>> = Vec::with_capacity(entries.len());
    for e in entries {
        match e.kind {
            EntryKind::Separator => {
                est_h += 9.0;
                rows.push(
                    container(space::horizontal())
                        .width(Length::Fill)
                        .height(1)
                        .style(theme::fill_style(theme::LINE))
                        .into(),
                );
                rows.push(space::vertical().height(4).into());
            }
            EntryKind::Caption(c) => {
                est_h += 22.0;
                rows.push(
                    container(text(c).size(8).font(bold()).color(theme::TEXT_MUTE))
                        .padding([4, 12])
                        .into(),
                );
            }
            EntryKind::Action {
                icon,
                icon_color,
                label,
                danger,
                msg,
            } => {
                est_h += 32.0;
                rows.push(menu_button(
                    row![
                        icons::icon(icon, 14.0, icon_color),
                        text(label).size(12).color(if danger { theme::ORANGE } else { soft }),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center)
                    .into(),
                    danger,
                    msg,
                ));
            }
            EntryKind::PlaylistRow {
                label,
                checked,
                msg,
            } => {
                est_h += 30.0;
                let mut content = row![
                    container(space::horizontal())
                        .width(4)
                        .height(4)
                        .style(theme::fill_style(if checked { theme::ORANGE } else { theme::TEXT_MUTE })),
                    text(crate::pages::fit(&label, 170.0, 12.0))
                        .size(12)
                        .color(soft)
                        .width(Length::Fill),
                ]
                .spacing(8)
                .align_y(Alignment::Center);
                if checked {
                    content = content.push(text("✓").size(12).color(theme::ORANGE));
                }
                rows.push(menu_button(content.into(), false, msg));
            }
        }
    }

    let max_h = (app.window_size.height - 32.0).max(180.0);
    let clamped_h = est_h.min(max_h);
    let x = menu.pos.x.clamp(8.0, (app.window_size.width - MENU_WIDTH - 12.0).max(8.0));
    let y = menu.pos.y.clamp(8.0, (app.window_size.height - clamped_h - 12.0).max(8.0));

    let header = column![
        container(text(title).size(9).font(bold()).color(title_color))
            .padding([8, 12])
            .width(Length::Fill),
        container(space::horizontal())
            .width(Length::Fill)
            .height(1)
            .style(theme::fill_style(theme::LINE)),
    ];

    let list = smooth(scrollable(column(rows).spacing(1)).height(Length::Shrink));
    let card = opaque(
        container(column![header, list].spacing(4))
            .padding(4)
            .width(Length::Fixed(MENU_WIDTH))
            .max_height(max_h)
            .style(|_t| container::Style {
                background: Some(Background::Color(theme::RAISED)),
                border: Border {
                    color: Color::from_rgb8(0x46, 0x46, 0x4C),
                    width: 1.0,
                    radius: 0.0.into(),
                },
                shadow: Shadow {
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.65),
                    offset: Vector::new(0.0, 8.0),
                    blur_radius: 24.0,
                },
                ..Default::default()
            }),
    );

    let placed = container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: y,
            left: x,
            right: 0.0,
            bottom: 0.0,
        });

    opaque(
        mouse_area(placed)
            .on_press(Message::CloseContextMenu)
            .on_right_press(Message::CloseContextMenu),
    )
}

fn menu_button<'a>(
    content: Element<'a, Message>,
    danger: bool,
    msg: Message,
) -> Element<'a, Message> {
    button(content)
        .width(Length::Fill)
        .padding([7, 12])
        .style(move |_t, st| {
            let hover = matches!(st, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: Some(Background::Color(if hover {
                    if danger {
                        theme::with_alpha(theme::ORANGE, 0.14)
                    } else {
                        theme::LINE_SOFT
                    }
                } else {
                    Color::TRANSPARENT
                })),
                text_color: theme::TEXT,
                border: Border::default(),
                ..Default::default()
            }
        })
        .on_press(msg)
        .into()
}
