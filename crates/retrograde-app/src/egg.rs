use iced::alignment::Horizontal;
use iced::widget::canvas::{self, Canvas};
use iced::widget::text::Wrapping;
use iced::widget::{button, column, container, image, mouse_area, opaque, row, space, stack, text};
use iced::{
    Alignment, Background, Border, Color, ContentFit, Element, Length, Padding, Point, Rectangle,
    Renderer, Shadow, Size, Theme, Vector, mouse,
};

use crate::icons;
use crate::theme;
use crate::views::bold;
use crate::{App, Message};

/// Morse code for `LONETRAIL`: typing it on the sidebar visualizer unlocks `PRIVATE_MEMORY_BANK`.
pub const TARGET_MORSE: &str = ".-.. --- -. . - .-. .- .. .-..";

/// Resting bar heights as percentages of the 32 px visualizer strip (same order as Electron / C#).
pub const BAR_HEIGHT_PCTS: [f32; 12] = [
    25.0, 45.0, 20.0, 55.0, 30.0, 60.0, 15.0, 40.0, 25.0, 50.0, 20.0, 35.0,
];

const KRISTEN_PNG: &[u8] = include_bytes!("../../../assets/kristen.png");

#[derive(Clone, Debug)]
pub struct EggAssets {
    pub color: image::Handle,
    pub gray: image::Handle,
}

#[derive(Debug, Default)]
pub struct EggMorseState {
    buffer: String,
    generation: u64,
    bar_tokens: [u64; 12],
}

impl EggMorseState {
    /// Registers a press on visualizer bar `index` and returns `(bar_token, timeout_gen, unlocked)`.
    pub fn press(&mut self, index: usize) -> (u64, u64, bool) {
        self.generation = self.generation.wrapping_add(1).max(1);
        let token = self.generation;
        if index < self.bar_tokens.len() {
            self.bar_tokens[index] = token;
        }
        self.buffer.push(bar_symbol(index));
        if self.buffer.len() > 96 {
            let cut = self.buffer.len() - 64;
            self.buffer.drain(..cut);
        }
        if self.buffer.contains(TARGET_MORSE) {
            self.buffer.clear();
            // Advance generation so any pending timeout from an earlier tap is ignored.
            self.generation = self.generation.wrapping_add(1).max(1);
            return (token, 0, true);
        }
        (token, self.generation, false)
    }

    /// Clears the active flash for `index` when `token` matches the latest press on that bar.
    pub fn release_bar(&mut self, index: usize, token: u64) -> bool {
        if index < self.bar_tokens.len() && self.bar_tokens[index] == token {
            self.bar_tokens[index] = 0;
            true
        } else {
            false
        }
    }

    /// Clears the partial Morse sequence when no newer tap happened before the timeout fired.
    pub fn timeout(&mut self, generation: u64) {
        if generation != 0 && self.generation == generation {
            self.buffer.clear();
        }
    }

    pub fn active_bars(&self) -> [bool; 12] {
        self.bar_tokens.map(|t| t != 0)
    }
}

/// Morse character emitted by clicking visualizer bar `index` (`Sidebar.jsx`).
pub fn bar_symbol(index: usize) -> char {
    match index % 3 {
        0 => '.',
        1 => ' ',
        _ => '-',
    }
}

/// Lit color when clicking visualizer bar `index`.
pub fn bar_active_color(index: usize) -> Color {
    match index % 3 {
        0 => theme::TEAL,
        1 => theme::GOLD,
        _ => theme::ORANGE,
    }
}

/// Decodes the embedded Kristen image into color and Rec. 709 grayscale handles off the UI thread.
pub fn decode_egg_assets() -> Option<EggAssets> {
    let decoded = ::image::load_from_memory(KRISTEN_PNG).ok()?.to_rgba8();
    let (w, h) = decoded.dimensions();
    let raw = decoded.into_raw();
    let mut gray = Vec::with_capacity(raw.len());
    for px in raw.as_chunks::<4>().0 {
        let luma = (0.2126 * f32::from(px[0])
            + 0.7152 * f32::from(px[1])
            + 0.0722 * f32::from(px[2]))
        .round()
        .clamp(0.0, 255.0) as u8;
        gray.extend_from_slice(&[luma, luma, luma, px[3]]);
    }
    Some(EggAssets {
        color: image::Handle::from_rgba(w, h, raw),
        gray: image::Handle::from_rgba(w, h, gray),
    })
}

struct ScanlinesCanvas;

impl<M> canvas::Program<M> for ScanlinesCanvas {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let line_color = Color::from_rgba8(0, 0, 0, 0.25);
        let mut y = 1.0f32;
        while y < bounds.height {
            frame.fill_rectangle(
                Point::new(0.0, y),
                Size::new(bounds.width, 1.0),
                line_color,
            );
            y += 2.0;
        }
        vec![frame.into_geometry()]
    }
}

struct EggCornersCanvas;

impl<M> canvas::Program<M> for EggCornersCanvas {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let len = 16.0;
        let thick = 2.0;
        // Top-left bracket
        frame.fill_rectangle(Point::new(0.0, 0.0), Size::new(len, thick), theme::ORANGE);
        frame.fill_rectangle(Point::new(0.0, 0.0), Size::new(thick, len), theme::ORANGE);
        // Bottom-right bracket
        let rx = (bounds.width - len).max(0.0);
        let ry = (bounds.height - len).max(0.0);
        frame.fill_rectangle(
            Point::new(rx, (bounds.height - thick).max(0.0)),
            Size::new(len, thick),
            theme::ORANGE,
        );
        frame.fill_rectangle(
            Point::new((bounds.width - thick).max(0.0), ry),
            Size::new(thick, len),
            theme::ORANGE,
        );
        vec![frame.into_geometry()]
    }
}

pub fn view_egg_modal<'a>(app: &'a App) -> Element<'a, Message> {
    let header_bar = container(
        row![
            icons::icon("terminal", 12.0, theme::TEAL),
            text(theme::tracked("PRIVATE_MEMORY_BANK // -.- .-. .. ... - . -."))
                .size(10)
                .color(theme::TEAL)
                .width(Length::Fill),
            button(icons::icon_hover(
                "x",
                16.0,
                Color::from_rgb8(0x56, 0x55, 0x4F),
                theme::ORANGE,
            ))
            .padding(2)
            .style(theme::bare_button_style())
            .on_press(Message::CloseEgg),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding([8, 10])
    .width(Length::Fill)
    .style(|_t| container::Style {
        background: Some(Background::Color(Color::from_rgb8(0x12, 0x12, 0x12))),
        border: Border {
            color: theme::LINE,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    });

    let pic_content: Element<'a, Message> = if let Some(assets) = &app.egg_assets {
        let mut layers: Vec<Element<'a, Message>> = vec![
            image(assets.gray.clone())
                .content_fit(ContentFit::Cover)
                .width(Length::Fill)
                .height(Length::Fill)
                .opacity(0.8_f32)
                .into(),
        ];
        if app.egg_color_alpha > 0.001 {
            layers.push(
                image(assets.color.clone())
                    .content_fit(ContentFit::Cover)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .opacity(app.egg_color_alpha.clamp(0.0, 1.0))
                    .into(),
            );
        }
        layers.push(
            Canvas::new(ScanlinesCanvas)
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
        );
        mouse_area(
            container(stack(layers))
                .width(Length::Fill)
                .height(Length::Fixed(378.0))
                .clip(true)
                .style(|_t| container::Style {
                    border: Border {
                        color: theme::LINE,
                        width: 1.0,
                        radius: 0.0.into(),
                    },
                    ..Default::default()
                }),
        )
        .on_enter(Message::EggImageHover(true))
        .on_exit(Message::EggImageHover(false))
        .into()
    } else {
        let msg = if app.egg_loading {
            "DECODING_MEMORY_BANK..."
        } else {
            "MEMORY_BANK_UNAVAILABLE"
        };
        container(
            text(theme::tracked(msg))
                .size(10)
                .color(theme::TEXT_MUTE),
        )
        .width(Length::Fill)
        .height(Length::Fixed(378.0))
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(|_t| container::Style {
            background: Some(Background::Color(theme::VOID)),
            border: Border {
                color: theme::LINE,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
    };

    let pulse_alpha = if app.render_tier.enable_animations {
        0.55 + 0.45 * (0.5 + 0.5 * app.egg_pulse_phase.sin())
    } else {
        1.0
    };
    let footer = container(
        column![
            text(theme::tracked("\"GOOD NIGHT, KRISTEN\""))
                .size(24)
                .font(bold())
                .color(Color::from_rgba(1.0, 1.0, 1.0, pulse_alpha))
                .align_x(Horizontal::Center)
                .wrapping(Wrapping::Word),
            container(space::horizontal())
                .width(48)
                .height(1)
                .style(theme::fill_style(theme::ORANGE)),
            text(theme::tracked(
                "IF, IN A CENTURY OR A MILLENIUM, OUR DESCENDANTS WALK AMONG THE STARS, THE MASSES WILL SING HER PRAISES."
            ))
            .size(10)
            .color(Color::from_rgb8(0x56, 0x55, 0x4F))
            .align_x(Horizontal::Center)
            .wrapping(Wrapping::Word),
        ]
        .spacing(12)
        .align_x(Alignment::Center),
    )
    .padding(Padding {
        top: 28.0,
        right: 28.0,
        bottom: 28.0,
        left: 28.0,
    })
    .width(Length::Fill)
    .style(|_t| container::Style {
        background: Some(Background::Color(Color::from_rgb8(0x12, 0x12, 0x12))),
        ..Default::default()
    });

    let inner = container(column![header_bar, pic_content, footer].width(Length::Fill))
        .width(Length::Fill)
        .style(|_t| container::Style {
            background: Some(Background::Color(Color::from_rgb8(0x08, 0x08, 0x0A))),
            ..Default::default()
        });

    let teal_frame = container(crate::motion::enter(
        "egg-card",
        crate::motion::EnterSpec::new(1000, crate::motion::Ease::Linear, 0.0, crate::motion::Fade::Cover(Color::from_rgb8(0x08, 0x08, 0x0A))),
        inner,
    ))
        .padding(4)
        .width(Length::Fixed(512.0))
        .style(|_t| container::Style {
            background: Some(Background::Color(Color::from_rgb8(0x1A, 0x1A, 0x1C))),
            border: Border {
                color: theme::TEAL,
                width: 1.0,
                radius: 0.0.into(),
            },
            shadow: Shadow {
                color: Color::from_rgba8(0x4F, 0xD6, 0xBE, 0.30),
                offset: Vector::ZERO,
                blur_radius: 50.0,
            },
            ..Default::default()
        });

    let card = opaque(
        container(stack![
            container(teal_frame).padding(8),
            Canvas::new(EggCornersCanvas)
                .width(Length::Fill)
                .height(Length::Fill),
        ])
        .width(Length::Fixed(528.0)),
    );

    // Border.egg[IsVisible=True]: fades in over 1 s (the backdrop dims in, the card's content fades in
    // over its own color).
    let backdrop = crate::motion::enter(
        "egg",
        crate::motion::EnterSpec::new(1000, crate::motion::Ease::Linear, 0.0, crate::motion::Fade::Scrim(Color::from_rgba8(0, 0, 0, 0.95))),
        container(card)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: 16.0,
                right: 16.0,
                bottom: 48.0,
                left: 16.0,
            })
            .align_x(Alignment::Center)
            .align_y(Alignment::Center),
    );

    opaque(mouse_area(backdrop).on_press(Message::CloseEgg))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tap_morse(state: &mut EggMorseState, pattern: &str) -> bool {
        let mut unlocked = false;
        for ch in pattern.chars() {
            let idx = match ch {
                '.' => 0,
                ' ' => 1,
                '-' => 2,
                _ => panic!("unexpected morse char {ch}"),
            };
            let (_, _, matched) = state.press(idx);
            if matched {
                unlocked = true;
            }
        }
        unlocked
    }

    #[test]
    fn morse_lonetrail_triggers_and_resets() {
        let mut state = EggMorseState::default();
        // Leading noise taps before typing LONETRAIL
        assert!(!tap_morse(&mut state, "..- "));
        assert!(tap_morse(&mut state, TARGET_MORSE));
        assert!(state.buffer.is_empty());
        // Does not stay latched after match
        assert!(!tap_morse(&mut state, "."));
    }

    #[test]
    fn morse_timeout_clears_partial_sequence() {
        let mut state = EggMorseState::default();
        let (_, g1, m1) = state.press(0);
        assert!(!m1);
        let (_, g2, m2) = state.press(2);
        assert!(!m2);
        // Stale timer for g1 must not clear the buffer
        state.timeout(g1);
        assert_eq!(state.buffer, ".-");
        // Current generation timer clears it
        state.timeout(g2);
        assert!(state.buffer.is_empty());
    }

    #[test]
    fn bar_release_respects_latest_token() {
        let mut state = EggMorseState::default();
        let (t1, _, _) = state.press(2);
        assert!(state.active_bars()[2]);
        let (t2, _, _) = state.press(2);
        assert!(!state.release_bar(2, t1));
        assert!(state.active_bars()[2]);
        assert!(state.release_bar(2, t2));
        assert!(!state.active_bars()[2]);
    }

    #[test]
    fn decode_egg_assets_loads_kristen_and_builds_grayscale() {
        let assets = decode_egg_assets().expect("embedded kristen.png should decode");
        assert!(matches!(assets.color, image::Handle::Rgba { .. }));
        assert!(matches!(assets.gray, image::Handle::Rgba { .. }));
    }
}
