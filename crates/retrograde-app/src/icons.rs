//! Lucide icons as SVG, drawn from the same path data as the original app (icon_paths.rs).

use std::cell::RefCell;
use std::collections::HashMap;

use iced::widget::svg;
use iced::{Color, Element, Length};

use crate::icon_paths::ICONS;

thread_local! {
    static CACHE: RefCell<HashMap<(String, bool), svg::Handle>> = RefCell::new(HashMap::new());
}

/// Icons the original draws as solid shapes rather than outlines.
fn is_filled(name: &str) -> bool {
    matches!(name, "play" | "pause" | "skipback" | "skipforward")
}

pub(crate) fn handle(name: &str) -> svg::Handle {
    let filled = is_filled(name);
    CACHE.with(|c| {
        c.borrow_mut()
            .entry((name.to_owned(), filled))
            .or_insert_with(|| {
                let d = ICONS.iter().find(|(n, _)| *n == name).map_or("", |(_, d)| *d);
                let fill = if filled { "white" } else { "none" };
                let doc = format!(
                    "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"{fill}\" stroke=\"white\" \
                     stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"{d}\"/></svg>"
                );
                svg::Handle::from_memory(doc.into_bytes())
            })
            .clone()
    })
}

/// A Lucide icon of the given name (lowercase, without the `Ic` prefix), `size` px square, tinted `color`.
pub fn icon<'a, M: 'a>(name: &str, size: f32, color: Color) -> Element<'a, M> {
    svg(handle(name))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(move |_t, _s| svg::Style { color: Some(color) })
        .into()
}

/// Like [`icon`], switching tint color when hovered.
pub fn icon_hover<'a, M: 'a>(name: &str, size: f32, color: Color, hover: Color) -> Element<'a, M> {
    svg(handle(name))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(move |_t, status| svg::Style {
            color: Some(if matches!(status, svg::Status::Hovered) { hover } else { color }),
        })
        .into()
}

/// The hexagon of the transport button (`ShapeHex` of the original) with a play or pause glyph:
/// off-white, or white while hovered.
pub(crate) fn hex_play_handle(playing: bool, hovered: bool) -> svg::Handle {
    let key = format!("hex{}{}", if playing { "pause" } else { "play" }, if hovered { "-hover" } else { "" });
    CACHE.with(|c| {
        c.borrow_mut()
            .entry((key, false))
            .or_insert_with(|| {
                let hex = "M16,0 L48,0 L64,28 L48,56 L16,56 L0,28 Z";
                let glyph = if playing {
                    "M24,17 H30 V39 H24 Z M34,17 H40 V39 H34 Z"
                } else {
                    "M26,16 L44,28 L26,40 Z"
                };
                let fill = if hovered { "#FFFFFF" } else { "#ECE9E1" };
                let doc = format!(
                    "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 64 56\"><path d=\"{hex}\" fill=\"{fill}\"/>\
                     <path d=\"{glyph}\" fill=\"#08080A\" stroke=\"#08080A\" stroke-width=\"2\" stroke-linejoin=\"round\"/></svg>"
                );
                svg::Handle::from_memory(doc.into_bytes())
            })
            .clone()
    })
}

/// The hexagonal play/pause button of the transport bar: an off-white hexagon with a dark glyph
/// that grows to 1.05 and turns white on hover (inside a [`crate::motion::hover_scope`]).
pub fn hex_play<'a, M: Clone + 'a>(playing: bool, width: f32) -> Element<'a, M> {
    crate::motion::hex(playing, width)
}

/// The 40 px play button of an album card (`ShapeCardPlay` of the original: a square with two cut
/// corners), off-white or white while hovered, with a black play glyph.
pub(crate) fn card_play_handle(hovered: bool) -> svg::Handle {
    let key = if hovered { "cardplay-hover" } else { "cardplay" }.to_owned();
    CACHE.with(|c| {
        c.borrow_mut()
            .entry((key, false))
            .or_insert_with(|| {
                let shape = "M8,0 L40,0 L40,32 L32,40 L0,40 L0,8 Z";
                let play = ICONS.iter().find(|(n, _)| *n == "play").map_or("", |(_, d)| *d);
                let fill = if hovered { "#FFFFFF" } else { "#ECE9E1" };
                let doc = format!(
                    "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 40 40\"><path d=\"{shape}\" fill=\"{fill}\"/>\
                     <g transform=\"translate(12,10) scale(0.8333)\"><path d=\"{play}\" fill=\"#000000\" stroke=\"#000000\" \
                     stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/></g></svg>"
                );
                svg::Handle::from_memory(doc.into_bytes())
            })
            .clone()
    })
}
