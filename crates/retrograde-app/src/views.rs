use iced::widget::canvas::{self, Canvas};
use iced::widget::text::Wrapping;
use iced::widget::{
    button, column, container, hover, mouse_area, opaque, pick_list, row, scrollable, slider,
    space, stack, text, text_input, toggler, tooltip,
};
use iced::{Alignment, Background, Border, Color, Element, Font, Length, Padding, Point, Rectangle, Renderer, Size, Theme, mouse};
use retrograde_core::{CoverPaths, PlaylistCovers, SmartPlaylists};
use retrograde_linux::{DEFAULT_DISCORD_CLIENT_ID, DiscordRpcStatus};

use crate::context_menu;
use crate::covers_ui::render_cover;
use crate::icons;
use crate::library_view::LibrarySort;
use crate::theme;
use crate::{
    AlbumKey, App, BridgeOption, DisplayOption, GpuOption, LibraryMode, Message, PerfOption,
    PlaylistView, SmartFieldOption, SmartOpOption, SmartSortOption, Tab,
    format_mm_ss,
};

pub fn render_app(app: &App) -> Element<'_, Message> {
    let accent = app.accent_anim.current_accent;
    let body = match if app.active_tab == Tab::NowPlaying { app.previous_tab } else { app.active_tab } {
        Tab::Library | Tab::NowPlaying => view_library(app, accent),
        Tab::Playlists => crate::pages::playlist_page(app, accent),
        Tab::Search => crate::pages::search_page(app, accent),
        Tab::Settings => view_settings(app, accent),
    };

    // The album, artist and playlist pages use the whole area under the stripe
    let shown = if app.active_tab == Tab::NowPlaying { app.previous_tab } else { app.active_tab };
    let full_bleed = match shown {
        Tab::Library => {
            (app.library_mode == LibraryMode::Albums && app.album_detail_open)
                || (app.library_mode == LibraryMode::Artists && app.artist_detail_open)
        }
        Tab::Playlists => true,
        _ => false,
    };
    let stripe = row![
        container(space::horizontal()).width(Length::FillPortion(1)).height(2).style(theme::fill_style(theme::ORANGE)),
        container(space::horizontal()).width(Length::FillPortion(1)).height(2).style(theme::fill_style(theme::GOLD)),
        container(space::horizontal()).width(Length::FillPortion(1)).height(2).style(theme::fill_style(theme::TEAL)),
    ]
    .width(Length::Fill);
    let content = column![
        stripe,
        if full_bleed || app.active_tab == Tab::Search {
            Element::from(
                container(body)
                    .padding(if full_bleed { Padding::ZERO } else { Padding { top: 0.0, right: 16.0, bottom: 0.0, left: 16.0 } })
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
        } else {
            column![
                if app.select_mode && matches!(shown, Tab::Library) {
                    container(view_header(app, accent))
                        .padding([12, 16])
                        .width(Length::Fill)
                        .style(|_t| container::Style {
                            background: Some(Background::Color(Color::from_rgba8(0x12, 0x12, 0x12, 0.95))),
                            border: Border { color: theme::LINE, width: 1.0, radius: 0.0.into() },
                            ..Default::default()
                        })
                        .into()
                } else {
                    Element::from(container(view_header(app, accent)).padding(Padding { top: 14.0, right: 16.0, bottom: 0.0, left: 16.0 }))
                },
                container(body)
                    .padding(Padding { top: 0.0, right: 16.0, bottom: 14.0, left: 16.0 })
                    .width(Length::Fill)
                    .height(Length::Fill),
            ]
            .spacing(14)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        },
    ]
    .width(Length::Fill)
    .height(Length::Fill);

    let win_btn = |icon_name: &str, size: f32, is_close: bool, msg: Message| {
        button(
            container(icons::icon(icon_name, size, theme::TEXT_DIM))
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center),
        )
        .width(30)
        .height(24)
        .padding(0)
        .style(move |_t, status| button::Style {
            background: if matches!(status, button::Status::Hovered) {
                Some(Background::Color(if is_close { theme::ORANGE } else { theme::RAISED }))
            } else {
                None
            },
            text_color: if matches!(status, button::Status::Hovered) { Color::WHITE } else { theme::TEXT_DIM },
            border: Border::default(),
            ..Default::default()
        })
        .on_press(msg)
    };

    let sidebar_toggle_icon = if app.settings.sidebar_visible { "panelleftclose" } else { "panelleftopen" };
    let sidebar_toggle_color = if app.settings.sidebar_visible { theme::TEXT_DIM } else { theme::TEAL };
    let title_strip = container(
        row![
            button(icons::icon(sidebar_toggle_icon, 14.0, sidebar_toggle_color))
                .padding([4, 6])
                .style(theme::bare_button_style())
                .on_press(Message::ToggleSidebar),
            icons::icon("terminal", 12.0, theme::ORANGE),
            text(format!("RETROGRADE // VER {}", env!("CARGO_PKG_VERSION")))
                .size(10)
                .color(theme::TEXT_MUTE),
            mouse_area(space::horizontal().width(Length::Fill).height(Length::Fill))
                .interaction(mouse::Interaction::Grab)
                .on_press(Message::DragMainWindow)
                .on_double_click(Message::ToggleMaximizeMainWindow),
            row![
                win_btn("minus", 14.0, false, Message::MinimizeMainWindow),
                win_btn("square", 11.0, false, Message::ToggleMaximizeMainWindow),
                win_btn("x", 14.0, true, Message::RequestCloseMainWindow),
            ]
            .spacing(2)
            .align_y(Alignment::Center),
        ]
        .spacing(8)
        .padding(Padding { top: 0.0, right: 6.0, bottom: 0.0, left: 8.0 })
        .align_y(Alignment::Center)
        .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(32);

    let main_col: Element<'_, Message> = row![view_sidebar(app, accent), content]
        .spacing(12)
        .padding(Padding { top: 0.0, right: 12.0, bottom: 0.0, left: 12.0 })
        .width(Length::Fill)
        .height(Length::Fill)
        .into();

    let workspace = container(
        column![
            title_strip,
            main_col,
            view_transport(app, accent),
        ]
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(|_t| container::Style {
        background: Some(Background::Color(theme::VOID)),
        border: Border { color: theme::LINE, width: 1.0, radius: 0.0.into() },
        text_color: Some(theme::TEXT),
        ..Default::default()
    });

    let workspace: Element<'_, Message> = crate::fullscreen::view(app, workspace.into(), accent);
    let workspace = crate::popups::overlay(app, workspace, accent);
    let mut layers: Vec<Element<'_, Message>> = vec![workspace];
    if app.add_music_menu_open {
        let menu = flyout_menu(
            vec![
                ("SCAN_FOLDER", false, Message::PickFolders),
                ("SCAN_FILES", false, Message::PickFiles),
            ],
            190.0,
        );
        let rail = app.sidebar_width < 150.0;
        let left = if rail { app.sidebar_width + 18.0 } else { 20.0 };
        let top = if rail { 210.0 } else { 248.0 };
        layers.push(
            mouse_area(space::horizontal().width(Length::Fill).height(Length::Fill))
                .on_press(Message::ToggleAddMusicMenu)
                .into(),
        );
        layers.push(
            container(menu)
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(Padding { top, left, right: 0.0, bottom: 0.0 })
                .into(),
        );
    }
    if app.bank_menu_open {
        let menu = flyout_menu(
            vec![
                ("NEW_PLAYLIST", false, Message::ToggleCreatePlaylist(true)),
                ("NEW_SMART_PLAYLIST", false, Message::NewSmartPlaylist),
            ],
            184.0,
        );
        let rail = app.sidebar_width < 150.0;
        let left = if rail { app.sidebar_width + 18.0 } else { 74.0 };
        let top = if rail { 290.0 } else { 370.0 };
        layers.push(
            mouse_area(space::horizontal().width(Length::Fill).height(Length::Fill))
                .on_press(Message::ToggleBankMenu)
                .into(),
        );
        layers.push(
            container(menu)
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(Padding { top, left, right: 0.0, bottom: 0.0 })
                .into(),
        );
    }
    if app.show_shortcuts_modal {
        layers.push(view_shortcuts_modal(accent));
    }
    if app.egg_open {
        layers.push(crate::egg::view_egg_modal(app));
    }
    if app.create_playlist_open {
        layers.push(crate::pages::create_playlist_modal(app));
    }
    if let Some(draft) = &app.smart_draft {
        layers.push(smart_editor_modal(draft));
    }
    if let Some(draft) = &app.tag_draft {
        layers.push(tag_editor_modal(app, draft, accent));
    }
    if let Some(d) = &app.confirm {
        layers.push(crate::dialogs::confirm_view(d));
    }
    if app.bg_prompt {
        layers.push(crate::dialogs::background_prompt(app.bg_remember));
    }
    if app.deleting_albums {
        let turn = (app.shader_phase * 1.4).fract() as f32;
        layers.push(crate::dialogs::deleting_overlay(turn));
    }
    if let Some(menu) = &app.context_menu {
        layers.push(context_menu::view(app, menu));
    }
    if app.hovered_files > 0 {
        layers.push(view_drop_overlay(accent));
    }
    if let Some(t) = crate::toasts::layer(app) {
        layers.push(t);
    }
    layers.push(resize_handles());
    // Always a stack, so overlays coming and going never rebuild the workspace's widget state (scroll, focus)
    stack(layers).width(Length::Fill).height(Length::Fill).into()
}

/// Thin non-blocking 4px edge & 10px corner resize strips for the borderless window.
fn resize_handles<'a>() -> Element<'a, Message> {
    use iced::window::Direction;
    let strip = |w: Length, h: Length, dir: Direction, cursor: mouse::Interaction| {
        mouse_area(space::horizontal().width(w).height(h))
            .interaction(cursor)
            .on_press(Message::ResizeMainWindow(dir))
    };
    let sides = row![
        container(strip(Length::Fixed(4.0), Length::Fill, Direction::West, mouse::Interaction::ResizingHorizontally))
            .height(Length::Fill),
        space::horizontal().width(Length::Fill),
        container(strip(Length::Fixed(4.0), Length::Fill, Direction::East, mouse::Interaction::ResizingHorizontally))
            .padding(Padding { top: 32.0, right: 0.0, bottom: 0.0, left: 0.0 })
            .height(Length::Fill),
    ]
    .width(Length::Fill)
    .height(Length::Fill);

    let bottom = row![
        strip(Length::Fixed(10.0), Length::Fixed(10.0), Direction::SouthWest, mouse::Interaction::ResizingDiagonallyUp),
        strip(Length::Fill, Length::Fixed(4.0), Direction::South, mouse::Interaction::ResizingVertically),
        strip(Length::Fixed(10.0), Length::Fixed(10.0), Direction::SouthEast, mouse::Interaction::ResizingDiagonallyDown),
    ]
    .width(Length::Fill)
    .align_y(Alignment::End);

    stack![sides, container(bottom).width(Length::Fill).height(Length::Fill).align_y(Alignment::End)]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

struct MiniDiscCanvas {
    phase: f32,
    playing: bool,
    accent: Color,
}

impl<M> canvas::Program<M> for MiniDiscCanvas {
    type State = ();

    fn draw(&self, _s: &(), renderer: &Renderer, _t: &Theme, bounds: Rectangle, _c: mouse::Cursor) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let c = Point::new(bounds.width * 0.5, bounds.height * 0.5);
        let r = bounds.width.min(bounds.height) * 0.45;
        frame.fill(
            &canvas::Path::circle(c, r),
            canvas::Fill::from(Color::from_rgb8(0x14, 0x14, 0x18)),
        );
        frame.stroke(
            &canvas::Path::circle(c, r),
            canvas::Stroke::default().with_width(1.2).with_color(theme::LINE),
        );
        frame.stroke(
            &canvas::Path::circle(c, r * 0.68),
            canvas::Stroke::default().with_width(0.8).with_color(Color::from_rgba(1.0, 1.0, 1.0, 0.1)),
        );
        frame.stroke(
            &canvas::Path::circle(c, r * 0.48),
            canvas::Stroke::default().with_width(0.8).with_color(Color::from_rgba(1.0, 1.0, 1.0, 0.08)),
        );
        let angle = if self.playing { self.phase * 2.4 } else { 0.35 };
        let marker_end = Point::new(c.x + angle.cos() * (r * 0.84), c.y + angle.sin() * (r * 0.84));
        let marker_start = Point::new(c.x + angle.cos() * (r * 0.42), c.y + angle.sin() * (r * 0.42));
        frame.stroke(
            &canvas::Path::line(marker_start, marker_end),
            canvas::Stroke::default().with_width(1.6).with_color(theme::with_alpha(self.accent, 0.7)),
        );
        frame.fill(
            &canvas::Path::circle(c, r * 0.24),
            canvas::Fill::from(theme::with_alpha(self.accent, 0.85)),
        );
        frame.fill(
            &canvas::Path::circle(c, 3.0),
            canvas::Fill::from(theme::VOID),
        );
        vec![frame.into_geometry()]
    }
}

fn mini_marquee(s: &str, max_w: f32, font_px: f32, hovered: bool, phase: f32) -> String {
    let char_w = font_px * 0.58;
    let max_chars = (max_w / char_w).floor().max(4.0) as usize;
    let count = s.chars().count();
    if count <= max_chars || !hovered {
        return crate::pages::fit(s, max_w, font_px);
    }
    let overflow = count - max_chars;
    let pause_s = 0.8_f32;
    let pan_s = (overflow as f32) * 0.22;
    let cycle = pause_s + pan_s + pause_s;
    let t = phase % cycle;
    let offset = if t < pause_s {
        0
    } else if t < pause_s + pan_s {
        (((t - pause_s) / pan_s) * overflow as f32).round() as usize
    } else {
        overflow
    };
    s.chars().skip(offset).take(max_chars).collect()
}

pub fn render_mini_player(app: &App) -> Element<'_, Message> {
    let accent = app.accent_anim.current_accent;
    let max_dur = app.duration_s.max(1.0);
    let pos = app.seeking_pos.unwrap_or(app.position_s).clamp(0.0, max_dur);

    let (title, artist, cover_path, is_liked) = match &app.current_track {
        Some(t) => (
            t.title.clone(),
            t.artist.clone(),
            app.covers
                .get_first(&[t.cover_thumb_path.as_deref(), t.cover_full_path.as_deref()]),
            Some(t.id.clone()),
        ),
        None => (
            "No track loaded".to_owned(),
            "Retrograde Mini".to_owned(),
            None,
            None,
        ),
    };

    let liked = is_liked.as_ref().is_some_and(|tid| app.liked_set.contains(tid));
    let mini_icon_btn = |icon_name: &'static str, icon_sz: f32, col: Color, msg: Option<Message>| {
        let b = button(
            container(icons::icon(icon_name, icon_sz, col))
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center),
        )
        .width(26)
        .height(26)
        .padding(0)
        .style(|_t, st| button::Style {
            background: if matches!(st, button::Status::Hovered) {
                Some(Background::Color(theme::HOVER))
            } else {
                None
            },
            border: Border { color: Color::TRANSPARENT, width: 0.0, radius: 3.0.into() },
            ..Default::default()
        });
        match msg {
            Some(m) => b.on_press(m),
            None => b,
        }
    };

    let cover_inner: Element<'_, Message> = if cover_path.is_some() {
        render_cover(cover_path, 82.0, 0.0, accent, &title)
    } else {
        Canvas::new(MiniDiscCanvas {
            phase: app.shader_phase as f32,
            playing: app.is_playing && app.render_tier.enable_animations,
            accent,
        })
        .width(82)
        .height(82)
        .into()
    };
    let cover_el = container(cover_inner)
        .width(88)
        .height(88)
        .padding(2)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(|_t| container::Style {
            background: Some(Background::Color(theme::RAISED)),
            border: Border { color: Color::from_rgb8(0x46, 0x46, 0x4C), width: 1.0, radius: 0.0.into() },
            ..Default::default()
        });

    let top_row = row![
        column![
            text(mini_marquee(&title, 194.0, 13.0, app.mini_hovered, app.mini_marquee_phase))
                .size(13)
                .font(bold())
                .color(theme::TEXT)
                .wrapping(Wrapping::None),
            text(mini_marquee(&artist, 194.0, 10.0, app.mini_hovered, app.mini_marquee_phase))
                .size(10)
                .color(Color::from_rgb8(0x6B, 0x69, 0x63))
                .wrapping(Wrapping::None),
        ]
        .spacing(2)
        .width(Length::Fill),
        row![
            mini_icon_btn("heart", 14.0, if liked { theme::ORANGE } else { theme::TEXT_MUTE }, is_liked.map(Message::ToggleLike)),
            mini_icon_btn("maximize2", 13.0, theme::TEXT_MUTE, Some(Message::CloseMiniPlayerToMain)),
        ]
        .spacing(2),
    ]
    .spacing(8)
    .align_y(Alignment::Start);

    let controls_row = row![
        space::horizontal(),
        button(icons::icon("skipback", 17.0, theme::TEXT_DIM))
            .style(theme::bare_button_style())
            .padding(4)
            .on_press(Message::Previous),
        button(icons::hex_play(app.is_playing, 40.0))
            .style(theme::bare_button_style())
            .padding(0)
            .on_press(Message::TogglePlayPause),
        button(icons::icon("skipforward", 17.0, theme::TEXT_DIM))
            .style(theme::bare_button_style())
            .padding(4)
            .on_press(Message::Next),
        space::horizontal(),
    ]
    .spacing(22)
    .align_y(Alignment::Center);

    let mute_ts = Color::from_rgb8(0x6B, 0x69, 0x63);
    let seek_row = row![
        container(text(format_mm_ss(pos)).size(9).color(mute_ts)).width(34),
        seek_strip(
            pos,
            max_dur,
            SeekStripSpec {
                height: 12.0,
                track_h: 3.0,
                fill: theme::TEAL,
                track: theme::LINE,
                knob: Some(theme::TEXT),
            },
        ),
        container(text(format_mm_ss(app.duration_s)).size(9).color(mute_ts))
            .width(34)
            .align_x(Alignment::End),
    ]
    .spacing(4)
    .align_y(Alignment::Center);

    let card = container(
        row![
            cover_el,
            column![top_row, controls_row, seek_row]
                .spacing(4)
                .width(Length::Fill)
                .height(Length::Fill),
        ]
        .spacing(16)
        .align_y(Alignment::Center),
    )
    .padding(16)
    .width(Length::Fill)
    .height(Length::Fill)
    .style(|_t| container::Style {
        background: Some(Background::Color(theme::PANEL_DEEP)),
        border: Border {
            color: theme::LINE,
            width: 1.0,
            radius: 0.0.into(),
        },
        text_color: Some(theme::TEXT),
        ..Default::default()
    });

    mouse_area(card)
        .on_press(Message::DragMiniWindow)
        .on_enter(Message::MiniHover(true))
        .on_exit(Message::MiniHover(false))
        .into()
}

fn section_head<'a>(label: &str, color: Color) -> Element<'a, Message> {
    row![
        container(space::horizontal()).width(8).height(1).style(theme::fill_style(color)),
        text(theme::tracked(label)).size(10).color(color),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

pub(crate) fn bold() -> Font {
    Font { weight: iced::font::Weight::Bold, ..Font::DEFAULT }
}

fn rail_tip<'a>(
    content: impl Into<Element<'a, Message>>,
    tip: &str,
    rail: bool,
) -> Element<'a, Message> {
    if !rail {
        return content.into();
    }
    tooltip(
        content,
        container(text(tip.to_owned()).size(10).color(theme::TEXT))
            .padding([5, 9])
            .style(|_t| container::Style {
                background: Some(Background::Color(theme::RAISED)),
                border: Border { color: theme::LINE, width: 1.0, radius: 0.0.into() },
                ..Default::default()
            }),
        tooltip::Position::Right,
    )
    .gap(8)
    .into()
}

fn nav_item<'a>(
    icon: &str,
    label: &'a str,
    active: bool,
    accent: Color,
    rail: bool,
    msg: Message,
) -> Element<'a, Message> {
    let inner: Element<'a, Message> = if rail {
        container(icons::icon(icon, 16.0, if active { theme::TEXT } else { theme::TEXT_DIM }))
            .width(Length::Fill)
            .align_x(Alignment::Center)
            .into()
    } else {
        row![
            icons::icon(icon, 16.0, if active { theme::TEXT } else { theme::TEXT_DIM }),
            text(label).size(13).font(bold()).wrapping(Wrapping::None),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .into()
    };
    let btn = button(inner)
        .width(Length::Fill)
        .padding(if rail { [9, 6] } else { [9, 16] })
        .style(theme::nav_item_style(active, accent))
        .on_press(msg);
    rail_tip(btn, label, rail)
}

struct VisualizerCanvas<'a> {
    playing: bool,
    phase: f32,
    active_bars: [bool; 12],
    cache: &'a canvas::Cache,
}

impl canvas::Program<Message> for VisualizerCanvas<'_> {
    type State = ();

    fn update(
        &self,
        _state: &mut (),
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if let iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event
            && let Some(pos) = cursor.position_in(bounds)
            && bounds.width > 0.0
        {
            let count = crate::egg::BAR_HEIGHT_PCTS.len();
            let idx = ((pos.x / bounds.width) * count as f32).floor() as usize;
            let idx = idx.min(count - 1);
            return Some(canvas::Action::publish(Message::VisualizerBarPressed(idx)).and_capture());
        }
        None
    }

    fn mouse_interaction(
        &self,
        _state: &(),
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if cursor.is_over(bounds) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let geom = self.cache.draw(renderer, bounds.size(), |frame| {
            let count = crate::egg::BAR_HEIGHT_PCTS.len();
            let gap = 4.0f32;
            let bar_w = ((bounds.width - gap * (count as f32 - 1.0)) / count as f32).max(2.0);
            for (i, pct) in crate::egg::BAR_HEIGHT_PCTS.iter().enumerate() {
                let lit = self.active_bars[i];
                let h = if lit {
                    bounds.height * 0.8
                } else if self.playing {
                    4.0 + (bounds.height - 6.0) * ((self.phase * 1.7 + i as f32 * 0.9).sin().abs())
                } else {
                    (bounds.height * (*pct / 100.0)).max(2.0)
                };
                let x = i as f32 * (bar_w + gap);
                let y = (bounds.height - h).max(0.0);
                let color = if lit {
                    crate::egg::bar_active_color(i)
                } else if i % 5 == 2 {
                    theme::TEXT_MUTE
                } else {
                    theme::LINE
                };
                frame.fill_rectangle(Point::new(x, y), Size::new(bar_w, h), color);
            }
        });
        vec![geom]
    }
}

fn view_sidebar(app: &App, accent: Color) -> Element<'_, Message> {
    let rail = app.sidebar_width < 150.0;
    let dot_alpha = if app.is_playing && app.render_tier.enable_animations && app.main_window_focused {
        0.4 + 0.6 * ((app.shader_phase * 2.5).sin() as f32 * 0.5 + 0.5)
    } else {
        1.0
    };
    let logo_badge = container(icons::icon("terminal", 16.0, theme::ORANGE))
        .padding(6)
        .style(|_t| container::Style {
            border: Border { color: theme::ORANGE, width: 1.0, radius: 1.0.into() },
            ..Default::default()
        });
    let logo_row: Element<'_, Message> = if rail {
        container(logo_badge)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .into()
    } else {
        container(
            row![
                logo_badge,
                column![
                    text("[PROJECT_LT] RL09").size(12).font(bold()).color(Color::WHITE).wrapping(Wrapping::None),
                    text("TERMINAL").size(8).color(theme::TEAL).wrapping(Wrapping::None),
                ]
                .spacing(2),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(Alignment::Center)
        .padding([0, 12])
        .into()
    };
    let brand_content = stack![
        logo_row,
        container(
            container(space::horizontal())
                .width(6)
                .height(6)
                .style(move |_t| container::Style {
                    background: Some(Background::Color(theme::with_alpha(theme::TEAL, dot_alpha))),
                    border: Border { radius: 3.0.into(), ..Default::default() },
                    ..Default::default()
                }),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::End)
        .align_y(Alignment::Start)
        .padding(Padding { top: 6.0, right: 6.0, bottom: 0.0, left: 0.0 }),
    ];
    let brand_with_hover = hover(
        brand_content,
        container(
            container(space::horizontal())
                .width(Length::Fill)
                .height(2)
                .style(theme::fill_style(theme::ORANGE)),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(Alignment::End),
    );
    let brand = rail_tip(
        button(brand_with_hover)
            .padding(0)
            .width(Length::Fill)
            .height(64)
            .style(|_t, status| button::Style {
                background: Some(Background::Color(if matches!(status, button::Status::Hovered) {
                    theme::RAISED
                } else {
                    theme::CARD
                })),
                text_color: theme::TEXT,
                border: Border { color: theme::LINE, width: 1.0, radius: 0.0.into() },
                ..Default::default()
            })
            .on_press(Message::ToggleShortcutsModal),
        "SYSTEM_MANUAL (?)",
        rail,
    );

    let tab = app.active_tab;
    let fav_active = tab == Tab::Playlists
        && matches!(app.playlist_view, PlaylistView::Liked)
        && !app.collections.liked.is_empty();
    let mut nav = column![].padding(if rail { [10, 0] } else { [16, 0] });
    if !rail {
        nav = nav.push(section_head("SYSTEM_NAV", theme::TEAL)).push(space::vertical().height(8));
    }
    nav = nav
        .push(nav_item("home", "HOME", tab == Tab::Library, accent, rail, Message::GoHome))
        .push(nav_item("search", "SEARCH", tab == Tab::Search, accent, rail, Message::FocusSearch))
        .push(nav_item("folderplus", "ADD MUSIC", app.add_music_menu_open, accent, rail, Message::ToggleAddMusicMenu))
        .push(nav_item("heart", "FAVORITES", fav_active, accent, rail, Message::OpenPlaylistPage(PlaylistView::Liked)))
        .push(nav_item("grip", "SETTINGS", tab == Tab::Settings, accent, rail, Message::SelectTab(Tab::Settings)));

    let bank_chip = |label: &'static str, on: bool, msg: Message| {
        button(text(label).size(9))
            .padding([4, 8])
            .style(theme::chip_style(on))
            .on_press(msg)
    };
    let bank_items: Vec<Element<'_, Message>> = if app.bank_playlists {
        let mut v: Vec<Element<'_, Message>> = vec![bank_row_icon(
            icons::icon("heart", 12.0, theme::GOLD),
            "FAVORITES",
            format!("{} UNITS", app.collections.liked.len()),
            tab == Tab::Playlists && matches!(app.playlist_view, PlaylistView::Liked),
            accent,
            rail,
            Message::OpenPlaylistPage(PlaylistView::Liked),
        )];
        for sp in app.collections.smart.iter().flatten() {
            let cnt = app.smart_counts.get(&sp.id).copied().unwrap_or(0);
            v.push(bank_row_icon(
                icons::icon("filter", 13.0, theme::TEAL),
                &sp.name,
                format!("{cnt} UNITS"),
                tab == Tab::Playlists && app.playlist_view == PlaylistView::Smart(sp.id),
                accent,
                rail,
                Message::OpenPlaylistPage(PlaylistView::Smart(sp.id)),
            ));
        }
        for p in &app.collections.playlists {
            let pid = p.id;
            let cov = CoverPaths::to_local_path(p.cover_art.as_deref())
                .and_then(|full| {
                    let thumb = PlaylistCovers::thumb_for(&full);
                    app.covers.get_first(&[Some(thumb.as_path()), Some(full.as_path())])
                })
                .or_else(|| {
                    p.track_ids
                        .first()
                        .and_then(|tid| app.lookup_track(tid))
                        .and_then(|t| app.covers.get_first(&[t.cover_thumb_path.as_deref(), t.cover_full_path.as_deref()]))
                });
            let thumb: Element<'_, Message> = if cov.is_some() {
                render_cover(cov, 32.0, 0.0, accent, &p.name)
            } else {
                container(icons::icon("listmusic", 13.0, theme::TEXT_MUTE))
                    .width(32)
                    .height(32)
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center)
                    .style(|_t| container::Style {
                        background: Some(Background::Color(theme::RAISED)),
                        border: Border { color: theme::LINE, width: 1.0, radius: 0.0.into() },
                        ..Default::default()
                    })
                    .into()
            };
            let item = bank_row_with_thumb(
                thumb,
                &p.name,
                format!("{} UNITS", p.track_ids.len()),
                tab == Tab::Playlists && app.playlist_view == PlaylistView::Custom(pid),
                accent,
                rail,
                Message::OpenPlaylistPage(PlaylistView::Custom(pid)),
            );
            v.push(context_menu::trigger(item, move |pos| Message::OpenPlaylistMenu(pid, pos)));
        }
        v
    } else {
        app.sidebar_albums
            .iter()
            .filter_map(|&idx| app.albums.get(idx))
            .map(|a| {
                let key = AlbumKey::from_album(a);
                let key_menu = key.clone();
                let sel = app.selected_album.as_ref() == Some(&key) && app.album_detail_open && tab == Tab::Library;
                let cover = app.covers.get_first(&[a.cover_thumb_path.as_deref(), a.cover_full_path.as_deref()]);
                let btn_inner: Element<'_, Message> = if rail {
                    container(render_cover(cover, 32.0, 0.0, accent, &a.name))
                        .width(Length::Fill)
                        .align_x(Alignment::Center)
                        .into()
                } else {
                    row![
                        render_cover(cover, 32.0, 0.0, accent, &a.name),
                        column![
                            text(crate::pages::fit(&a.name, 152.0, 11.0)).size(11).font(bold()).wrapping(Wrapping::None),
                            text(crate::pages::fit(&a.artist, 152.0, 9.0)).size(9).color(theme::TEXT_MUTE).wrapping(Wrapping::None),
                        ]
                        .spacing(2),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center)
                    .into()
                };
                let btn = rail_tip(
                    button(btn_inner)
                        .width(Length::Fill)
                        .padding(if rail { [4, 4] } else { [5, 8] })
                        .style(theme::nav_item_style(sel, accent))
                        .on_press(Message::SelectAlbum(key)),
                    &a.name,
                    rail,
                );
                context_menu::trigger(btn, move |pos| Message::OpenAlbumMenu(key_menu.clone(), pos))
            })
            .collect()
    };

    let plus_btn = rail_tip(
        button(icons::icon("plus", 14.0, if app.bank_menu_open { theme::TEXT } else { theme::TEXT_DIM }))
            .padding(4)
            .style(theme::bare_button_style())
            .on_press(Message::ToggleBankMenu),
        "CREATE_LIST",
        rail,
    );
    let bank_head: Element<'_, Message> = if rail {
        container(plus_btn).width(Length::Fill).align_x(Alignment::Center).into()
    } else {
        row![section_head("DATA_BANK", theme::GOLD), space::horizontal(), plus_btn]
            .align_y(Alignment::Center)
            .into()
    };
    let list_pad = if rail { Padding::ZERO } else { Padding::ZERO.right(10.0) };
    let list = scrollable(column(bank_items).spacing(2).padding(list_pad)).height(Length::Fill);

    let mut bank_col = column![bank_head]
        .spacing(10)
        .padding(if rail { [8, 4] } else { [14, 8] })
        .height(Length::Fill);
    if !rail {
        bank_col = bank_col.push(
            row![
                bank_chip("PLAYLISTS", app.bank_playlists, Message::SetBankPlaylists(true)),
                bank_chip("ALBUMS", !app.bank_playlists, Message::SetBankPlaylists(false)),
            ]
            .spacing(4),
        );
    }
    bank_col = bank_col.push(list);

    let mut inner_col = column![
        container(nav).style(|_t| container::Style {
            border: Border { color: theme::LINE_SOFT, width: 1.0, radius: 0.0.into() },
            ..Default::default()
        }),
        bank_col,
    ];
    if !rail {
        let spectrum = container(
            Canvas::new(VisualizerCanvas {
                playing: app.is_playing,
                phase: app.shader_phase as f32,
                active_bars: app.egg_morse.active_bars(),
                cache: &app.visualizer_cache,
            })
            .width(Length::Fill)
            .height(Length::Fixed(32.0)),
        )
        .padding(Padding { top: 4.0, right: 12.0, bottom: 8.0, left: 12.0 })
        .height(40)
        .align_y(Alignment::End)
        .width(Length::Fill)
        .style(|_t| container::Style {
            background: Some(Background::Color(Color::from_rgb8(0x0C, 0x0C, 0x0E))),
            border: Border { color: theme::LINE, width: 1.0, radius: 0.0.into() },
            ..Default::default()
        });
        inner_col = inner_col.push(spectrum);
    }

    let panel = container(inner_col)
        .style(|_t| container::Style {
            background: Some(Background::Color(theme::PANEL)),
            border: Border { color: theme::LINE, width: 1.0, radius: 0.0.into() },
            ..Default::default()
        })
        .height(Length::Fill);

    container(
        column![brand, panel]
            .spacing(12)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding::ZERO.bottom(8.0)),
    )
    .width(Length::Fixed(app.sidebar_width))
    .height(Length::Fill)
    .clip(true)
    .into()
}

fn bank_row_icon<'a>(
    glyph: Element<'a, Message>,
    name: &str,
    sub: String,
    active: bool,
    accent: Color,
    rail: bool,
    msg: Message,
) -> Element<'a, Message> {
    let thumb = container(glyph)
        .width(32)
        .height(32)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(|_t| container::Style {
            background: Some(Background::Color(theme::RAISED)),
            border: Border { color: theme::LINE, width: 1.0, radius: 0.0.into() },
            ..Default::default()
        })
        .into();
    bank_row_with_thumb(thumb, name, sub, active, accent, rail, msg)
}

fn bank_row_with_thumb<'a>(
    thumb: Element<'a, Message>,
    name: &str,
    sub: String,
    active: bool,
    accent: Color,
    rail: bool,
    msg: Message,
) -> Element<'a, Message> {
    let inner: Element<'a, Message> = if rail {
        container(thumb).width(Length::Fill).align_x(Alignment::Center).into()
    } else {
        row![
            thumb,
            column![
                text(crate::pages::fit(name, 152.0, 11.0)).size(11).font(bold()).wrapping(Wrapping::None),
                text(sub).size(9).color(theme::TEXT_MUTE).wrapping(Wrapping::None),
            ]
            .spacing(2),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .into()
    };
    let btn = button(inner)
        .width(Length::Fill)
        .padding(if rail { [4, 4] } else { [6, 8] })
        .style(theme::nav_item_style(active, accent))
        .on_press(msg);
    rail_tip(btn, name, rail)
}

/// The page title: an accent bar and the name in capitals, like the original's LIBRARY_DATA.
fn title_block<'a>(label: &str) -> Element<'a, Message> {
    row![
        container(space::horizontal()).width(4).height(24).style(theme::fill_style(theme::ORANGE)),
        text(theme::tracked(label)).size(20).font(bold()).color(Color::WHITE),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .into()
}

fn view_header(app: &App, _accent: Color) -> Element<'_, Message> {
    let title = match app.active_tab {
        Tab::Library | Tab::NowPlaying => "LIBRARY_DATA",
        Tab::Playlists => "PLAYLISTS",
        Tab::Search => "SEARCH",
        Tab::Settings => "SYSTEM_SETTINGS",
    };
    if app.active_tab != Tab::Library {
        return row![title_block(title)].align_y(Alignment::Center).height(28).into();
    }
    if app.select_mode {
        let sel_count = app.selected_albums.len();
        let total = app.albums.len();
        let has_sel = sel_count > 0;
        return row![
            title_block(title),
            space::horizontal(),
            text(format!("{sel_count} / {total} SELECTED"))
                .size(10)
                .color(theme::TEXT_DIM),
            button(text("SELECT_ALL").size(10).font(bold()).color(theme::TEAL))
                .padding([4, 8])
                .style(theme::bare_button_style())
                .on_press(Message::SelectAllAlbums),
            button(
                row![
                    icons::icon("trash", 12.0, if has_sel { theme::ORANGE } else { theme::TEXT_MUTE }),
                    text("DELETE")
                        .size(10)
                        .font(bold())
                        .color(if has_sel { theme::ORANGE } else { theme::TEXT_MUTE }),
                ]
                .spacing(4)
                .align_y(Alignment::Center),
            )
            .padding([5, 10])
            .style(move |_t, st| {
                let hot = has_sel && matches!(st, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: Some(Background::Color(if has_sel {
                        theme::with_alpha(theme::ORANGE, if hot { 0.25 } else { 0.14 })
                    } else {
                        Color::TRANSPARENT
                    })),
                    text_color: if has_sel { theme::ORANGE } else { theme::TEXT_MUTE },
                    border: Border {
                        color: if has_sel { theme::ORANGE } else { theme::LINE },
                        width: 1.0,
                        radius: 0.0.into(),
                    },
                    ..Default::default()
                }
            })
            .on_press_maybe(has_sel.then_some(Message::DeleteSelectedAlbums)),
            button(icons::icon("x", 16.0, theme::TEXT_DIM))
                .padding(4)
                .style(theme::bare_button_style())
                .on_press(Message::ExitSelectMode),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .height(28)
        .into();
    }
    let chip = |label: &'static str, on: bool, msg: Message| {
        button(text(label).size(10))
            .padding([5, 8])
            .style(theme::chip_style(on))
            .on_press(msg)
    };
    let mut header = row![
        title_block(title),
        row![
            chip("ALBUMS", app.library_mode == LibraryMode::Albums, Message::SetLibraryMode(LibraryMode::Albums)),
            chip("ARTISTS", app.library_mode == LibraryMode::Artists, Message::SetLibraryMode(LibraryMode::Artists)),
        ]
        .spacing(4),
        space::horizontal(),
        button(
            row![
                icons::icon("arrowupdown", 12.0, theme::TEXT_DIM),
                text(app.library_sort.label()).size(10).color(theme::TEXT),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding([6, 12])
        .style(outline_button_style(app.sort_menu_open))
        .on_press(Message::ToggleSortMenu),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .height(28);
    if app.library_mode == LibraryMode::Albums {
        header = header.push(
            button(
                row![
                    icons::icon("checksquare", 12.0, theme::TEXT_DIM),
                    text("SELECT").size(10).color(theme::TEXT),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .padding([6, 12])
            .style(outline_button_style(false))
            .on_press(Message::EnterSelectMode),
        );
    }
    header.into()
}

/// A retro flyout: a column of menu entries (label, selected, message) on a raised card.
pub(crate) fn flyout_menu<'a>(items: Vec<(&'a str, bool, Message)>, width: f32) -> Element<'a, Message> {
    let entries: Vec<Element<'a, Message>> = items
        .into_iter()
        .map(|(label, selected, msg)| {
            button(text(label).size(10).color(if selected { theme::ORANGE } else { Color::from_rgb8(0xCF, 0xCC, 0xC4) }))
                .width(Length::Fill)
                .padding([8, 12])
                .style(move |_t, st| button::Style {
                    background: Some(Background::Color(if matches!(st, button::Status::Hovered) {
                        theme::LINE_SOFT
                    } else if selected {
                        theme::with_alpha(theme::ORANGE, 0.08)
                    } else {
                        Color::TRANSPARENT
                    })),
                    text_color: theme::TEXT,
                    border: Border::default(),
                    ..Default::default()
                })
                .on_press(msg)
                .into()
        })
        .collect();
    opaque(
        container(column(entries))
            .padding(4)
            .width(width)
            .style(|_t| container::Style {
                background: Some(Background::Color(theme::RAISED)),
                border: Border { color: Color::from_rgb8(0x46, 0x46, 0x4C), width: 1.0, radius: 0.0.into() },
                shadow: iced::Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.6), offset: iced::Vector::new(0.0, 8.0), blur_radius: 24.0 },
                ..Default::default()
            }),
    )
}

/// The outline buttons of the library header (sort).
fn outline_button_style(open: bool) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_t, st| {
        let hot = open || matches!(st, button::Status::Hovered);
        button::Style {
            background: Some(Background::Color(if hot { theme::LINE_SOFT } else { Color::TRANSPARENT })),
            text_color: theme::TEXT,
            border: Border { color: if hot { theme::TEXT_MUTE } else { theme::LINE }, width: 1.0, radius: 0.0.into() },
            ..Default::default()
        }
    }
}

fn view_library(app: &App, accent: Color) -> Element<'_, Message> {
    let on_grid = !(app.library_mode == LibraryMode::Albums && app.album_detail_open
        || app.library_mode == LibraryMode::Artists && app.artist_detail_open);
    // Always a stack, so the grid keeps its scroll when the flyout opens and closes
    let mut layers = stack![view_library_page(app, accent)].width(Length::Fill).height(Length::Fill);
    if app.sort_menu_open && on_grid {
        // The sort flyout opens under its button, at the right edge of the page
        let items = LibrarySort::ALL
            .iter()
            .map(|s| (s.label(), *s == app.library_sort, Message::SelectLibrarySort(*s)))
            .collect();
        layers = layers
            .push(opaque(mouse_area(space::horizontal().width(Length::Fill).height(Length::Fill)).on_press(Message::ToggleSortMenu)))
            .push(container(flyout_menu(items, 168.0)).width(Length::Fill).align_x(Alignment::End).padding(Padding::ZERO.right(14.0)));
    }
    layers.into()
}

fn view_library_page(app: &App, accent: Color) -> Element<'_, Message> {
    match app.library_mode {
        LibraryMode::Albums => {
            if app.album_detail_open
                && let Some(page) = crate::pages::album_detail(app, accent)
            {
                return page;
            }
            albums_grid(app, accent)
        }
        LibraryMode::Artists => {
            if app.artist_detail_open
                && let Some(page) = crate::pages::artist_detail(app, accent)
            {
                return page;
            }
            crate::pages::artists_grid(app, accent)
        }
    }
}

fn albums_grid(app: &App, accent: Color) -> Element<'_, Message> {
    if app.library_albums.is_empty() {
        // winit has no drag-and-drop on Wayland yet. Native Wayland labels all start with "Wayland";
        // "X11 (Wayland unavailable)" must keep the DnD hint.
        let hint = if app.graphics_report.effective_display.starts_with("Wayland") {
            "USE ADD MUSIC TO SCAN A FOLDER"
        } else {
            "USE ADD MUSIC OR DROP FOLDERS AND FILES ANYWHERE IN THE WINDOW"
        };
        return crate::pages::empty_library_box(hint);
    }
    let (columns, card_h) = app.grid_layout(crate::pages::GridArea::Library);
    let cols = columns.max(1);
    let total = app.library_albums.len();
    let row_stride = card_h + 12.0;
    let total_rows = total.div_ceil(cols);
    let range = crate::pages::visible_row_range(
        app.lib_scroll_y,
        (app.window_size.height - 160.0).max(200.0),
        row_stride,
        total_rows,
        2,
    );
    let start_idx = range.start * cols;
    let end_idx = (range.end * cols).min(total);
    let cards: Vec<Element<'_, Message>> = app.library_albums[start_idx..end_idx]
        .iter()
        .filter_map(|&idx| app.albums.get(idx))
        .map(|a| {
            let key = AlbumKey::from_album(a);
            let cover = app.covers.get_first(&[a.cover_thumb_path.as_deref(), a.cover_full_path.as_deref()]);
            if app.select_mode {
                let selected = app.selected_albums.contains(&key);
                crate::pages::media_card_opts(
                    "ALB",
                    a.name.clone(),
                    a.artist.to_uppercase(),
                    cover,
                    &a.name,
                    card_h,
                    accent,
                    Message::ToggleAlbumSelected(key),
                    Some(selected),
                )
            } else {
                let key_menu = key.clone();
                let card = crate::pages::media_card(
                    "ALB",
                    a.name.clone(),
                    a.artist.to_uppercase(),
                    cover,
                    &a.name,
                    card_h,
                    accent,
                    Message::SelectAlbum(key),
                );
                context_menu::trigger(card, move |pos| Message::OpenAlbumMenu(key_menu.clone(), pos))
            }
        })
        .collect();
    let top_h = range.start as f32 * row_stride;
    let bot_h = (total_rows.saturating_sub(range.end) as f32 * row_stride - 12.0).max(0.0);
    let mut stack_col = column![];
    if top_h > 0.0 {
        stack_col = stack_col.push(space::vertical().height(top_h));
    }
    stack_col = stack_col.push(crate::pages::card_grid(cards, cols));
    if bot_h > 0.0 {
        stack_col = stack_col.push(space::vertical().height(bot_h));
    }
    scrollable(container(stack_col).padding(Padding::ZERO.right(14.0)))
        .on_scroll(|vp| Message::LibScrolled(vp.absolute_offset().y))
        .height(Length::Fill)
        .into()
}

pub fn smart_editor_modal(draft: &crate::SmartDraft) -> Element<'_, Message> {
    let field_opts: Vec<SmartFieldOption> = SmartPlaylists::FIELDS
        .iter()
        .map(|f| SmartFieldOption {
            key: f.key,
            label: f.label,
        })
        .collect();
    let sort_opts: Vec<SmartSortOption> = SmartPlaylists::SORTS
        .iter()
        .map(|&(key, label)| SmartSortOption { key, label })
        .collect();
    let selected_sort = sort_opts.iter().find(|s| s.key == draft.sort).copied();

    let field_label_style = |_t: &Theme| text::Style { color: Some(theme::TEXT_MUTE) };
    let chip = |label: &'static str, on: bool, msg: Message| {
        button(text(label).size(10))
            .padding([5, 10])
            .style(theme::chip_style(on))
            .on_press(msg)
    };

    let mut rule_rows = column![].spacing(8);
    let can_remove = draft.rules.len() > 1;
    for (i, rule) in draft.rules.iter().enumerate() {
        let cur_field = SmartPlaylists::field(&rule.field);
        let op_opts: Vec<SmartOpOption> = cur_field
            .ops
            .iter()
            .map(|&op| SmartOpOption {
                op,
                label: SmartPlaylists::op_label(op),
            })
            .collect();
        let selected_field = field_opts.iter().find(|f| f.key == rule.field).copied();
        let selected_op = op_opts.iter().find(|o| o.op == rule.op).copied();
        let needs_val = SmartPlaylists::needs_value(&rule.op);

        let mut r = row![
            pick_list(field_opts.clone(), selected_field, move |f| Message::SmartRuleFieldChanged(i, f))
                .text_size(11)
                .width(Length::Fixed(146.0))
                .style(theme::pick_style)
                .menu_style(theme::pick_menu_style),
            pick_list(op_opts, selected_op, move |o| Message::SmartRuleOpChanged(i, o))
                .text_size(11)
                .width(Length::Fixed(166.0))
                .style(theme::pick_style)
                .menu_style(theme::pick_menu_style),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        if needs_val {
            r = r.push(
                text_input(cur_field.hint, &rule.value)
                    .size(12)
                    .padding([8, 10])
                    .style(theme::input_style)
                    .on_input(move |v| Message::SmartRuleValueChanged(i, v))
                    .width(Length::Fill),
            );
        } else {
            r = r.push(space::horizontal().width(Length::Fill));
        }

        if can_remove {
            r = r.push(
                button(icons::icon("x", 14.0, theme::TEXT_DIM))
                    .padding(6)
                    .style(theme::bare_button_style())
                    .on_press(Message::RemoveSmartRule(i)),
            );
        } else {
            r = r.push(space::horizontal().width(26));
        }
        rule_rows = rule_rows.push(r);
    }

    let match_line = row![
        text("MATCH").size(9).style(field_label_style),
        chip("ALL", draft.match_all, Message::SmartDraftMatchAllChanged(true)),
        chip("ANY", !draft.match_all, Message::SmartDraftMatchAllChanged(false)),
        text("OF THESE RULES").size(9).style(field_label_style),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let add_rule_btn = button(
        row![
            icons::icon("plus", 12.0, theme::TEXT_DIM),
            text("ADD_RULE").size(10).color(theme::TEXT),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding([6, 12])
    .style(outline_button_style(false))
    .on_press(Message::AddSmartRule);

    let order_row = row![
        text("ORDER").size(9).style(field_label_style),
        pick_list(sort_opts, selected_sort, Message::SmartDraftSortChanged)
            .text_size(11)
            .width(Length::Fixed(190.0))
            .style(theme::pick_style)
            .menu_style(theme::pick_menu_style),
        space::horizontal().width(16),
        text("LIMIT").size(9).style(field_label_style),
        text_input("NONE", &draft.limit)
            .size(12)
            .padding([8, 10])
            .style(theme::input_style)
            .on_input(Message::SmartDraftLimitChanged)
            .width(Length::Fixed(110.0)),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let preview = if draft.match_count == 1 {
        "1 TRACK MATCHES".to_owned()
    } else {
        format!("{} TRACKS MATCH", draft.match_count)
    };

    let body = column![
        text_input("ENTER_DESIGNATION...", &draft.name)
            .size(13)
            .padding([10, 10])
            .style(theme::input_style)
            .on_input(Message::SmartDraftNameChanged)
            .width(Length::Fill),
        match_line,
        container(scrollable(rule_rows)).max_height(220.0),
        add_rule_btn,
        order_row,
        text(preview).size(11).font(bold()).color(theme::TEAL),
    ]
    .spacing(14)
    .into();

    let can_save = !draft.name.trim().is_empty();
    let save_label = if draft.id > 0 { "SAVE" } else { "CREATE" };
    let footer = row![
        space::horizontal(),
        button(text("CANCEL").size(10).font(bold()).color(theme::TEXT_DIM))
            .padding([8, 16])
            .style(theme::bare_button_style())
            .on_press(Message::CancelSmartPlaylist),
        button(text(save_label).size(10).font(bold()))
            .padding([8, 24])
            .style(move |_t, s| {
                let bg = if !can_save {
                    theme::LINE
                } else if matches!(s, button::Status::Hovered) {
                    Color::WHITE
                } else {
                    theme::GOLD
                };
                button::Style {
                    background: Some(Background::Color(bg)),
                    text_color: if can_save { Color::BLACK } else { theme::TEXT_MUTE },
                    border: Border::default(),
                    ..Default::default()
                }
            })
            .on_press_maybe(can_save.then_some(Message::SaveSmartPlaylist)),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .into();

    let heading = if draft.id > 0 {
        "EDIT_SMART_PLAYLIST"
    } else {
        "NEW_SMART_PLAYLIST"
    };
    crate::dialogs::modal_frame_sized(
        crate::dialogs::ModalChrome {
            title: heading,
            dot_color: theme::TEAL,
            corner_color: theme::TEAL,
            width: 640.0,
            header_right: None,
        },
        body,
        footer,
        Some(Message::CancelSmartPlaylist),
    )
}

fn tag_editor_modal<'a>(app: &'a App, draft: &'a crate::TagDraft, _accent: Color) -> Element<'a, Message> {
    let field_label = |label: &'static str| text(label).size(9).font(bold()).color(theme::TEXT_MUTE);
    let cover_handle = draft
        .cover_preview
        .as_deref()
        .and_then(|p| app.covers.get_first(&[Some(p)]));
    let cover_box: Element<'a, Message> = if let Some(h) = cover_handle {
        iced::widget::image(h.clone())
            .width(Length::Fixed(166.0))
            .height(Length::Fixed(166.0))
            .content_fit(iced::ContentFit::Cover)
            .into()
    } else {
        icons::icon("disc", 48.0, Color::from_rgb8(0x46, 0x46, 0x4C))
    };
    let framed_cover = container(cover_box)
        .width(168)
        .height(168)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(|_t| container::Style {
            background: Some(Background::Color(Color::from_rgb8(0x22, 0x22, 0x25))),
            border: Border {
                color: Color::from_rgb8(0x46, 0x46, 0x4C),
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        });

    let outline_btn = |label: &'static str, msg: Option<Message>| {
        let b = button(text(label).size(10).font(bold()).color(theme::TEXT))
            .padding([6, 8])
            .width(Length::Fill)
            .style(|_t, st| button::Style {
                background: Some(Background::Color(if matches!(st, button::Status::Hovered) {
                    theme::HOVER
                } else {
                    theme::PANEL_DEEP
                })),
                border: Border { color: theme::LINE, width: 1.0, radius: 0.0.into() },
                text_color: theme::TEXT,
                ..Default::default()
            });
        match msg {
            Some(m) => b.on_press(m),
            None => b,
        }
    };
    let cover_btns = row![
        outline_btn("CHANGE", (!draft.saving).then_some(Message::TagPickCover)),
        outline_btn("REMOVE", (!draft.saving).then_some(Message::TagRemoveCover)),
    ]
    .spacing(8);

    let cover_note = if draft.new_cover.is_some() {
        "NEW COVER SELECTED"
    } else if draft.remove_cover {
        "COVER WILL BE REMOVED"
    } else {
        ""
    };
    let mut left_col = column![framed_cover, cover_btns].spacing(8).width(168);
    if !cover_note.is_empty() {
        left_col = left_col.push(text(cover_note).size(9).color(Color::from_rgb8(0x7A, 0x78, 0x72)));
    }

    let mut fields = column![].spacing(4).width(Length::Fill);
    if draft.is_track {
        fields = fields
            .push(field_label("TITLE"))
            .push(
                text_input("", &draft.title)
                    .size(12)
                    .padding([8, 10])
                    .style(theme::input_style)
                    .on_input(Message::TagTitleChanged)
                    .width(Length::Fill),
            )
            .push(space::vertical().height(6))
            .push(field_label("ARTIST"))
            .push(
                text_input("", &draft.artist)
                    .size(12)
                    .padding([8, 10])
                    .style(theme::input_style)
                    .on_input(Message::TagArtistChanged)
                    .width(Length::Fill),
            )
            .push(space::vertical().height(6));
    }
    fields = fields
        .push(field_label("ALBUM"))
        .push(
            text_input("", &draft.album)
                .size(12)
                .padding([8, 10])
                .style(theme::input_style)
                .on_input(Message::TagAlbumChanged)
                .width(Length::Fill),
        )
        .push(space::vertical().height(6))
        .push(field_label("ALBUM ARTIST"))
        .push(
            text_input("EMPTY: THE TRACK ARTISTS DECIDE", &draft.album_artist)
                .size(12)
                .padding([8, 10])
                .style(theme::input_style)
                .on_input(Message::TagAlbumArtistChanged)
                .width(Length::Fill),
        )
        .push(space::vertical().height(6));

    let mut num_row = row![
        column![
            field_label("YEAR"),
            text_input("", &draft.year)
                .size(12)
                .padding([8, 10])
                .style(theme::input_style)
                .on_input(Message::TagYearChanged)
                .width(Length::Fill),
        ]
        .spacing(4)
        .width(Length::Fill),
    ]
    .spacing(12)
    .width(Length::Fill);
    if draft.is_track {
        num_row = num_row
            .push(
                column![
                    field_label("TRACK #"),
                    text_input("", &draft.track)
                        .size(12)
                        .padding([8, 10])
                        .style(theme::input_style)
                        .on_input(Message::TagTrackChanged)
                        .width(Length::Fill),
                ]
                .spacing(4)
                .width(Length::Fill),
            )
            .push(
                column![
                    field_label("DISC #"),
                    text_input("", &draft.disc)
                        .size(12)
                        .padding([8, 10])
                        .style(theme::input_style)
                        .on_input(Message::TagDiscChanged)
                        .width(Length::Fill),
                ]
                .spacing(4)
                .width(Length::Fill),
            );
    }
    fields = fields.push(num_row).push(space::vertical().height(8)).push(
        text("WRITTEN INTO THE FILES. ONLY CHANGED FIELDS ARE WRITTEN.")
            .size(9)
            .color(Color::from_rgb8(0x7A, 0x78, 0x72)),
    );
    if let Some(status) = &app.tag_status {
        let (msg, col) = match status {
            Ok(s) => (s.as_str(), theme::TEAL),
            Err(e) => (e.as_str(), theme::ORANGE),
        };
        fields = fields
            .push(space::vertical().height(6))
            .push(text(msg).size(10).color(col));
    }

    let body = row![left_col, fields].spacing(24).width(Length::Fill).into();

    let saving_indicator: Element<'a, Message> = if draft.saving {
        text("WRITING_TAGS...")
            .size(10)
            .font(bold())
            .color(theme::TEAL)
            .into()
    } else {
        space::horizontal().into()
    };

    let cancel_btn = {
        let b = button(text("CANCEL").size(12).color(theme::TEXT_DIM))
            .padding([8, 14])
            .style(theme::bare_button_style());
        if draft.saving {
            b
        } else {
            b.on_press(Message::CloseTagEditor)
        }
    };
    let save_btn = {
        let b = button(text("SAVE").size(12).font(bold()))
            .padding([8, 16])
            .style(theme::accent_button_style(theme::ORANGE));
        if draft.saving {
            b
        } else {
            b.on_press(Message::SaveTrackTags)
        }
    };
    let footer = row![saving_indicator, space::horizontal(), cancel_btn, save_btn]
        .spacing(12)
        .align_y(Alignment::Center)
        .into();

    let header_right = text(crate::pages::fit(&draft.target_label, 340.0, 10.0))
        .size(10)
        .color(theme::TEXT_MUTE)
        .into();

    crate::dialogs::modal_frame_sized(
        crate::dialogs::ModalChrome {
            title: if draft.is_track { "EDIT_TAGS" } else { "EDIT_ALBUM_TAGS" },
            dot_color: theme::ORANGE,
            corner_color: theme::ORANGE,
            width: 620.0,
            header_right: Some(header_right),
        },
        body,
        footer,
        (!draft.saving).then_some(Message::CloseTagEditor),
    )
}

fn orbit_section<'a>(
    title: &str,
    dot_color: Color,
    body: Element<'a, Message>,
) -> Element<'a, Message> {
    let head = container(
        row![
            container(space::horizontal()).width(8).height(8).style(theme::fill_style(dot_color)),
            text(theme::tracked(title)).size(11).font(bold()).color(Color::WHITE),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .padding([10, 16])
    .width(Length::Fill)
    .style(|_t| container::Style {
        background: Some(Background::Color(theme::LINE_SOFT)),
        border: Border { color: theme::LINE, width: 1.0, radius: 0.0.into() },
        ..Default::default()
    });
    container(column![head, container(body).padding(16).width(Length::Fill)])
        .width(Length::Fill)
        .style(|_t| container::Style {
            background: Some(Background::Color(theme::PANEL)),
            border: Border { color: theme::LINE, width: 1.0, radius: 0.0.into() },
            text_color: Some(theme::TEXT),
            ..Default::default()
        })
        .into()
}

fn view_settings(app: &App, accent: Color) -> Element<'_, Message> {
    let gpu_opts = [
        GpuOption(retrograde_core::GpuPreference::Igpu),
        GpuOption(retrograde_core::GpuPreference::Auto),
        GpuOption(retrograde_core::GpuPreference::Dgpu),
        GpuOption(retrograde_core::GpuPreference::Software),
    ];
    let disp_opts = [
        DisplayOption(retrograde_core::DisplayBackend::Auto),
        DisplayOption(retrograde_core::DisplayBackend::Wayland),
        DisplayOption(retrograde_core::DisplayBackend::X11),
    ];
    let perf_opts = [
        PerfOption(retrograde_core::PerformanceMode::Auto),
        PerfOption(retrograde_core::PerformanceMode::Performance),
        PerfOption(retrograde_core::PerformanceMode::Balanced),
        PerfOption(retrograde_core::PerformanceMode::PowerSaver),
    ];
    let bridge_opts = [
        BridgeOption(retrograde_core::DirectVolumeBridge::VirtualSink),
        BridgeOption(retrograde_core::DirectVolumeBridge::HardwareOnly),
    ];

    let mut folders_col = column![].spacing(6);
    if app.settings.library_folders.is_empty() {
        folders_col = folders_col.push(text("NO_FOLDERS_CONFIGURED").size(11).color(theme::TEXT_MUTE));
    } else {
        for f in &app.settings.library_folders {
            folders_col = folders_col.push(
                container(
                    row![
                        text(f.clone())
                            .size(12)
                            .color(theme::TEXT_DIM)
                            .width(Length::Fill),
                        button(text("REMOVE").size(10).font(bold()))
                            .padding([4, 10])
                            .style(theme::ghost_button_style(false, theme::ORANGE))
                            .on_press(Message::RemoveLibraryFolder(f.clone())),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                )
                .padding([8, 12])
                .style(|_t| container::Style {
                    background: Some(Background::Color(theme::PANEL_DEEP)),
                    border: Border { color: theme::LINE_SOFT, width: 1.0, radius: 0.0.into() },
                    ..Default::default()
                }),
            );
        }
    }

    let telemetry = orbit_section(
        "RUNTIME_TELEMETRY",
        theme::TEAL,
        column![
            diagnostics_row(app),
            text("GPU & DISPLAY BACKEND CHANGES REQUIRE RESTART; ACCENT, PERFORMANCE TIER, WATCHER, TRAY, DISCORD RPC & PIPEWIRE BRIDGE APPLY LIVE.")
                .size(10)
                .color(theme::TEXT_MUTE),
        ]
        .spacing(10)
        .into(),
    );

    let graphics = orbit_section(
        "GRAPHICS_&_DISPLAY",
        theme::GOLD,
        column![
            setting_row(
                "GPU PREFERENCE",
                pick_list(
                    gpu_opts.to_vec(),
                    Some(GpuOption(app.settings.gpu_preference)),
                    Message::SetGpuPreference,
                )
                .text_size(12)
                .style(theme::pick_style)
                .menu_style(theme::pick_menu_style)
                .into(),
            ),
            setting_row(
                "DISPLAY BACKEND",
                pick_list(
                    disp_opts.to_vec(),
                    Some(DisplayOption(app.settings.display_backend)),
                    Message::SetDisplayBackend,
                )
                .text_size(12)
                .style(theme::pick_style)
                .menu_style(theme::pick_menu_style)
                .into(),
            ),
            setting_row(
                "PERFORMANCE / POWER MODE",
                pick_list(
                    perf_opts.to_vec(),
                    Some(PerfOption(app.settings.performance_mode)),
                    Message::SetPerformanceMode,
                )
                .text_size(12)
                .style(theme::pick_style)
                .menu_style(theme::pick_menu_style)
                .into(),
            ),
            setting_row(
                "DYNAMIC ACCENT FROM COVER",
                toggler(app.settings.accent_from_cover)
                    .style(theme::toggler_style)
                    .on_toggle(Message::ToggleAccentFromCover)
                    .into(),
            ),
        ]
        .spacing(12)
        .into(),
    );

    let (rpc_label, rpc_color) = match app.discord_rpc_status {
        DiscordRpcStatus::Connected => ("CONNECTED", theme::TEAL),
        DiscordRpcStatus::WaitingForDiscord => ("WAITING FOR DISCORD", theme::GOLD),
        DiscordRpcStatus::Rejected => ("INVALID APP ID", theme::ORANGE),
        DiscordRpcStatus::Disabled => ("OFF", theme::TEXT_MUTE),
    };
    let mut audio_rows = column![
        setting_row(
            "DIRECT ALSA VOLUME SYNC",
            pick_list(
                bridge_opts.to_vec(),
                Some(BridgeOption(app.settings.direct_volume_bridge)),
                Message::SetDirectBridge,
            )
            .text_size(12)
            .style(theme::pick_style)
            .menu_style(theme::pick_menu_style)
            .into(),
        ),
        setting_row(
            "STATUSNOTIFIER TRAY",
            row![
                toggler(app.settings.enable_tray)
                    .style(theme::toggler_style)
                    .on_toggle(Message::ToggleTray),
                text(if app.tray_available { "CONNECTED" } else { "UNAVAILABLE" })
                    .size(10)
                    .color(if app.tray_available { theme::TEAL } else { theme::TEXT_MUTE }),
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .into(),
        ),
        setting_row(
            "MINIMIZE TO TRAY ON CLOSE",
            toggler(app.settings.minimize_to_tray != Some(false))
                .style(theme::toggler_style)
                .on_toggle(Message::ToggleMinimizeToTray)
                .into(),
        ),
        setting_row(
            "MPRIS MEDIA CONTROLS",
            toggler(app.settings.enable_mpris)
                .style(theme::toggler_style)
                .on_toggle(Message::ToggleMpris)
                .into(),
        ),
        setting_row(
            "DISCORD RICH PRESENCE",
            row![
                toggler(app.settings.enable_discord_rpc)
                    .style(theme::toggler_style)
                    .on_toggle(Message::ToggleDiscordRpc),
                text(rpc_label).size(10).color(rpc_color),
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .into(),
        ),
    ]
    .spacing(12);
    if app.settings.enable_discord_rpc {
        audio_rows = audio_rows.push(setting_row(
            "DISCORD ALBUM ARTWORK",
            row![
                toggler(app.settings.discord_fetch_artwork)
                    .style(theme::toggler_style)
                    .on_toggle(Message::ToggleDiscordArtwork),
                text("QUERIES ITUNES / MUSICBRAINZ")
                    .size(10)
                    .color(if app.settings.discord_fetch_artwork {
                        theme::TEXT_DIM
                    } else {
                        theme::TEXT_MUTE
                    }),
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .into(),
        ));
        let mut id_controls = row![
            text_input(DEFAULT_DISCORD_CLIENT_ID, &app.discord_client_id_draft)
                .on_input(Message::DiscordClientIdChanged)
                .on_submit(Message::ApplyDiscordClientId)
                .size(12)
                .padding([6, 10])
                .width(Length::Fixed(210.0))
                .style(theme::input_style),
            button(text("APPLY").size(10).font(bold()))
                .padding([6, 12])
                .style(theme::ghost_button_style(false, accent))
                .on_press(Message::ApplyDiscordClientId),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        if !app.discord_client_id_draft.is_empty() || !app.settings.discord_client_id.is_empty() {
            id_controls = id_controls.push(
                button(text("RESET").size(10).font(bold()))
                    .padding([6, 10])
                    .style(theme::ghost_button_style(false, theme::TEXT_DIM))
                    .on_press(Message::ResetDiscordClientId),
            );
        }
        audio_rows = audio_rows.push(setting_row("DISCORD APP ID", id_controls.into()));
    }
    audio_rows = audio_rows
        .push(setting_row(
            "AUTO-WATCH MUSIC FOLDERS",
            toggler(app.settings.watch_folders)
                .style(theme::toggler_style)
                .on_toggle(Message::ToggleWatchFolders)
                .into(),
        ))
        .push(
            text(format!(
                "AUDIO_PATH // IN: {:?} // OUT: {:?}",
                app.audio_path.input, app.audio_path.output
            ))
            .size(10)
            .color(theme::TEXT_MUTE),
        );
    let audio_sys = orbit_section("AUDIO_&_SYSTEM", theme::ORANGE, audio_rows.into());

    let library = orbit_section(
        "LIBRARY_SOURCES",
        theme::TEAL,
        column![
            row![
                button(text("+ ADD FOLDERS").size(11).font(bold()))
                    .padding([7, 14])
                    .style(theme::accent_button_style(accent))
                    .on_press(Message::PickFolders),
                button(text("+ ADD FILES").size(11).font(bold()))
                    .padding([7, 14])
                    .style(theme::ghost_button_style(false, accent))
                    .on_press(Message::PickFiles),
                button(text("RESCAN ALL").size(11).font(bold()))
                    .padding([7, 14])
                    .style(theme::ghost_button_style(false, accent))
                    .on_press(Message::RescanLibrary),
            ]
            .spacing(8),
            folders_col,
        ]
        .spacing(12)
        .into(),
    );

    scrollable(column![telemetry, graphics, audio_sys, library].spacing(16).padding(Padding::ZERO.right(10.0)))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// A text button that looks like plain text and lightens on hover (artist links).
pub(crate) fn link_style(color: Color) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_t, st| button::Style {
        background: None,
        text_color: if matches!(st, button::Status::Hovered) { Color::WHITE } else { color },
        border: Border::default(),
        ..Default::default()
    }
}

/// An icon button of the transport bar: `on` tints it with the accent.
fn icon_btn<'a>(name: &str, size: f32, on: bool, accent: Color, msg: Message) -> Element<'a, Message> {
    button(icons::icon(name, size, if on { accent } else { theme::TEXT_DIM }))
        .padding(6)
        .style(theme::bare_button_style())
        .on_press(msg)
        .into()
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SeekStripSpec {
    pub height: f32,
    pub track_h: f32,
    pub fill: Color,
    pub track: Color,
    pub knob: Option<Color>,
}

#[derive(Default)]
struct SeekState {
    dragging: bool,
    hovered: bool,
}

struct SeekStrip {
    pos: f64,
    max_dur: f64,
    spec: SeekStripSpec,
}

impl SeekStrip {
    fn seek_pos(&self, bounds: Rectangle, cursor_pos: Point) -> f64 {
        let w = bounds.width.max(1.0);
        let frac = ((cursor_pos.x - bounds.x) / w).clamp(0.0, 1.0) as f64;
        frac * self.max_dur.max(0.0)
    }
}

impl canvas::Program<Message> for SeekStrip {
    type State = SeekState;

    fn update(
        &self,
        state: &mut SeekState,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            iced::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let over = cursor.is_over(bounds);
                let prev = state.hovered;
                state.hovered = over;
                if state.dragging
                    && let Some(p) = cursor.position()
                {
                    return Some(
                        canvas::Action::publish(Message::SeekDragging(self.seek_pos(bounds, p)))
                            .and_capture(),
                    );
                }
                if over != prev {
                    return Some(canvas::Action::request_redraw());
                }
                None
            }
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if !cursor.is_over(bounds) {
                    return None;
                }
                let p = cursor.position()?;
                state.dragging = true;
                Some(
                    canvas::Action::publish(Message::SeekDragging(self.seek_pos(bounds, p)))
                        .and_capture(),
                )
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if state.dragging {
                    state.dragging = false;
                    return Some(canvas::Action::publish(Message::SeekCommit).and_capture());
                }
                None
            }
            _ => None,
        }
    }

    fn mouse_interaction(
        &self,
        state: &SeekState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.dragging || cursor.is_over(bounds) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::None
        }
    }

    fn draw(
        &self,
        state: &SeekState,
        renderer: &Renderer,
        _t: &Theme,
        bounds: Rectangle,
        _c: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let frac = if self.max_dur > 0.0 {
            (self.pos / self.max_dur).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let th = self.spec.track_h;
        let ty = (bounds.height - th) * 0.5;
        frame.fill_rectangle(
            Point::new(0.0, ty),
            Size::new(bounds.width, th),
            self.spec.track,
        );
        let fw = (bounds.width * frac).max(0.0);
        if fw > 0.5 {
            frame.fill_rectangle(Point::new(0.0, ty), Size::new(fw, th), self.spec.fill);
        }
        if (state.hovered || state.dragging)
            && let Some(kc) = self.spec.knob
        {
            let kw = 6.0;
            let kh = (th + 6.0).min(bounds.height);
            let kx = (fw - kw * 0.5).clamp(0.0, (bounds.width - kw).max(0.0));
            let ky = (bounds.height - kh) * 0.5;
            frame.fill_rectangle(Point::new(kx, ky), Size::new(kw, kh), kc);
        }
        vec![frame.into_geometry()]
    }
}

pub(crate) fn seek_strip<'a>(pos: f64, max_dur: f64, spec: SeekStripSpec) -> Element<'a, Message> {
    Canvas::new(SeekStrip { pos, max_dur, spec })
        .width(Length::Fill)
        .height(Length::Fixed(spec.height))
        .into()
}

fn view_transport(app: &App, accent: Color) -> Element<'_, Message> {
    let (title, artist, cover_path, liked_id) = match &app.current_track {
        Some(t) => (
            t.title.clone(),
            t.artist.clone(),
            app.covers
                .get_first(&[t.cover_thumb_path.as_deref(), t.cover_full_path.as_deref()]),
            Some(t.id.clone()),
        ),
        None => ("NO TRACK LOADED".to_owned(), "Select an album to begin".to_owned(), None, None),
    };

    let max_dur = app.duration_s.max(1.0);
    let pos = app.seeking_pos.unwrap_or(app.position_s).clamp(0.0, max_dur);

    // Progress line across the top rendered as a lightweight Canvas SeekStrip
    let line = seek_strip(
        pos,
        max_dur,
        SeekStripSpec {
            height: 12.0,
            track_h: 3.0,
            fill: accent,
            track: theme::LINE_SOFT,
            knob: Some(Color::WHITE),
        },
    );

    let cover_btn = button(render_cover(cover_path, 56.0, 0.0, accent, &title))
        .padding(0)
        .style(theme::bare_button_style())
        .on_press(Message::SelectTab(if app.active_tab == Tab::NowPlaying {
            app.previous_tab
        } else {
            Tab::NowPlaying
        }));
    let left = row![
        cover_btn,
        column![
            text(crate::pages::fit(&title.to_uppercase(), 300.0, 14.0)).size(14).font(bold()).color(theme::TEXT).wrapping(Wrapping::None),
            // The artist opens the artist page, as the original's
            button(text(crate::pages::fit(&artist, 300.0, 12.0)).size(12).wrapping(Wrapping::None))
                .padding(0)
                .style(link_style(theme::TEXT_DIM))
                .on_press_maybe(app.current_track.as_ref().map(|_| Message::OpenCurrentArtist)),
        ]
        .spacing(4),
    ]
    .spacing(16)
    .align_y(Alignment::Center)
    .width(Length::FillPortion(1));

    let repeating = app.repeat_mode != crate::playback::RepeatMode::Off;
    let center = column![
        row![
            icon_btn("shuffle", 18.0, app.shuffle, theme::TEAL, Message::ToggleShuffle),
            icon_btn("skipback", 22.0, false, accent, Message::Previous),
            button(icons::hex_play(app.is_playing, 62.0))
                .padding(0)
                .style(theme::bare_button_style())
                .on_press(Message::TogglePlayPause),
            icon_btn("skipforward", 22.0, false, accent, Message::Next),
            icon_btn("repeat", 18.0, repeating, theme::TEAL, Message::CycleRepeat),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
        row![
            text(format_mm_ss(pos)).size(9).color(theme::TEXT_MUTE),
            space::horizontal(),
            text(format_mm_ss(app.duration_s)).size(9).color(theme::TEXT_MUTE),
        ]
        .width(Length::Fixed(220.0)),
    ]
    .spacing(2)
    .align_x(Alignment::Center);

    // Volume: the original's 15-segment meter (5 teal, 5 gold, 5 orange); an invisible slider over it takes the
    // clicks and snaps to the segments.
    let lit = (app.volume * 15.0).round() as usize;
    let bars: Vec<Element<'_, Message>> = (0..15)
        .map(|i| {
            let on = i < lit && !app.muted;
            let col = if !on {
                theme::LINE
            } else if i < 5 {
                theme::TEAL
            } else if i < 10 {
                theme::GOLD
            } else {
                theme::ORANGE
            };
            container(space::horizontal()).width(Length::Fill).height(18).style(theme::fill_style(col)).into()
        })
        .collect();
    let meter = stack![
        container(row(bars).spacing(2).align_y(Alignment::Center)).align_y(Alignment::Center).height(24),
        slider(0.0..=1.0, app.volume, Message::VolumeDragging)
            .on_release(Message::VolumeCommit)
            .step(1.0 / 15.0)
            .height(24)
            .style(theme::invisible_slider_style),
    ]
    .width(Length::Fixed(98.0))
    .height(24);
    let vol_icon = if app.muted || app.volume <= 0.001 {
        "volumex"
    } else if app.volume < 0.5 {
        "volume1"
    } else {
        "volume2"
    };

    let liked = liked_id.as_ref().is_some_and(|id| app.liked_set.contains(id));
    let heart = match liked_id {
        Some(id) => icon_btn("heart", 18.0, liked, theme::ORANGE, Message::ToggleLike(id)),
        None => icon_btn("heart", 18.0, false, accent, Message::SelectTab(Tab::Playlists)),
    };
    let toggle = |tab: Tab| Message::SelectTab(if app.active_tab == tab { app.previous_tab } else { tab });
    let right = row![
        space::horizontal(),
        button(icons::icon(vol_icon, 18.0, theme::TEXT_DIM))
            .padding(6)
            .style(theme::bare_button_style())
            .on_press(Message::ToggleMute),
        meter,
        container(space::horizontal()).width(1).height(24).style(theme::fill_style(theme::LINE)),
        heart,
        icon_btn("listmusic", 18.0, app.queue_open, accent, Message::ToggleQueuePanel),
        icon_btn("sliders", 18.0, app.audio_open, accent, Message::ToggleAudioPanel),
        icon_btn("miniplayer", 18.0, false, accent, Message::ToggleMiniPlayer),
        icon_btn("maximize2", 18.0, app.active_tab == Tab::NowPlaying, accent, toggle(Tab::NowPlaying)),
    ]
    .spacing(6)
    .align_y(Alignment::Center)
    .width(Length::FillPortion(1));

    container(
        column![
            line,
            row![left, center, right]
                .spacing(16)
                .padding([6, 20])
                .align_y(Alignment::Center),
        ],
    )
    .width(Length::Fill)
    .style(|_t| container::Style {
        background: Some(Background::Color(theme::PANEL)),
        border: Border { color: theme::LINE_SOFT, width: 1.0, radius: 0.0.into() },
        ..Default::default()
    })
    .into()
}

fn view_shortcuts_modal<'a>(_accent: Color) -> Element<'a, Message> {
    let entries = [
        ("Space", "Play / Pause"),
        ("← / →", "Previous / Next Track"),
        ("Shift + ← / →", "Seek -10s / +10s"),
        ("Ctrl + S", "Toggle Shuffle"),
        ("Ctrl + R", "Cycle Repeat Mode"),
        ("M", "Mute / Unmute"),
        ("K / J or ↑ / ↓", "Volume Up / Down"),
        ("F", "Full-Screen Player"),
        ("/ or Ctrl + F", "Focus Library Search"),
        ("Ctrl + B", "Show / Hide Sidebar"),
        ("F5", "Rescan Library"),
        ("Ctrl + M", "Mini Player"),
        ("?", "This List of Shortcuts"),
        ("Escape", "Close Menus / Go Back"),
    ];
    let mut rows = column![
        column![
            text("KEYBOARD SHORTCUTS").size(10).font(bold()).color(theme::ORANGE),
            container(space::horizontal()).width(Length::Fill).height(1).style(theme::fill_style(theme::LINE)),
        ]
        .spacing(4),
    ]
    .spacing(8);

    for (key_label, desc) in entries {
        rows = rows.push(
            row![
                container(text(key_label).size(11).color(Color::from_rgb8(0xA3, 0xA0, 0x99)))
                    .width(Length::Fixed(130.0))
                    .align_x(Alignment::End),
                text(desc).size(12).color(Color::WHITE),
            ]
            .spacing(16)
            .align_y(Alignment::Center),
        );
    }

    let footer = container(
        column![
            text("[PROJECT_LT] RL09 - TERMINAL_V2").size(10).color(theme::TEXT_MUTE),
            text(format!("RETROGRADE // VER {}", env!("CARGO_PKG_VERSION"))).size(10).color(theme::TEXT_MUTE),
        ]
        .spacing(2)
        .align_x(Alignment::Center),
    )
    .width(Length::Fill)
    .align_x(Alignment::Center)
    .into();

    crate::dialogs::modal_frame(
        "SYSTEM_MANUAL",
        rows.into(),
        footer,
        Some(Message::ToggleShortcutsModal),
    )
}

fn view_drop_overlay<'a>(accent: Color) -> Element<'a, Message> {
    let overlay = container(
        container(
            column![
                text("DROP MUSIC TO IMPORT").size(18).color(accent),
                text("Folders are added & scanned; single tracks are scanned directly.")
                    .size(13)
                    .color(theme::TEXT_DIM),
            ]
            .spacing(6)
            .align_x(Alignment::Center),
        )
        .padding(28)
        .style(theme::modal_style),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(Alignment::Center)
    .align_y(Alignment::Center)
    .style(|_t| container::Style {
        background: Some(Background::Color(theme::with_alpha(theme::VOID, 0.76))),
        ..Default::default()
    });
    opaque(overlay)
}

/// What the header chips showed before: graphics, power profile, audio bridge and the bit-perfect check.
fn diagnostics_row(app: &App) -> Element<'_, Message> {
    let bit_perfect_ok = app.bit_perfect_status.starts_with("BIT-PERFECT");
    let shader_mode = if app.wgsl_shader_enabled() { "WGSL Shader" } else { "Static Backdrop" };
    let bridge_label = match app.pw_bridge_node_id {
        Some(id) => format!("PW Bridge #{id}"),
        None if app.direct_mixer_control.is_some() => "ALSA HW Mixer".to_owned(),
        None => "Shared Audio".to_owned(),
    };
    row![
        badge(format!("GPU: {}", app.graphics_report.summary)),
        badge(format!(
            "PPD: {} · {} FPS · {}",
            app.power_profile.label(),
            app.render_tier.fps_cap,
            shader_mode
        )),
        badge(bridge_label),
        accent_pill(
            if app.bit_perfect_status.is_empty() { "SHARED OUTPUT" } else { &app.bit_perfect_status },
            if bit_perfect_ok { theme::TEAL } else { theme::GOLD },
        ),
    ]
    .spacing(6)
    .wrap()
    .vertical_spacing(6)
    .into()
}

fn badge<'a>(label: String) -> Element<'a, Message> {
    container(text(label).size(11))
        .padding([4, 8])
        .style(theme::badge_style)
        .into()
}

fn accent_pill<'a>(label: &str, color: Color) -> Element<'a, Message> {
    container(text(label.to_owned()).size(11))
        .padding([4, 8])
        .style(theme::accent_badge_style(color))
        .into()
}

fn setting_row<'a>(label: &'a str, control: Element<'a, Message>) -> Element<'a, Message> {
    row![
        text(label)
            .size(13)
            .color(theme::TEXT_DIM)
            .width(Length::Fixed(240.0)),
        control
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .into()
}
