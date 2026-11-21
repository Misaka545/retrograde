//! Toasts and the scan progress card, like the original's: a stack of at most five notes at the bottom right
//! that slide in from the right (0.3 s) and out again (0.4 s), errors staying up 6 s and the rest 3 s, and while a
//! scan runs a card with its phase and a progress bar.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use iced::widget::canvas::{self, Canvas, Path, Stroke};
use iced::widget::{column, container, float, row, space, text};
use iced::{Alignment, Background, Border, Color, Element, Length, Padding, Point, Rectangle, Renderer, Task, Theme, Vector, mouse};

use crate::controller::NoticeKind;
use crate::theme;
use crate::views::bold;
use crate::{App, Message};

const ENTER_MS: f32 = 300.0;
const EXIT_MS: f32 = 400.0;
const MAX_TOASTS: usize = 5;

#[derive(Debug, Clone)]
struct Toast {
    id: u64,
    kind: NoticeKind,
    text: String,
    time_label: String,
    born: Instant,
    exiting_since: Option<Instant>,
}

/// Formats the current local time as `HH:MM:SS` without pulling in an external time crate.
#[cfg(unix)]
fn format_local_hms() -> String {
    #[repr(C)]
    #[derive(Default)]
    struct Tm {
        tm_sec: i32,
        tm_min: i32,
        tm_hour: i32,
        tm_mday: i32,
        tm_mon: i32,
        tm_year: i32,
        tm_wday: i32,
        tm_yday: i32,
        tm_isdst: i32,
        tm_gmtoff: isize,
        tm_zone: *const std::ffi::c_char,
    }
    unsafe extern "C" {
        fn localtime_r(timep: *const i64, result: *mut Tm) -> *mut Tm;
    }
    let Ok(dur) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return String::new();
    };
    let secs = dur.as_secs() as i64;
    let mut tm = Tm::default();
    let ptr = unsafe { localtime_r(&raw const secs, &raw mut tm) };
    if ptr.is_null() {
        String::new()
    } else {
        format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
    }
}

#[cfg(not(unix))]
fn format_local_hms() -> String {
    String::new()
}

#[derive(Debug, Default)]
pub struct Toasts {
    items: Vec<Toast>,
    next_id: u64,
}

impl Toasts {
    /// Adds a toast and schedules its exit (6 s for errors, 3 s otherwise).
    pub fn push(&mut self, kind: NoticeKind, text: String) -> Task<Message> {
        self.next_id += 1;
        let id = self.next_id;
        self.items.push(Toast {
            id,
            kind,
            text,
            time_label: format_local_hms(),
            born: Instant::now(),
            exiting_since: None,
        });
        while self.items.len() > MAX_TOASTS {
            self.items.remove(0);
        }
        let secs = if kind == NoticeKind::Error { 6 } else { 3 };
        Task::perform(tokio::time::sleep(Duration::from_secs(secs)), move |_| Message::ToastExpire(id))
    }

    /// Starts the slide-out; the toast leaves the list once it has played.
    pub fn expire(&mut self, id: u64) -> Task<Message> {
        let Some(t) = self.items.iter_mut().find(|t| t.id == id) else { return Task::none() };
        t.exiting_since = Some(Instant::now());
        Task::perform(tokio::time::sleep(Duration::from_millis(EXIT_MS as u64)), move |_| Message::ToastRemove(id))
    }

    pub fn remove(&mut self, id: u64) {
        self.items.retain(|t| t.id != id);
    }

    /// True while a toast slides in or out (the window then redraws every frame).
    pub fn animating(&self) -> bool {
        let now = Instant::now();
        self.items.iter().any(|t| {
            t.exiting_since.is_some() || now.saturating_duration_since(t.born).as_secs_f32() * 1000.0 < ENTER_MS
        })
    }
}

/// A scan in progress: the scanner's phase and its counts.
#[derive(Debug, Clone)]
pub struct ScanState {
    pub phase: &'static str,
    pub scanned: usize,
    pub total: usize,
    pub started: Instant,
}

fn scan_title(s: &ScanState) -> String {
    match s.phase {
        "discovering" => "DISCOVERING_FILES...".to_owned(),
        "scanning" => format!("SCANNING_METADATA [{}/{}]", s.scanned, s.total),
        "covers" => format!("EXTRACTING_COVERS [{}/{}]", s.scanned, s.total),
        _ => "SYSTEM_SCANNING...".to_owned(),
    }
}

fn kind_color(kind: NoticeKind) -> Color {
    match kind {
        NoticeKind::Warn => theme::GOLD,
        NoticeKind::Error => theme::ORANGE,
        NoticeKind::Info => theme::TEXT_DIM,
        NoticeKind::Success => theme::TEAL,
    }
}

fn kind_label(kind: NoticeKind) -> (&'static str, &'static str) {
    match kind {
        NoticeKind::Warn => ("[WARNING]", "⚠"),
        NoticeKind::Error => ("[ERROR]", "✗"),
        NoticeKind::Info => ("[INFO]", "−"),
        NoticeKind::Success => ("[SUCCESS]", "✓"),
    }
}

fn fade(c: Color, a: f32) -> Color {
    Color { a: c.a * a, ..c }
}

/// Where a toast is in its slide: horizontal offset and opacity.
fn slide(t: &Toast, now: Instant) -> (f32, f32) {
    if let Some(since) = t.exiting_since {
        let p = (now.saturating_duration_since(since).as_secs_f32() * 1000.0 / EXIT_MS).clamp(0.0, 1.0);
        let e = p * p * p;
        return (450.0 * e, 1.0 - e);
    }
    let p = (now.saturating_duration_since(t.born).as_secs_f32() * 1000.0 / ENTER_MS).clamp(0.0, 1.0);
    let e = 1.0 - (1.0 - p).powi(3);
    (380.0 * (1.0 - e), e)
}

fn toast_card<'a>(t: &'a Toast, now: Instant) -> Element<'a, Message> {
    let (dx, a) = slide(t, now);
    let accent = kind_color(t.kind);
    let (label, glyph) = kind_label(t.kind);
    let line = container(space::horizontal()).width(Length::Fill).height(2).style(move |_t| container::Style {
        background: Some(Background::Gradient(
            iced::gradient::Linear::new(iced::Radians(std::f32::consts::FRAC_PI_2))
                .add_stop(0.0, fade(accent, a))
                .add_stop(1.0, Color::TRANSPARENT)
                .into(),
        )),
        ..Default::default()
    });
    let icon = container(text(glyph).size(11).font(bold()).color(fade(accent, a)))
        .width(20)
        .height(20)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(move |_t| container::Style {
            background: Some(Background::Color(fade(accent, 0.07 * a))),
            border: Border { color: fade(accent, 0.27 * a), width: 1.0, radius: 0.0.into() },
            ..Default::default()
        });
    let body = row![
        icon,
        column![
            row![
                text(label).size(10).font(bold()).color(fade(accent, a)),
                space::horizontal(),
                text(&t.time_label)
                    .size(8)
                    .color(fade(Color::from_rgb8(0x56, 0x55, 0x4F), a)),
            ]
            .align_y(Alignment::Center)
            .width(Length::Fill),
            text(crate::pages::fit(&t.text, 290.0, 11.0))
                .size(11)
                .color(fade(Color::from_rgb8(0xCF, 0xCC, 0xC4), a))
                .wrapping(iced::widget::text::Wrapping::None),
        ]
        .spacing(2)
        .width(Length::Fill),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .padding([12, 16]);
    let card = container(column![line, body])
        .width(Length::Fill)
        .style(move |_t| container::Style {
            background: Some(Background::Color(Color::from_rgba8(0x12, 0x12, 0x12, 0.95 * a))),
            border: Border { color: fade(accent, 0.2 * a), width: 1.0, radius: 0.0.into() },
            ..Default::default()
        });
    if dx > 0.5 {
        float(card).translate(move |_b, _v| Vector::new(dx, 0.0)).into()
    } else {
        card.into()
    }
}

/// The scan card's spinner: a three-quarter ring that turns once a second.
struct Spinner {
    turn: f32,
}

impl<M> canvas::Program<M> for Spinner {
    type State = ();

    fn draw(&self, _s: &(), renderer: &Renderer, _t: &Theme, bounds: Rectangle, _c: mouse::Cursor) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let c = Point::new(bounds.width / 2.0, bounds.height / 2.0);
        let start = self.turn * std::f32::consts::TAU;
        let arc = Path::new(|p| {
            p.arc(canvas::path::Arc {
                center: c,
                radius: bounds.width / 2.0 - 1.5,
                start_angle: iced::Radians(start),
                end_angle: iced::Radians(start + std::f32::consts::TAU * 0.75),
            });
        });
        frame.stroke(&arc, Stroke::default().with_color(theme::TEAL).with_width(2.0));
        vec![frame.into_geometry()]
    }
}

fn scan_card<'a>(s: &ScanState) -> Element<'a, Message> {
    let turn = s.started.elapsed().as_secs_f32().fract();
    let mut col = column![
        row![
            Canvas::new(Spinner { turn }).width(16).height(16),
            text(scan_title(s)).size(12).font(bold()).color(theme::TEAL),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    ]
    .spacing(12);
    if s.total > 0 {
        let done = ((s.scanned as f32 / s.total as f32).clamp(0.0, 1.0) * 1000.0) as u16;
        let bar = row![
            container(space::horizontal()).width(Length::FillPortion(done.max(1))).height(3).style(|_t| container::Style {
                background: Some(Background::Gradient(
                    iced::gradient::Linear::new(iced::Radians(std::f32::consts::FRAC_PI_2))
                        .add_stop(0.0, theme::TEAL)
                        .add_stop(1.0, theme::ORANGE)
                        .into(),
                )),
                ..Default::default()
            }),
            container(space::horizontal())
                .width(Length::FillPortion(1000 - done.clamp(1, 999)))
                .height(3)
                .style(theme::fill_style(theme::LINE_SOFT)),
        ];
        col = col.push(bar);
    }
    container(col)
        .padding(16)
        .width(320)
        .style(|_t| container::Style {
            background: Some(Background::Color(Color::from_rgba8(0x12, 0x12, 0x12, 0.95))),
            border: Border { color: theme::LINE, width: 1.0, radius: 4.0.into() },
            ..Default::default()
        })
        .into()
}

/// The toasts and the scan card over the bottom right of the window. Nothing here takes clicks.
pub fn layer(app: &App) -> Option<Element<'_, Message>> {
    if app.toasts.items.is_empty() && app.scan.is_none() {
        return None;
    }
    let now = Instant::now();
    let mut col = column![].spacing(8).align_x(Alignment::End);
    if !app.toasts.items.is_empty() {
        let cards: Vec<Element<'_, Message>> = app.toasts.items.iter().map(|t| toast_card(t, now)).collect();
        col = col.push(container(column(cards).spacing(8)).width(360));
    }
    if let Some(s) = &app.scan {
        col = col.push(scan_card(s));
    }
    let right = if app.queue_open || app.audio_open { 364.0 } else { 24.0 };
    Some(
        container(col)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::End)
            .align_y(Alignment::End)
            .padding(Padding { top: 0.0, right, bottom: 112.0, left: 0.0 })
            .into(),
    )
}
