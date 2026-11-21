use std::time::{Duration, Instant};

use iced::widget::{button, container};
use iced::{Background, Border, Color, Shadow, Theme, Vector, theme};
use retrograde_core::CoverPalette;

pub const VOID: Color = Color::from_rgb(8.0 / 255.0, 8.0 / 255.0, 10.0 / 255.0);
pub const PANEL: Color = Color::from_rgb(18.0 / 255.0, 18.0 / 255.0, 18.0 / 255.0);
pub const PANEL_DEEP: Color = Color::from_rgb(13.0 / 255.0, 13.0 / 255.0, 15.0 / 255.0);
pub const CARD: Color = Color::from_rgb(22.0 / 255.0, 22.0 / 255.0, 24.0 / 255.0);
pub const RAISED: Color = Color::from_rgb(26.0 / 255.0, 26.0 / 255.0, 28.0 / 255.0);
pub const HOVER: Color = Color::from_rgb(38.0 / 255.0, 38.0 / 255.0, 42.0 / 255.0);
pub const LINE: Color = Color::from_rgb(46.0 / 255.0, 46.0 / 255.0, 51.0 / 255.0);
pub const LINE_SOFT: Color = Color::from_rgb(34.0 / 255.0, 34.0 / 255.0, 37.0 / 255.0);
pub const TEAL: Color = Color::from_rgb(79.0 / 255.0, 214.0 / 255.0, 190.0 / 255.0);
pub const ORANGE: Color = Color::from_rgb(1.0, 107.0 / 255.0, 53.0 / 255.0);
pub const GOLD: Color = Color::from_rgb(232.0 / 255.0, 192.0 / 255.0, 96.0 / 255.0);
pub const TEXT: Color = Color::from_rgb(236.0 / 255.0, 233.0 / 255.0, 225.0 / 255.0);
pub const TEXT_DIM: Color = Color::from_rgb(142.0 / 255.0, 140.0 / 255.0, 134.0 / 255.0);
pub const TEXT_MUTE: Color = Color::from_rgb(86.0 / 255.0, 85.0 / 255.0, 79.0 / 255.0);
#[allow(dead_code)]
pub const STARLIGHT: Color = Color::from_rgb(220.0 / 255.0, 214.0 / 255.0, 242.0 / 255.0);
pub const DANGER: Color = Color::from_rgb(224.0 / 255.0, 82.0 / 255.0, 82.0 / 255.0);

const FADE_DURATION: Duration = Duration::from_millis(400);

pub fn color_from_rgb8(rgb: [u8; 3]) -> Color {
    Color::from_rgb8(rgb[0], rgb[1], rgb[2])
}

#[derive(Debug, Clone)]
pub struct AccentAnimator {
    from_accent: Color,
    to_accent: Color,
    pub current_accent: Color,
    from_glow: Color,
    to_glow: Color,
    pub current_glow: Color,
    started_at: Option<Instant>,
    pub from_cover: bool,
}

impl Default for AccentAnimator {
    fn default() -> Self {
        let def = CoverPalette::ORBIT_DEFAULT;
        let accent = color_from_rgb8(def.accent);
        let glow = color_from_rgb8(def.glow);
        Self {
            from_accent: accent,
            to_accent: accent,
            current_accent: accent,
            from_glow: glow,
            to_glow: glow,
            current_glow: glow,
            started_at: None,
            from_cover: false,
        }
    }
}

impl AccentAnimator {
    pub fn set_target(&mut self, palette: CoverPalette, from_cover: bool, animate: bool) {
        let target_accent = color_from_rgb8(palette.accent);
        let target_glow = color_from_rgb8(palette.glow);
        self.from_cover = from_cover;

        if !animate || (approx_eq_color(self.current_accent, target_accent) && approx_eq_color(self.current_glow, target_glow)) {
            self.from_accent = target_accent;
            self.to_accent = target_accent;
            self.current_accent = target_accent;
            self.from_glow = target_glow;
            self.to_glow = target_glow;
            self.current_glow = target_glow;
            self.started_at = None;
            return;
        }

        self.from_accent = self.current_accent;
        self.to_accent = target_accent;
        self.from_glow = self.current_glow;
        self.to_glow = target_glow;
        self.started_at = Some(Instant::now());
    }

    pub fn is_animating(&self) -> bool {
        self.started_at.is_some()
    }

    pub fn tick(&mut self) -> bool {
        let Some(start) = self.started_at else {
            return false;
        };
        let p = (start.elapsed().as_secs_f32() / FADE_DURATION.as_secs_f32()).clamp(0.0, 1.0);
        let ease = 1.0 - (1.0 - p).powi(3);
        self.current_accent = lerp_color(self.from_accent, self.to_accent, ease);
        self.current_glow = lerp_color(self.from_glow, self.to_glow, ease);
        if p >= 1.0 {
            self.current_accent = self.to_accent;
            self.current_glow = self.to_glow;
            self.started_at = None;
        }
        self.started_at.is_some()
    }
}

fn approx_eq_color(a: Color, b: Color) -> bool {
    (a.r - b.r).abs() < 0.002 && (a.g - b.g).abs() < 0.002 && (a.b - b.b).abs() < 0.002
}

fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    Color::from_rgba(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
        a.a + (b.a - a.a) * t,
    )
}

pub fn with_alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

pub fn orbit_theme(accent: Color) -> Theme {
    Theme::custom(
        "Retrograde Orbit",
        theme::Palette {
            background: VOID,
            text: TEXT,
            primary: accent,
            success: TEAL,
            warning: GOLD,
            danger: DANGER,
        },
    )
}

pub fn modal_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(RAISED)),
        border: Border {
            color: LINE,
            width: 1.0,
            radius: 0.0.into(),
        },
        text_color: Some(TEXT),
        shadow: Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.65),
            offset: Vector::new(0.0, 8.0),
            blur_radius: 24.0,
        },
        ..Default::default()
    }
}

pub fn badge_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(RAISED)),
        border: Border {
            color: LINE,
            width: 1.0,
            radius: 5.0.into(),
        },
        text_color: Some(TEXT_DIM),
        ..Default::default()
    }
}

pub fn accent_badge_style(accent: Color) -> impl Fn(&Theme) -> container::Style {
    move |_theme| container::Style {
        background: Some(Background::Color(with_alpha(accent, 0.14))),
        border: Border {
            color: with_alpha(accent, 0.55),
            width: 1.0,
            radius: 5.0.into(),
        },
        text_color: Some(accent),
        ..Default::default()
    }
}

pub fn ghost_button_style(active: bool, _accent: Color) -> impl Fn(&Theme, button::Status) -> button::Style {
    // Like the chips, but with readable text: these are working buttons, not labels
    move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if active {
                with_alpha(GOLD, 0.10)
            } else if hovered {
                Color::from_rgba(1.0, 1.0, 1.0, 0.06)
            } else {
                Color::TRANSPARENT
            })),
            text_color: if active { GOLD } else if hovered { TEXT } else { TEXT_DIM },
            border: Border { color: if active { GOLD } else { LINE }, width: 1.0, radius: 0.0.into() },
            ..Default::default()
        }
    }
}

fn luminance(c: Color) -> f32 {
    fn lin(v: f32) -> f32 {
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    }
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

/// WCAG contrast ratio of two colors.
fn contrast(a: Color, b: Color) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The text color (light or dark) that reads better on `bg`.
#[allow(dead_code)]
pub fn readable_text_on(bg: Color) -> Color {
    if contrast(bg, VOID) >= contrast(bg, TEXT) { VOID } else { TEXT }
}

pub fn accent_button_style(_accent: Color) -> impl Fn(&Theme, button::Status) -> button::Style {
    // The main action is an off-white block with dark text, like the original's play button
    move |_theme, status| {
        let bg = match status {
            button::Status::Hovered => Color::WHITE,
            button::Status::Pressed => Color::from_rgb8(0xD8, 0xD5, 0xCD),
            _ => TEXT,
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: VOID,
            border: Border { color: Color::TRANSPARENT, width: 0.0, radius: 0.0.into() },
            ..Default::default()
        }
    }
}

pub fn album_card_style(selected: bool, _accent: Color) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if selected || hovered { RAISED } else { CARD })),
            text_color: TEXT,
            border: Border {
                color: if selected || hovered { GOLD } else { LINE_SOFT },
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        }
    }
}

/// A flat nav row: dim until hovered, bright and underlined by the accent when it is the current page.
pub fn nav_item_style(active: bool, accent: Color) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if active {
                with_alpha(accent, 0.10)
            } else if hovered {
                Color::from_rgba(1.0, 1.0, 1.0, 0.05)
            } else {
                Color::TRANSPARENT
            })),
            text_color: if active || hovered { TEXT } else { TEXT_DIM },
            border: Border { color: Color::TRANSPARENT, width: 0.0, radius: 0.0.into() },
            ..Default::default()
        }
    }
}

/// The small outlined mode chips (ALBUMS / ARTISTS, PLAYLISTS / ALBUMS): gold when on.
pub fn chip_style(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if active { with_alpha(GOLD, 0.10) } else { Color::TRANSPARENT })),
            text_color: if active { GOLD } else if hovered { TEXT } else { TEXT_MUTE },
            border: Border { color: if active { GOLD } else { LINE_SOFT }, width: 1.0, radius: 0.0.into() },
            ..Default::default()
        }
    }
}

/// A button with no chrome (icons in the transport bar): the icon colour is set where it is drawn.
pub fn bare_button_style() -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if hovered { Color::from_rgba(1.0, 1.0, 1.0, 0.06) } else { Color::TRANSPARENT })),
            text_color: TEXT,
            border: Border { color: Color::TRANSPARENT, width: 0.0, radius: 2.0.into() },
            ..Default::default()
        }
    }
}

/// A slider that draws nothing: a plain hit area laid over a custom-drawn bar.
pub fn invisible_slider_style(_theme: &Theme, _status: iced::widget::slider::Status) -> iced::widget::slider::Style {
    use iced::widget::slider::{Handle, HandleShape, Rail, Style};
    Style {
        rail: Rail {
            backgrounds: (Background::Color(Color::TRANSPARENT), Background::Color(Color::TRANSPARENT)),
            width: 1.0,
            border: Border::default(),
        },
        handle: Handle {
            shape: HandleShape::Rectangle { width: 1, border_radius: 0.0.into() },
            background: Background::Color(Color::TRANSPARENT),
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        },
    }
}

/// A flat rectangle of one color (stripes, meter segments, dividers).
pub fn fill_style(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_theme| container::Style { background: Some(Background::Color(color)), ..Default::default() }
}

/// Dropdown field: a dark box with a thin outline that turns gold when open.
pub fn pick_style(_theme: &Theme, status: iced::widget::pick_list::Status) -> iced::widget::pick_list::Style {
    use iced::widget::pick_list::Status;
    let open = matches!(status, Status::Opened { .. });
    let hovered = matches!(status, Status::Hovered | Status::Opened { .. });
    iced::widget::pick_list::Style {
        text_color: TEXT,
        placeholder_color: TEXT_MUTE,
        handle_color: if hovered { GOLD } else { TEXT_DIM },
        background: Background::Color(VOID),
        border: Border { color: if open { GOLD } else if hovered { TEXT_MUTE } else { LINE }, width: 1.0, radius: 0.0.into() },
    }
}

pub fn pick_menu_style(_theme: &Theme) -> iced::overlay::menu::Style {
    iced::overlay::menu::Style {
        background: Background::Color(Color::from_rgb8(0x12, 0x12, 0x12)),
        border: Border { color: LINE, width: 1.0, radius: 0.0.into() },
        text_color: TEXT_DIM,
        selected_text_color: VOID,
        selected_background: Background::Color(GOLD),
        shadow: Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.6), offset: Vector::new(0.0, 8.0), blur_radius: 20.0 },
    }
}

/// On/off switch: square, teal when on.
pub fn toggler_style(_theme: &Theme, status: iced::widget::toggler::Status) -> iced::widget::toggler::Style {
    use iced::widget::toggler::Status;
    let on = matches!(status, Status::Active { is_toggled: true } | Status::Hovered { is_toggled: true });
    iced::widget::toggler::Style {
        background: Background::Color(if on { with_alpha(TEAL, 0.25) } else { VOID }),
        background_border_width: 1.0,
        background_border_color: if on { TEAL } else { LINE },
        foreground: Background::Color(if on { TEAL } else { TEXT_MUTE }),
        foreground_border_width: 0.0,
        foreground_border_color: Color::TRANSPARENT,
        text_color: None,
        border_radius: Some(2.0.into()),
        padding_ratio: 0.2,
    }
}

/// Text field: dark, thin outline, gold when focused.
pub fn input_style(_theme: &Theme, status: iced::widget::text_input::Status) -> iced::widget::text_input::Style {
    use iced::widget::text_input::Status;
    iced::widget::text_input::Style {
        background: Background::Color(VOID),
        border: Border {
            color: match status {
                Status::Focused { .. } => GOLD,
                Status::Hovered => TEXT_MUTE,
                _ => LINE,
            },
            width: 1.0,
            radius: 0.0.into(),
        },
        icon: TEXT_DIM,
        placeholder: TEXT_MUTE,
        value: TEXT,
        selection: with_alpha(GOLD, 0.3),
    }
}

/// Inserts U+200A hair spaces between characters for wide HUD labels where Iced lacks native tracking.
/// Apply only to short static UI headers, never user metadata.
pub fn tracked(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 4);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 {
            out.push('\u{200A}');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracked_inserts_hair_space_between_characters() {
        assert_eq!(tracked(""), "");
        assert_eq!(tracked("A"), "A");
        assert_eq!(tracked("AB"), "A\u{200A}B");
        assert_eq!(tracked("SYSTEM"), "S\u{200A}Y\u{200A}S\u{200A}T\u{200A}E\u{200A}M");
    }

    #[test]
    fn accent_button_text_is_readable_for_default_and_extreme_accents() {
        for rgb in [(255u8, 107u8, 53u8), (60, 120, 255), (150, 90, 230), (255, 255, 255), (40, 40, 40)] {
            let bg = Color::from_rgb8(rgb.0, rgb.1, rgb.2);
            assert!(contrast(bg, readable_text_on(bg)) >= 4.5, "{rgb:?}");
        }
    }

    #[test]
    fn accent_animator_snaps_without_animation_and_interpolates_with_animation() {
        let mut anim = AccentAnimator::default();
        let target = CoverPalette {
            accent: [40, 200, 160],
            glow: [20, 120, 100],
            vivid: true,
        };
        anim.set_target(target, true, false);
        assert!(!anim.is_animating());
        assert!(approx_eq_color(anim.current_accent, color_from_rgb8(target.accent)));

        anim.set_target(CoverPalette::ORBIT_DEFAULT, false, true);
        assert!(anim.is_animating());
        assert!(anim.tick());
    }
}
