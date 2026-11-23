//! The two popups of the player bar, like the original's: QUEUE SYSTEM (current process, next sequence) and
//! the sound popup with its EQUALIZER and OUTPUT tabs. Both sit above the bar at its right edge.

use iced::widget::canvas::{self, Canvas};
use iced::widget::{button, column, container, mouse_area, row, scrollable, space, stack, text};
use iced::{Alignment, Background, Border, Color, Element, Length, Padding, Point, Rectangle, Shadow, Size, Vector, mouse};
use retrograde_audio::EqualizerConfig;

use crate::smooth_scroll::smooth;
use crate::covers_ui::render_cover;
use crate::theme;
use crate::views::bold;
use crate::{App, Message, ReplayGainOption};

const WIDTH: f32 = 320.0;
/// Upcoming queue rows built at most (popup and full screen): shuffle queues the whole library.
pub const QUEUE_ROWS: usize = 50;

fn frame_style(_t: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgb8(0x12, 0x12, 0x12))),
        border: Border { color: theme::LINE, width: 1.0, radius: 0.0.into() },
        shadow: Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.6), offset: Vector::new(0.0, 8.0), blur_radius: 24.0 },
        ..Default::default()
    }
}

fn trunc(s: &str, max: usize) -> String {
    let mut it = s.chars();
    let head: String = it.by_ref().take(max).collect();
    if it.next().is_some() { format!("{head}…") } else { head }
}

fn small_label<'a>(label: &'a str, color: Color) -> Element<'a, Message> {
    text(label).size(10).font(bold()).color(color).into()
}

/// Adds whichever popup is open over the window. A click outside closes it.
pub fn overlay<'a>(app: &'a App, base: Element<'a, Message>, accent: Color) -> Element<'a, Message> {
    if !(app.queue_open || app.audio_open) {
        return stack![base].width(Length::Fill).height(Length::Fill).into();
    }
    let (popup, close): (Element<'a, Message>, Message) = if app.queue_open {
        // QueuePopup[IsVisible=True]: rises 16 px and fades in over 200 ms (cubic-out). What lies under
        // it varies, so the fade is of its content over its own background.
        (
            crate::motion::enter(
                "queue",
                crate::motion::EnterSpec::new(200, crate::motion::Ease::CubicOut, 16.0, crate::motion::Fade::Cover(Color::from_rgb8(0x12, 0x12, 0x12))),
                queue_popup(app, accent),
            ),
            Message::ToggleQueuePanel,
        )
    } else {
        (audio_popup(app, accent), Message::ToggleAudioPanel)
    };
    let dismiss = mouse_area(space::horizontal().width(Length::Fill).height(Length::Fill)).on_press(close);
    // Above the player bar at the right (the queue as the original's: 32 px in, 112 px up)
    let right = if app.queue_open { 32.0 } else { 16.0 };
    let placed = container(iced::widget::opaque(popup))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::End)
        .align_y(Alignment::End)
        .padding(Padding { top: 48.0, right, bottom: 112.0, left: 16.0 });
    stack![base, dismiss, placed].width(Length::Fill).height(Length::Fill).into()
}

fn queue_row<'a>(app: &'a App, i: usize, accent: Color, current: bool, by_hand: bool) -> Element<'a, Message> {
    let t = &app.queue_tracks[i];
    let cover = app.covers.get_first(&[t.cover_thumb_path.as_deref(), t.cover_full_path.as_deref()]);
    let number: Element<'a, Message> = if current {
        space::horizontal().width(0).into()
    } else {
        container(text(format!("{}", i + 1)).size(10).color(Color::from_rgb8(0x46, 0x46, 0x4C))).width(20).align_x(Alignment::Center).into()
    };
    let body = row![
        number,
        render_cover(cover, 32.0, 0.0, accent, &t.title),
        column![
            text(trunc(&t.title, 30)).size(if current { 14 } else { 13 }).font(if current { bold() } else { iced::Font::DEFAULT }).color(if current { theme::TEAL } else { theme::TEXT }).wrapping(iced::widget::text::Wrapping::None),
            text(trunc(&t.artist.to_uppercase(), 34)).size(9).color(theme::TEXT_MUTE).wrapping(iced::widget::text::Wrapping::None),
        ]
        .spacing(2)
        .width(Length::Fill),
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    let main = button(body)
        .width(Length::Fill)
        .padding([8, 12])
        .style(theme::nav_item_style(current, theme::TEAL))
        .on_press(Message::JumpToQueueIndex(i));
    if current {
        main.into()
    } else {
        // Tracks queued by hand (ADD_TO_QUEUE / PLAY_NEXT) play first; a gold edge marks them
        let edge = container(space::horizontal())
            .width(2)
            .height(32)
            .style(theme::fill_style(if by_hand { theme::GOLD } else { Color::TRANSPARENT }));
        row![
            edge,
            main,
            button(crate::icons::icon("x", 12.0, theme::TEXT_MUTE))
                .padding(6)
                .style(theme::bare_button_style())
                .on_press(Message::RemoveQueueIndex(i)),
        ]
        .align_y(Alignment::Center)
        .into()
    }
}

fn queue_popup<'a>(app: &'a App, accent: Color) -> Element<'a, Message> {
    let head = row![
        container(space::horizontal()).width(8).height(8).style(theme::fill_style(theme::GOLD)),
        text("QUEUE SYSTEM").size(12).font(bold()).color(Color::from_rgb8(0xCF, 0xCC, 0xC4)),
        space::horizontal(),
        button(text("CLEAR_ALL").size(9).color(theme::TEXT_DIM))
            .padding([5, 8])
            .style(theme::chip_style(false))
            .on_press(Message::ClearQueue),
        button(crate::icons::icon("x", 14.0, theme::TEXT_DIM))
            .padding(6)
            .style(theme::bare_button_style())
            .on_press(Message::ToggleQueuePanel),
    ]
    .spacing(10)
    .align_y(Alignment::Center)
    .padding([10, 14]);

    let mut list = column![].spacing(0);
    if app.queue_tracks.is_empty() {
        list = list.push(
            container(text("QUEUE_EMPTY").size(12).font(iced::Font { style: iced::font::Style::Italic, ..iced::Font::DEFAULT }).color(theme::TEXT_MUTE))
                .padding(32)
                .width(Length::Fill)
                .align_x(Alignment::Center),
        );
    } else {
        list = list.push(section_band("CURRENT PROCESS", theme::TEAL, true));
        if let Some(cur) = app.queue_index.filter(|&i| i < app.queue_tracks.len()) {
            list = list.push(queue_row(app, cur, accent, true, false));
        }
        list = list.push(section_band("NEXT SEQUENCE", theme::GOLD, false));
        let from = app.queue_index.map_or(0, |i| i + 1).min(app.queue_tracks.len());
        // Only the next rows are built: a shuffled library queues every track
        let to = (from + QUEUE_ROWS).min(app.queue_tracks.len());
        for i in from..to {
            list = list.push(queue_row(app, i, accent, false, i - from < app.queue_hand_len));
        }
        let rest = app.queue_tracks.len() - to;
        let end = if rest > 0 { format!("+{rest} MORE") } else { "END_OF_QUEUE".to_owned() };
        list = list.push(container(text(end).size(10).color(Color::from_rgb8(0x46, 0x46, 0x4C))).padding(16).width(Length::Fill).align_x(Alignment::Center));
    }

    // As tall as its rows, at most 60 % of the window and 520 px
    let max_h = (app.window_size.height * 0.6).min(520.0);
    container(column![
        head,
        container(space::horizontal()).height(1).width(Length::Fill).style(theme::fill_style(theme::LINE)),
        smooth(scrollable(container(list).padding(Padding::ZERO.right(12.0))).height(Length::Shrink)),
    ])
    .width(WIDTH + 2.0)
    .height(Length::Shrink)
    .max_height(max_h)
    .style(frame_style)
    .into()
}

/// A queue section title on its own band, like the original's CURRENT PROCESS / NEXT SEQUENCE.
fn section_band<'a>(label: &'a str, color: Color, tinted: bool) -> Element<'a, Message> {
    container(small_label(label, color))
        .padding([8, 16])
        .width(Length::Fill)
        .style(move |_t| container::Style {
            background: tinted.then_some(Background::Color(theme::with_alpha(theme::TEAL, 0.05))),
            border: Border { color: theme::LINE, width: 0.0, radius: 0.0.into() },
            ..Default::default()
        })
        .into()
}

// ---------- Sound popup (C# MainWindow.axaml, AudioButton flyout) ----------

/// Crossfade choices in seconds (C# `CrossfadeChoices`).
pub const CROSSFADE_CHOICES: [i32; 7] = [0, 2, 4, 6, 8, 10, 12];

const HINT: Color = Color::from_rgb(0x7A as f32 / 255.0, 0x78 as f32 / 255.0, 0x72 as f32 / 255.0);
const SLASHES: Color = Color::from_rgb(0x56 as f32 / 255.0, 0x55 as f32 / 255.0, 0x4F as f32 / 255.0);
const DEVICE_TEXT: Color = Color::from_rgb(0xCF as f32 / 255.0, 0xCC as f32 / 255.0, 0xC4 as f32 / 255.0);
const ROW_LINE: Color = Color::from_rgb(0x22 as f32 / 255.0, 0x22 as f32 / 255.0, 0x25 as f32 / 255.0);

fn hline<'a>(color: Color) -> Element<'a, Message> {
    container(space::horizontal()).height(1).width(Length::Fill).style(theme::fill_style(color)).into()
}

/// C# `Button.device.gain`: a flat cell, gold on a gold tint when selected.
fn gain_style(selected: bool) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_t, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if selected {
                theme::with_alpha(theme::GOLD, 0x22 as f32 / 255.0)
            } else if hovered {
                ROW_LINE
            } else {
                Color::TRANSPARENT
            })),
            text_color: if selected { theme::GOLD } else { DEVICE_TEXT },
            border: Border::default(),
            ..Default::default()
        }
    }
}

/// C# `Button.device`: a full-width row, gold when it is the chosen output.
fn device_style(selected: bool) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_t, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if hovered { ROW_LINE } else { Color::TRANSPARENT })),
            text_color: if selected { theme::GOLD } else { DEVICE_TEXT },
            border: Border::default(),
            ..Default::default()
        }
    }
}

/// C# `Button.wasapi`: the small ON / OFF switch, orange when on.
fn switch_style(on: bool) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_t, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if on {
                theme::with_alpha(theme::ORANGE, 0x1A as f32 / 255.0)
            } else {
                Color::TRANSPARENT
            })),
            text_color: if on { theme::ORANGE } else if hovered { theme::TEXT_DIM } else { theme::TEXT_MUTE },
            border: Border { color: if on { theme::ORANGE } else { theme::LINE }, width: 1.0, radius: 2.0.into() },
            ..Default::default()
        }
    }
}

fn gain_cell<'a>(label: &'a str, selected: bool, msg: Message) -> Element<'a, Message> {
    button(container(text(label).size(10)).width(Length::Fill).align_x(Alignment::Center))
        .width(Length::Fill)
        .padding([10, 4])
        .style(gain_style(selected))
        .on_press(msg)
        .into()
}

/// A section title on its band (C#: teal title, "///" at the right, hairlines above and below).
fn header<'a>(label: &'a str, right: Option<Element<'a, Message>>) -> Element<'a, Message> {
    let right = right.unwrap_or_else(|| text("///").size(9).font(bold()).color(SLASHES).into());
    column![
        hline(theme::LINE),
        row![text(label).size(9).font(bold()).color(theme::TEAL), space::horizontal(), right]
            .align_y(Alignment::Center)
            .padding([8, 12]),
        hline(theme::LINE),
    ]
    .into()
}

fn hint<'a>(s: impl text::IntoFragment<'a>, color: Color, top: f32, bottom: f32) -> Element<'a, Message> {
    container(text(s).size(9).color(color).wrapping(text::Wrapping::Word))
        .padding(Padding { top, right: 12.0, bottom, left: 12.0 })
        .width(Length::Fill)
        .into()
}

fn audio_popup<'a>(app: &'a App, _accent: Color) -> Element<'a, Message> {
    let tabs = row![
        gain_cell("EQUALIZER", !app.audio_output_tab, Message::SetAudioOutputTab(false)),
        gain_cell("OUTPUT", app.audio_output_tab, Message::SetAudioOutputTab(true)),
    ];

    let body: Element<'a, Message> = if app.audio_output_tab { output_tab(app) } else { eq_tab(app) };

    // The equalizer bands take the plain wheel to step their gain, so that tab keeps the stock list
    // (the smooth wrapper would claim the notch before the bands see it).
    let body_list = scrollable(body).height(Length::Shrink);
    let body_list: Element<'a, Message> = if app.audio_output_tab { smooth(body_list) } else { body_list.into() };
    container(column![tabs, body_list])
        .width(WIDTH)
        .height(Length::Shrink)
        .max_height(640.0)
        .style(frame_style)
        .into()
}

fn eq_tab<'a>(app: &'a App) -> Element<'a, Message> {
    let on = app.settings.eq_enabled;
    let mut col = column![];
    if app.direct_active {
        col = col.push(
            container(
                text("BYPASSED: DIRECT OUTPUT (BIT-PERFECT) PLAYS THE FILE AS IT IS. CROSSFADE IS OFF TOO.")
                    .size(9)
                    .font(bold())
                    .color(theme::GOLD)
                    .wrapping(text::Wrapping::Word),
            )
            .padding(Padding { top: 8.0, right: 12.0, bottom: 0.0, left: 12.0 }),
        );
    }
    let switch = button(container(text(if on { "ON" } else { "OFF" }).size(10)).align_y(Alignment::Center).height(Length::Fill))
        .height(24)
        .padding([0, 8])
        .style(switch_style(on))
        .on_press(Message::SetEqEnabled(!on));
    col = col.push(container(header("STATE", Some(switch.into()))).padding(Padding::ZERO.top(4.0)));

    // Presets: three to a row
    let mut presets = column![].padding(Padding::ZERO.top(4.0));
    for chunk in EqualizerConfig::PRESETS.chunks(3) {
        presets = presets.push(row(chunk.iter().map(|(name, _)| {
            gain_cell(name, app.settings.eq_preset == *name, Message::SelectEqPreset((*name).to_owned()))
        })));
    }
    col = col.push(presets);

    // dB scale and the ten bands
    let mut gains = [0.0; 10];
    for (i, g) in gains.iter_mut().enumerate() {
        *g = app.settings.eq_gains.get(i).copied().unwrap_or(0.0);
    }
    let scale = column![
        text("+12").size(8).color(SLASHES),
        container(text("0").size(8).color(SLASHES)).height(Length::Fill).align_y(Alignment::Center),
        text("-12").size(8).color(SLASHES),
    ]
    .width(22)
    .height(EQ_HEIGHT);
    let labels = row(EqualizerConfig::BAND_HZ.iter().map(|&hz| {
        let label = if hz >= 1000 { format!("{}K", hz / 1000) } else { hz.to_string() };
        Element::from(container(text(label).size(8).color(HINT)).width(Length::FillPortion(1)).align_x(Alignment::Center))
    }));
    let bands = column![
        Canvas::new(EqBands { gains, active: on }).width(Length::Fill).height(EQ_HEIGHT),
        labels,
    ]
    .spacing(6)
    .width(Length::Fill);
    col = col.push(
        row![scale, bands].padding(Padding { top: 10.0, right: 12.0, bottom: 4.0, left: 8.0 }),
    );
    col = col.push(hint("DRAG OR SCROLL A BAND, DOUBLE-CLICK RESETS IT.", HINT, 2.0, 8.0));

    // Crossfade
    let cf = app.settings.crossfade_seconds;
    col = col.push(container(header("CROSSFADE", None)).padding([4, 0]));
    col = col.push(row(CROSSFADE_CHOICES.iter().map(|&s| {
        let label = if s == 0 { "OFF".to_owned() } else { format!("{s}S") };
        Element::from(
            button(container(text(label).size(10)).width(Length::Fill).align_x(Alignment::Center))
                .width(Length::Fill)
                .padding([10, 4])
                .style(gain_style(cf == s))
                .on_press(Message::SetCrossfadeSeconds(s)),
        )
    })));
    let cf_hint = if cf == 0 {
        "OFF: TRACKS FOLLOW EACH OTHER WITHOUT A GAP.".to_owned()
    } else {
        format!("TRACKS OVERLAP FOR {cf} S. AN ALBUM PLAYED IN ORDER STAYS GAPLESS.")
    };
    col = col.push(hint(cf_hint, HINT, 6.0, 8.0));
    col.into()
}

fn device_row<'a>(d: &'a crate::controller::DeviceOption, selected: bool) -> Element<'a, Message> {
    let dot = container(space::horizontal())
        .width(8)
        .height(8)
        .style(move |_t| container::Style {
            background: Some(Background::Color(if selected { theme::GOLD } else { theme::LINE })),
            border: Border { radius: 4.0.into(), ..Default::default() },
            shadow: if selected {
                Shadow { color: theme::GOLD, offset: Vector::ZERO, blur_radius: 5.0 }
            } else {
                Shadow::default()
            },
            ..Default::default()
        });
    column![
        button(
            row![
                dot,
                text(trunc(&d.label, 36))
                    .size(12)
                    .font(iced::Font { weight: iced::font::Weight::Medium, ..iced::Font::DEFAULT })
                    .wrapping(text::Wrapping::None),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .padding(12)
        .style(device_style(selected))
        .on_press(Message::SelectDevice(d.clone())),
        hline(ROW_LINE),
    ]
    .into()
}

fn output_tab<'a>(app: &'a App) -> Element<'a, Message> {
    let current = app.audio_device.as_str();
    let mut col = column![container(header("SELECT OUTPUT", None)).padding([4, 0])];

    let shared: Vec<_> = app.devices.iter().filter(|d| !d.is_direct).collect();
    if shared.is_empty() {
        col = col.push(
            container(
                text("No devices found")
                    .size(12)
                    .font(iced::Font { style: iced::font::Style::Italic, ..iced::Font::DEFAULT })
                    .color(Color::from_rgb8(0x6B, 0x69, 0x63)),
            )
            .padding([8, 12]),
        );
    } else {
        // At most 240 px of shared devices, the rest scrolls (C# `MaxHeight="240"`)
        let list = column(shared.into_iter().map(|d| device_row(d, d.id == current)));
        col = col.push(container(smooth(scrollable(list).height(Length::Shrink))).max_height(240.0));
    }

    let direct: Vec<_> = app.devices.iter().filter(|d| d.is_direct).collect();
    if !direct.is_empty() {
        col = col.push(container(header("DIRECT OUTPUT // BIT-PERFECT", None)).padding([4, 0]));
        for d in direct {
            col = col.push(device_row(d, d.id == current));
        }
        if !app.bit_perfect_status.is_empty() {
            col = col.push(
                container(text(&app.bit_perfect_status).size(9).font(bold()).color(theme::TEAL).wrapping(text::Wrapping::Word))
                    .padding(Padding { top: 6.0, right: 12.0, bottom: 2.0, left: 12.0 })
                    .width(Length::Fill),
            );
        }
        if app.direct_active {
            col = col.push(hint(
                "THE CARD IS TAKEN FROM PIPEWIRE WHILE SELECTED. EQ, REPLAYGAIN AND CROSSFADE ARE OFF.",
                HINT,
                0.0,
                2.0,
            ));
            col = col.push(hint(&app.direct_volume_hint, HINT, 0.0, 8.0));
        }
    }

    let rg = app.settings.replay_gain.as_str();
    let rg_msg = |v: &str| Message::SelectReplayGain(ReplayGainOption(v.to_owned()));
    col = col.push(container(header("REPLAYGAIN", None)).padding([4, 0]));
    col = col.push(row![
        gain_cell("OFF", !matches!(rg, "track" | "album"), rg_msg("no")),
        gain_cell("TRACK", rg == "track", rg_msg("track")),
        gain_cell("ALBUM", rg == "album", rg_msg("album")),
    ]);
    let rg_hint = match rg {
        "track" => "TRACK: EVERY SONG AT THE SAME LOUDNESS. BEST FOR SHUFFLE.",
        "album" => "ALBUM: EACH ALBUM AT THE SAME LOUDNESS, ITS OWN DYNAMICS KEPT.",
        _ => "OFF: FILES PLAY AS MASTERED. TRACK / ALBUM EVEN OUT LOUDNESS USING REPLAYGAIN TAGS.",
    };
    col = col.push(hint(rg_hint, HINT, 6.0, 8.0));
    col.into()
}

// ---------- Equalizer bands (C# `EqSlider`) ----------

const EQ_HEIGHT: f32 = 132.0;
const EQ_RANGE: i32 = 12;
const DOUBLE_CLICK: std::time::Duration = std::time::Duration::from_millis(400);

/// The ten bands as vertical segment meters: a cell per dB from the 0 dB line, boosts upwards in
/// gold (orange in the top third), cuts downwards in teal, grey while the equalizer is off.
/// Drag or wheel in 1 dB steps; a double-click resets a band to 0.
struct EqBands {
    gains: [f64; 10],
    active: bool,
}

#[derive(Default)]
struct EqBandsState {
    dragging: Option<usize>,
    last_click: Option<(usize, std::time::Instant)>,
}

fn band_at(bounds: Rectangle, x: f32) -> usize {
    let col = bounds.width / 10.0;
    ((x / col.max(1.0)).floor().max(0.0) as usize).min(9)
}

/// The gain for a pointer at height `y` inside the meter (C# `EqSlider.SetFrom`).
fn gain_at(height: f32, y: f32) -> f64 {
    let mid = height / 2.0;
    let v = ((mid - y) / mid.max(1.0) * EQ_RANGE as f32).round();
    f64::from(v.clamp(-(EQ_RANGE as f32), EQ_RANGE as f32))
}

impl canvas::Program<Message> for EqBands {
    type State = EqBandsState;

    fn update(
        &self,
        state: &mut EqBandsState,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let set = |band: usize, gain: f64| Message::SetEqBand { band, gain };
        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = cursor.position_in(bounds)?;
                let band = band_at(bounds, p.x);
                let now = std::time::Instant::now();
                if state
                    .last_click
                    .is_some_and(|(b, at)| b == band && now.duration_since(at) <= DOUBLE_CLICK)
                {
                    state.last_click = None;
                    state.dragging = None;
                    return Some(canvas::Action::publish(set(band, 0.0)).and_capture());
                }
                state.last_click = Some((band, now));
                state.dragging = Some(band);
                let g = gain_at(bounds.height, p.y);
                if (g - self.gains[band]).abs() < 0.01 {
                    return Some(canvas::Action::capture());
                }
                Some(canvas::Action::publish(set(band, g)).and_capture())
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                let band = state.dragging?;
                let g = gain_at(bounds.height, position.y - bounds.y);
                if (g - self.gains[band]).abs() < 0.01 {
                    return None;
                }
                // Moving on is a drag, not the first half of a double-click.
                state.last_click = None;
                Some(canvas::Action::publish(set(band, g)).and_capture())
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.dragging.take().map(|_| canvas::Action::capture())
            }
            iced::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let p = cursor.position_in(bounds)?;
                let band = band_at(bounds, p.x);
                let dy = match delta {
                    mouse::ScrollDelta::Lines { y, .. } | mouse::ScrollDelta::Pixels { y, .. } => *y,
                };
                if dy == 0.0 {
                    return None;
                }
                let step = if dy > 0.0 { 1.0 } else { -1.0 };
                let g = (self.gains[band].round() + step).clamp(-f64::from(EQ_RANGE), f64::from(EQ_RANGE));
                Some(canvas::Action::publish(set(band, g)).and_capture())
            }
            _ => None,
        }
    }

    fn draw(&self, _s: &EqBandsState, renderer: &iced::Renderer, _t: &iced::Theme, bounds: Rectangle, _c: mouse::Cursor) -> Vec<canvas::Geometry> {
        let empty = Color::from_rgb8(0x22, 0x22, 0x25);
        let zero = Color::from_rgb8(0x56, 0x55, 0x4F);
        let off = Color::from_rgb8(0x46, 0x46, 0x4C);
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let cells = (EQ_RANGE * 2) as f32;
        let (gap, zero_line) = (1.0_f32, 2.0_f32);
        let cell = (bounds.height - zero_line - gap * cells) / cells;
        if cell <= 0.0 {
            return vec![frame.into_geometry()];
        }
        let mid = bounds.height / 2.0;
        let col_w = bounds.width / 10.0;
        let w = col_w.min(14.0);
        for (b, &gain) in self.gains.iter().enumerate() {
            let left = b as f32 * col_w + (col_w - w) / 2.0;
            let steps = gain.clamp(-f64::from(EQ_RANGE), f64::from(EQ_RANGE)).round() as i32;
            for i in 1..=EQ_RANGE {
                let up = mid - zero_line / 2.0 - i as f32 * (cell + gap);
                let down = mid + zero_line / 2.0 + gap + (i - 1) as f32 * (cell + gap);
                let up_color = if steps >= i {
                    if !self.active {
                        off
                    } else if i > EQ_RANGE * 2 / 3 {
                        theme::ORANGE
                    } else {
                        theme::GOLD
                    }
                } else {
                    empty
                };
                let down_color = if steps <= -i { if self.active { theme::TEAL } else { off } } else { empty };
                frame.fill_rectangle(Point::new(left, up + gap), Size::new(w, cell), up_color);
                frame.fill_rectangle(Point::new(left, down), Size::new(w, cell), down_color);
            }
            frame.fill_rectangle(Point::new(left - 2.0, mid - zero_line / 2.0), Size::new(w + 4.0, zero_line), zero);
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, state: &EqBandsState, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if state.dragging.is_some() || cursor.is_over(bounds) {
            mouse::Interaction::ResizingVertically
        } else {
            mouse::Interaction::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eq_pointer_maps_to_whole_db_like_csharp() {
        assert_eq!(gain_at(132.0, 0.0), 12.0);
        assert_eq!(gain_at(132.0, 66.0), 0.0);
        assert_eq!(gain_at(132.0, 132.0), -12.0);
        assert_eq!(gain_at(132.0, 200.0), -12.0);
        assert_eq!(gain_at(132.0, 60.5), 1.0);
        let b = Rectangle { x: 0.0, y: 0.0, width: 250.0, height: 132.0 };
        assert_eq!(band_at(b, 0.0), 0);
        assert_eq!(band_at(b, 249.0), 9);
        assert_eq!(band_at(b, 400.0), 9);
    }
}
