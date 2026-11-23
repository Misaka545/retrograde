//! Lenis-style smooth wheel scrolling, a port of the C# build's `Controls/SmoothWheel.cs`.
//!
//! [`smooth`] wraps a vertical `scrollable` (or anything whose first scrollable descendant is the
//! list to drive). Each wheel notch moves a target by [`STEP`] pixels; every frame the offset closes
//! a fixed share of the remaining distance (exponential smoothing, frame-rate independent).
//! Notches that arrive while it glides add to the same target, so a fast flick becomes one
//! continuous glide that slows down softly instead of a series of 60 px jumps.
//!
//! Like Chromium (and the C# build's `.scrolling` class), hover effects are held off while the
//! content moves under the pointer, and come back with a synthetic pointer move when it settles.
//! Touchpad (pixel) deltas, modified wheels (Shift / Ctrl) and wheels over a nested list that can
//! still scroll are passed through untouched.

use std::sync::atomic::Ordering;
use std::time::Instant;

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::operation::scrollable::{AbsoluteOffset, Scrollable};
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Shell, Widget, overlay};
use iced::{Element, Event, Length, Point, Rectangle, Size, Vector, keyboard, mouse, window};

/// Pixels per wheel notch (Chromium on Windows: ~100 CSS px), as in the C# build.
pub const STEP: f32 = 100.0;
/// Share of the remaining distance closed per second (Lenis' 0.1 per 60 Hz frame is ~6.3/s;
/// slightly firmer, so a long library never feels sluggish).
pub const LAMBDA: f32 = 8.5;
/// Close enough: snap and stop.
pub const SETTLE_PX: f32 = 0.5;
/// The offset was moved by something else (scrollbar drag, keyboard, a programmatic jump).
pub const TAKEOVER_PX: f32 = 2.0;
/// Longest frame step used for the smoothing, so a stalled frame does not teleport the list.
const MAX_DT: f32 = 0.1;

/// Wraps `content` (a vertical `scrollable`, possibly inside containers) with smooth wheel scrolling.
pub fn smooth<'a, Message: 'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    Element::new(SmoothWheel { content: content.into() })
}

struct SmoothWheel<'a, Message> {
    content: Element<'a, Message>,
}

#[derive(Debug, Default)]
struct Glide {
    running: bool,
    target: f32,
    /// Where the glide put the offset on the last frame.
    current: f32,
    last_frame: Option<Instant>,
    modifiers: keyboard::Modifiers,
    /// Start of the glide, for the RETRO_SCROLL_LOG frame log.
    started: Option<Instant>,
}

/// Developer hooks: synthetic wheel notches for the RETRO_SHOT harness (no input device needed),
/// and `RETRO_SCROLL_LOG=<file>`, one line per glide frame: `ms_since_start offset target`.
pub mod dev {
    use std::io::Write;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    use iced::{Point, Rectangle};

    pub(super) static PENDING: AtomicBool = AtomicBool::new(false);
    static WHEEL: Mutex<Option<(f32, Point, Instant)>> = Mutex::new(None);
    /// Notches no list claimed (nothing under the point) are dropped after this long.
    const EXPIRY: Duration = Duration::from_secs(1);

    /// Queues `notches` (positive = up) for the list under `at`, as if the wheel turned there.
    pub fn wheel(notches: f32, at: Point) {
        if let Ok(mut w) = WHEEL.lock() {
            let total = w.map_or(0.0, |(n, _, _)| n) + notches;
            *w = Some((total, at, Instant::now()));
            PENDING.store(true, Ordering::Relaxed);
        }
    }

    pub(super) fn take(bounds: Rectangle) -> Option<(f32, Point)> {
        let mut w = WHEEL.lock().ok()?;
        let (n, at, queued) = (*w)?;
        if queued.elapsed() > EXPIRY {
            *w = None;
            PENDING.store(false, Ordering::Relaxed);
            return None;
        }
        if !bounds.contains(at) {
            return None;
        }
        *w = None;
        PENDING.store(false, Ordering::Relaxed);
        Some((n, at))
    }

    pub(super) fn log(ms: f32, offset: f32, target: f32) {
        static FILE: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();
        let file = FILE.get_or_init(|| {
            let path = std::env::var_os("RETRO_SCROLL_LOG")?;
            std::fs::OpenOptions::new().create(true).append(true).open(path).ok().map(Mutex::new)
        });
        if let Some(f) = file
            && let Ok(mut f) = f.lock()
        {
            let _ = writeln!(f, "{ms:.1} {offset:.2} {target:.2}");
        }
    }
}

/// One step of the exponential smoothing: the new offset after `dt` seconds.
pub fn step(current: f32, target: f32, dt: f32) -> f32 {
    let remaining = target - current;
    if remaining.abs() <= SETTLE_PX {
        target
    } else {
        current + remaining * (1.0 - (-LAMBDA * dt.clamp(0.0, MAX_DT)).exp())
    }
}

/// The geometry of the first scrollable under the wrapper, plus whether the pointer is over a
/// nested scrollable that can still scroll (which then keeps the wheel, as in the C# build).
#[derive(Default)]
struct Probe {
    cursor: Option<Point>,
    found: Option<(f32, f32)>, // (offset y, max offset y)
    /// Translation of the outer list, to bring layout bounds of nested lists to screen space.
    outer_translation: Vector,
    nested_wants_wheel: bool,
    set_to: Option<f32>,
}

impl Operation for Probe {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        // Nested lists only matter for a wheel event; a frame only needs the outer list.
        if self.found.is_some() && (self.cursor.is_none() || self.nested_wants_wheel) {
            return;
        }
        operate(self);
    }

    fn scrollable(
        &mut self,
        _id: Option<&iced::advanced::widget::Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        translation: Vector,
        state: &mut dyn Scrollable,
    ) {
        if self.found.is_none() {
            let max = (content_bounds.height - bounds.height).max(0.0);
            self.found = Some((translation.y, max));
            self.outer_translation = translation;
            if let Some(y) = self.set_to {
                state.scroll_to(AbsoluteOffset { x: None, y: Some(y.clamp(0.0, max)) });
            }
            return;
        }
        if let Some(cursor) = self.cursor {
            let on_screen = Rectangle {
                x: bounds.x - self.outer_translation.x,
                y: bounds.y - self.outer_translation.y,
                ..bounds
            };
            if on_screen.contains(cursor) && content_bounds.height > bounds.height + 0.5 {
                self.nested_wants_wheel = true;
            }
        }
    }
}

impl<Message> SmoothWheel<'_, Message> {
    /// Moves the glide target by `notches` (positive = up, as wheel deltas). Returns `false` when the
    /// wheel belongs to someone else: no list, nothing to scroll, or a nested list under the pointer.
    fn wheel(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        notches: f32,
        cursor: Option<Point>,
    ) -> bool {
        let probe = self.probe(tree, layout, renderer, cursor, None);
        let Some((offset, max)) = probe.found else { return false };
        if max <= 0.0 || probe.nested_wants_wheel {
            return false;
        }
        let glide = tree.state.downcast_mut::<Glide>();
        let from = if glide.running { glide.target } else { offset };
        glide.target = (from - notches * STEP).clamp(0.0, max);
        if !glide.running {
            glide.running = true;
            glide.current = offset;
            glide.last_frame = None;
            glide.started = Some(Instant::now());
        }
        true
    }

    fn probe(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        cursor: Option<Point>,
        set_to: Option<f32>,
    ) -> Probe {
        let mut probe = Probe { cursor, set_to, ..Probe::default() };
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, &mut probe);
        probe
    }
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for SmoothWheel<'_, Message> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Glide>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(Glide::default())
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
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
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
        if let Event::Keyboard(keyboard::Event::ModifiersChanged(m)) = event {
            tree.state.downcast_mut::<Glide>().modifiers = *m;
        }
        // A press anywhere ends the glide, as in Lenis and Chromium: a scrollbar grab or a drag then
        // gets the real pointer, and a glide never carries over to a list swapped in by a click or key.
        if matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(_)) | Event::Keyboard(keyboard::Event::KeyPressed { .. })
        ) {
            let glide = tree.state.downcast_mut::<Glide>();
            glide.running = false;
            glide.last_frame = None;
        }

        // Wheel notches injected by the RETRO_SHOT harness take the same path as real ones.
        if dev::PENDING.load(Ordering::Relaxed)
            && let Some((y, at)) = dev::take(layout.bounds())
            && self.wheel(tree, layout, renderer, y, Some(at))
        {
            shell.request_redraw();
        }

        // A notch over the list: extend the glide instead of letting the list jump.
        if let Event::Mouse(mouse::Event::WheelScrolled { delta: mouse::ScrollDelta::Lines { y, .. } }) = event
            && *y != 0.0
            && cursor.is_over(layout.bounds())
            && tree.state.downcast_ref::<Glide>().modifiers.is_empty()
            && self.wheel(tree, layout, renderer, *y, cursor.position())
        {
            shell.capture_event();
            shell.request_redraw();
            return;
        }

        let running = tree.state.downcast_ref::<Glide>().running;
        if running && let Event::Window(window::Event::RedrawRequested(now)) = event {
            let probe = self.probe(tree, layout, renderer, None, None);
            let glide = tree.state.downcast_mut::<Glide>();
            let finished = match probe.found {
                // Something else moved the list (scrollbar drag, keyboard, a jump): give way to it.
                Some((offset, _)) if (offset - glide.current).abs() > TAKEOVER_PX => true,
                Some((_, max)) => {
                    // Lists that re-measure while scrolling can move their end.
                    glide.target = glide.target.clamp(0.0, max);
                    let dt = glide
                        .last_frame
                        .map_or(1.0 / 60.0, |last| now.saturating_duration_since(last).as_secs_f32());
                    glide.last_frame = Some(*now);
                    let y = step(glide.current, glide.target, dt);
                    glide.current = y;
                    if let Some(start) = glide.started {
                        dev::log(now.saturating_duration_since(start).as_secs_f32() * 1000.0, y, glide.target);
                    }
                    let done = y == glide.target;
                    self.probe(tree, layout, renderer, None, Some(y));
                    done
                }
                None => true,
            };
            let glide = tree.state.downcast_mut::<Glide>();
            if finished {
                glide.running = false;
                glide.last_frame = None;
            } else {
                shell.request_redraw();
            }
            // The list is redrawn under a hidden pointer while it glides; once it settles, a
            // pointer move brings hover back where the pointer actually is.
            let child_cursor = if finished { cursor } else { mouse::Cursor::Unavailable };
            self.content.as_widget_mut().update(
                &mut tree.children[0],
                event,
                layout,
                child_cursor,
                renderer,
                clipboard,
                shell,
                viewport,
            );
            if finished && let Some(position) = cursor.position() {
                self.content.as_widget_mut().update(
                    &mut tree.children[0],
                    &Event::Mouse(mouse::Event::CursorMoved { position }),
                    layout,
                    cursor,
                    renderer,
                    clipboard,
                    shell,
                    viewport,
                );
            }
            return;
        }

        let hide_hover = running && matches!(event, Event::Mouse(mouse::Event::CursorMoved { .. }));
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            if hide_hover { mouse::Cursor::Unavailable } else { cursor },
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content
            .as_widget()
            .mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
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
        let cursor = if tree.state.downcast_ref::<Glide>().running {
            mouse::Cursor::Unavailable
        } else {
            cursor
        };
        self.content
            .as_widget()
            .draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_matches_csharp_exponential_smoothing() {
        // One 60 Hz frame closes 1 - e^(-8.5/60) ≈ 13.2% of the way.
        let y = step(0.0, 100.0, 1.0 / 60.0);
        assert!((y - 13.21).abs() < 0.05, "{y}");
    }

    #[test]
    fn step_is_frame_rate_independent() {
        let mut a = 0.0;
        for _ in 0..60 {
            a = step(a, 1000.0, 1.0 / 60.0);
        }
        let mut b = 0.0;
        for _ in 0..144 {
            b = step(b, 1000.0, 1.0 / 144.0);
        }
        assert!((a - b).abs() < 0.5, "60 Hz {a} vs 144 Hz {b}");
    }

    #[test]
    fn step_snaps_when_close_and_caps_long_frames() {
        assert_eq!(step(99.6, 100.0, 1.0 / 60.0), 100.0);
        // A 2 s stall advances like a 100 ms frame, not straight to the end.
        let y = step(0.0, 1000.0, 2.0);
        assert!((y - 572.6).abs() < 1.0, "{y}");
    }
}
