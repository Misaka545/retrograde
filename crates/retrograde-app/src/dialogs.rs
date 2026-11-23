//! The original's modal dialogs: the WARNING confirm (missing files, deleting a playlist or smart playlist) and
//! BACKGROUND_PLAY, asked on the first close. Both use the same frame: a header strip with an orange square, the
//! body, a footer with CANCEL and a gold button, and an orange corner at the bottom right.

use std::path::PathBuf;

use iced::widget::{button, column, container, mouse_area, opaque, row, space, stack, text};
use iced::{Alignment, Background, Border, Color, Element, Length};

use crate::theme;
use crate::views::bold;
use crate::Message;

/// What a confirmed WARNING dialog does.
#[derive(Debug, Clone)]
pub enum ConfirmAction {
    /// Scan these roots again and drop the tracks whose files are gone.
    PruneMissing(Vec<PathBuf>),
    /// Delete a playlist and its own cover files (`cover_art`).
    DeletePlaylist { id: i64, cover_art: Option<String> },
    DeleteSmartPlaylist(i64),
    /// Delete the named albums from `library.json`.
    DeleteAlbums(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct ConfirmDialog {
    pub title: &'static str,
    /// "TARGET: " for one named item, "TARGETS: " for a count.
    pub target_label: &'static str,
    pub target: String,
    pub target_orange: bool,
    pub detail: Option<String>,
    pub note: &'static str,
    pub confirm: &'static str,
    pub action: ConfirmAction,
}

fn cancel_button<'a>(label: &'a str, msg: Message) -> Element<'a, Message> {
    button(text(label).size(10).font(bold()))
        .padding([8, 16])
        .style(|_t, s| {
            let hover = matches!(s, button::Status::Hovered);
            button::Style {
                background: hover.then_some(Background::Color(theme::LINE_SOFT)),
                text_color: if hover { Color::WHITE } else { Color::from_rgb8(0x6B, 0x69, 0x63) },
                border: Border {
                    color: if hover { Color::from_rgb8(0x46, 0x46, 0x4C) } else { Color::TRANSPARENT },
                    width: 1.0,
                    radius: 0.0.into(),
                },
                ..Default::default()
            }
        })
        .on_press(msg)
        .into()
}

pub fn ok_button<'a>(label: &'a str, msg: Message) -> Element<'a, Message> {
    button(text(label).size(10).font(bold()))
        .padding([8, 24])
        .style(|_t, s| button::Style {
            background: Some(Background::Color(if matches!(s, button::Status::Hovered) { Color::WHITE } else { theme::GOLD })),
            text_color: Color::BLACK,
            border: Border::default(),
            ..Default::default()
        })
        .on_press(msg)
        .into()
}

/// Chrome specification for [`modal_frame_sized`].
pub struct ModalChrome<'a> {
    pub title: &'a str,
    pub dot_color: Color,
    pub corner_color: Color,
    pub width: f32,
    pub header_right: Option<Element<'a, Message>>,
}

/// The shared modal: `chrome.width` px card centered over a dark backdrop. A click on the backdrop sends `dismiss`.
pub fn modal_frame_sized<'a>(
    chrome: ModalChrome<'a>,
    body: Element<'a, Message>,
    footer: Element<'a, Message>,
    dismiss: Option<Message>,
) -> Element<'a, Message> {
    let mut head_row = row![
        container(space::horizontal())
            .width(8)
            .height(8)
            .style(theme::fill_style(chrome.dot_color)),
        text(chrome.title).size(14).font(bold()).color(Color::WHITE),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    if let Some(right) = chrome.header_right {
        head_row = head_row.push(space::horizontal()).push(right);
    }
    let head = container(head_row)
    .padding([12, 24])
    .width(Length::Fill)
    .style(|_t| container::Style {
        background: Some(Background::Color(theme::LINE_SOFT)),
        border: Border { color: theme::LINE, width: 1.0, radius: 0.0.into() },
        ..Default::default()
    });
    let foot = container(footer).padding(16).width(Length::Fill).style(|_t| container::Style {
        background: Some(Background::Color(theme::PANEL)),
        border: Border { color: theme::LINE, width: 1.0, radius: 0.0.into() },
        ..Default::default()
    });
    let corner_color = chrome.corner_color;
    let corner = container(
        column![
            container(container(space::horizontal()).width(2).height(10).style(theme::fill_style(corner_color)))
                .width(12)
                .align_x(Alignment::End),
            container(space::horizontal()).width(12).height(2).style(theme::fill_style(corner_color)),
        ],
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(Alignment::End)
    .align_y(Alignment::End);
    let card = container(stack![column![head, container(body).padding(24).width(Length::Fill), foot], corner])
        .width(Length::Fixed(chrome.width))
        .style(|_t| container::Style {
            background: Some(Background::Color(theme::RAISED)),
            border: Border { color: Color::from_rgb8(0x46, 0x46, 0x4C), width: 1.0, radius: 0.0.into() },
            text_color: Some(theme::TEXT),
            ..Default::default()
        });
    // Border.backdrop[IsVisible=True]: the dim backdrop and the card fade in over 200 ms. The card's
    // fade is of its content over its own color (what lies under it varies).
    let card = crate::motion::enter(
        "modal-card",
        crate::motion::EnterSpec::new(200, crate::motion::Ease::Linear, 0.0, crate::motion::Fade::Cover(theme::RAISED)),
        card,
    );
    let backdrop = crate::motion::enter(
        "modal",
        crate::motion::EnterSpec::new(200, crate::motion::Ease::Linear, 0.0, crate::motion::Fade::Scrim(Color::from_rgba(0.0, 0.0, 0.0, 0.8))),
        container(opaque(card))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center),
    );
    match dismiss {
        Some(msg) => opaque(mouse_area(backdrop).on_press(msg)),
        None => opaque(backdrop),
    }
}

/// The shared modal: 448 px card centered over a dark backdrop.
pub fn modal_frame<'a>(
    title: &'a str,
    body: Element<'a, Message>,
    footer: Element<'a, Message>,
    dismiss: Option<Message>,
) -> Element<'a, Message> {
    modal_frame_sized(
        ModalChrome {
            title,
            dot_color: theme::ORANGE,
            corner_color: theme::ORANGE,
            width: 448.0,
            header_right: None,
        },
        body,
        footer,
        dismiss,
    )
}

/// WARNING: ... with the target, optional detail block, an optional note, CANCEL and the confirm button.
pub fn confirm_view(d: &ConfirmDialog) -> Element<'_, Message> {
    let soft = Color::from_rgb8(0xCF, 0xCC, 0xC4);
    let target_color = if d.target_orange { theme::ORANGE } else { Color::WHITE };
    let mut body = column![
        row![
            text(d.target_label).size(14).color(soft),
            text(d.target.as_str()).size(14).font(bold()).color(target_color),
        ]
        .wrap(),
    ]
    .spacing(4);
    if let Some(detail) = &d.detail {
        body = body.push(
            container(
                row![
                    container(space::horizontal())
                        .width(2)
                        .height(16)
                        .style(theme::fill_style(theme::ORANGE)),
                    text(detail.as_str()).size(14).color(Color::WHITE),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .padding([2, 0]),
        );
    }
    body = body.push(text(d.note).size(14).color(soft));
    let footer = row![
        space::horizontal(),
        cancel_button("CANCEL", Message::ConfirmCancel),
        ok_button(d.confirm, Message::ConfirmAccept),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .into();
    modal_frame(d.title, body.into(), footer, Some(Message::ConfirmCancel))
}

/// Blocking overlay shown while deleting albums from `library.json`.
pub fn deleting_overlay(turn: f32) -> Element<'static, Message> {
    use iced::widget::canvas::{self, Canvas, Path, Stroke};
    struct GoldRing(f32);
    impl<M> canvas::Program<M> for GoldRing {
        type State = ();
        fn draw(
            &self,
            _s: &(),
            renderer: &iced::Renderer,
            _t: &iced::Theme,
            bounds: iced::Rectangle,
            _c: iced::mouse::Cursor,
        ) -> Vec<canvas::Geometry> {
            let mut frame = canvas::Frame::new(renderer, bounds.size());
            let center = iced::Point::new(bounds.width / 2.0, bounds.height / 2.0);
            let r = bounds.width / 2.0 - 2.0;
            frame.stroke(
                &Path::circle(center, r),
                Stroke::default().with_color(theme::LINE).with_width(2.0),
            );
            let start = self.0 * std::f32::consts::TAU;
            let arc = Path::new(|p| {
                p.arc(canvas::path::Arc {
                    center,
                    radius: r,
                    start_angle: iced::Radians(start),
                    end_angle: iced::Radians(start + std::f32::consts::FRAC_PI_2),
                });
            });
            frame.stroke(&arc, Stroke::default().with_color(theme::GOLD).with_width(2.0));
            vec![frame.into_geometry()]
        }
    }
    let content = column![
        Canvas::new(GoldRing(turn)).width(48).height(48),
        text("DELETING_DATA...").size(12).color(theme::GOLD),
    ]
    .spacing(16)
    .align_x(Alignment::Center);
    opaque(
        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .style(|_t| container::Style {
                background: Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, 0.7))),
                ..Default::default()
            }),
    )
}

/// BACKGROUND_PLAY: asked on the first close while "minimize to tray" was never chosen.
pub fn background_prompt(remember: bool) -> Element<'static, Message> {
    let check = button(
        row![
            container(text(if remember { "✓" } else { "" }).size(11).font(bold()).color(Color::BLACK))
                .width(16)
                .height(16)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center)
                .style(move |_t| container::Style {
                    background: remember.then_some(Background::Color(theme::GOLD)),
                    border: Border {
                        color: if remember { theme::GOLD } else { Color::from_rgb8(0x6B, 0x69, 0x63) },
                        width: 1.0,
                        radius: 2.0.into(),
                    },
                    ..Default::default()
                }),
            text("REMEMBER MY CHOICE").size(11).color(if remember { theme::TEXT } else { theme::TEXT_DIM }),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .padding([4, 8])
    .style(theme::bare_button_style())
    .on_press(Message::BackgroundRemember(!remember));
    let body = column![
        text("Keep music playing in the background?").size(14).color(Color::from_rgb8(0xCF, 0xCC, 0xC4)),
        text(
            "If you choose MINIMIZE, the player will stay active in your system tray when closed. You can toggle this \
             setting anytime from the tray icon right-click menu.",
        )
        .size(12)
        .color(theme::TEXT_DIM),
        // A rule above the checkbox, as in the original
        column![
            container(space::horizontal()).width(Length::Fill).height(1).style(theme::fill_style(theme::LINE)),
            check,
        ]
        .spacing(12),
    ]
    .spacing(16)
    .into();
    let footer = row![
        space::horizontal(),
        cancel_button("QUIT", Message::BackgroundChoice(false)),
        ok_button("MINIMIZE", Message::BackgroundChoice(true)),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .into();
    modal_frame("BACKGROUND_PLAY", body, footer, None)
}
