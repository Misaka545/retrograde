//! Hover, press and entrance transitions of the C# build (Avalonia `Transitions` and one-shot
//! `Animation`s), done inside widgets so a moving value only redraws the window: no `update`, no
//! `view` rebuild.
//!
//! * [`hover_scope`] watches the pointer over its content (a card, a button) and publishes
//!   "hovered" / "pressed" to the animated leaves drawn inside it, through a thread-local that is
//!   set only while the scope draws. Each leaf keeps its own tween (Avalonia semantics: a change
//!   animates from the current value to the new target over the full duration), keyed so a
//!   recycled slot of a virtualized grid never carries a half-finished zoom over to another album.
//! * [`enter`] replays an entrance (slide + fade) whenever its channel was absent from the window
//!   for a couple of frames, like the C# `[IsVisible=True]` animations.
//!
//! Frames are capped at ~60 Hz ([`FRAME`]) and stop as soon as nothing moves. With animations off
//! (`RenderTier::enable_animations`, battery saver), everything jumps to its end state.
//!
//! iced cannot fade a whole subtree, so a fade-in over a solid background draws a quad of that
//! background on top with falling alpha (exact over a solid color, see [`Fade::Cover`]).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::svg::{Renderer as _, Svg};
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Shell, Widget, overlay};
use iced::widget::svg;
use iced::window::RedrawRequest;
use iced::{
    Background, Color, ContentFit, Element, Event, Length, Radians, Rectangle, Size, Transformation, Vector,
    mouse, window,
};

/// Frame spacing of every transition here (~60 Hz, whatever the panel's refresh rate).
pub const FRAME: Duration = Duration::from_millis(16);

static ENABLED: AtomicBool = AtomicBool::new(true);
/// Windows presented so far (counted from the raw RedrawRequested listener in `main.rs`).
static FRAME_SEQ: AtomicU64 = AtomicU64::new(0);

/// Follows `RenderTier::enable_animations`: off, transitions jump to their end.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Called once per presented frame.
pub fn frame_presented() {
    FRAME_SEQ.fetch_add(1, Ordering::Relaxed);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ease {
    Linear,
    CubicOut,
    CubicInOut,
}

impl Ease {
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Ease::Linear => t,
            Ease::CubicOut => 1.0 - (1.0 - t).powi(3),
            Ease::CubicInOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
        }
    }
}

/// Progress (0..1, eased) of a one-shot animation started at `start`.
pub fn progress(start: Instant, now: Instant, dur: Duration, ease: Ease) -> f32 {
    if !enabled() || dur.is_zero() {
        return 1.0;
    }
    ease.apply(now.saturating_duration_since(start).as_secs_f32() / dur.as_secs_f32())
}

/// A stable key for a leaf's tween (e.g. the album a card shows).
pub fn key_of(v: &impl std::hash::Hash) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    Color { r: lerp(a.r, b.r, t), g: lerp(a.g, b.g, t), b: lerp(a.b, b.b, t), a: lerp(a.a, b.a, t) }
}

/// A transition toward a target that can change midway (Avalonia `Transition` semantics).
#[derive(Debug, Clone, Copy)]
struct Tween {
    key: u64,
    from: f32,
    to: f32,
    start: Instant,
}

impl Tween {
    fn at(&self, now: Instant, dur: Duration, ease: Ease) -> f32 {
        lerp(self.from, self.to, progress(self.start, now, dur, ease))
    }

    /// The value now, retargeting first. A new key (another album in a recycled slot) starts settled.
    fn step(cell: &Cell<Option<Tween>>, key: u64, target: f32, now: Instant, dur: Duration, ease: Ease) -> f32 {
        let t = match cell.get() {
            Some(t) if t.key == key && enabled() => {
                if t.to == target {
                    t
                } else {
                    Tween { key, from: t.at(now, dur, ease), to: target, start: now }
                }
            }
            _ => Tween { key, from: target, to: target, start: now },
        };
        cell.set(Some(t));
        t.at(now, dur, ease)
    }
}

// ---------------------------------------------------------------- hover scope

#[derive(Debug, Clone, Copy)]
struct ScopeCtx {
    hovered: bool,
    pressed: bool,
    now: Instant,
}

thread_local! {
    static SCOPES: RefCell<Vec<ScopeCtx>> = const { RefCell::new(Vec::new()) };
}

struct ScopeGuard;

impl ScopeGuard {
    fn push(ctx: ScopeCtx) -> Self {
        SCOPES.with(|s| s.borrow_mut().push(ctx));
        ScopeGuard
    }
}

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        SCOPES.with(|s| {
            s.borrow_mut().pop();
        });
    }
}

fn scope() -> Option<ScopeCtx> {
    SCOPES.with(|s| s.borrow().last().copied())
}

/// Makes `content` a hover target for the animated leaves inside it. `hold` is the longest leaf
/// transition: frames keep coming that long after the pointer enters, leaves or presses.
pub fn hover_scope<'a, Message: 'a>(hold: Duration, content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    // Leaves start their tween on the first frame drawn after the change: two spare frames let
    // them reach their end before the frames stop.
    Element::new(HoverScope { hold: hold + FRAME * 2, content: content.into() })
}

struct HoverScope<'a, Message> {
    hold: Duration,
    content: Element<'a, Message>,
}

#[derive(Debug, Default)]
struct ScopeState {
    hovered: bool,
    pressed: bool,
    changed: Option<Instant>,
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for HoverScope<'_, Message> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<ScopeState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(ScopeState::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &iced::Renderer, operation: &mut dyn Operation) {
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
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
        let pointer = dev::cursor().map_or(cursor, mouse::Cursor::Available);
        let state = tree.state.downcast_mut::<ScopeState>();
        let hovered = pointer.is_over(layout.bounds());
        let mut pressed = state.pressed;
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) if hovered => pressed = true,
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => pressed = false,
            _ => {}
        }
        let tracked = matches!(
            event,
            Event::Mouse(_) | Event::Window(window::Event::RedrawRequested(_))
        );
        if tracked && (hovered != state.hovered || pressed != state.pressed) {
            state.hovered = hovered;
            state.pressed = pressed;
            state.changed = Some(Instant::now());
            shell.request_redraw();
        }
        if let Event::Window(window::Event::RedrawRequested(now)) = event
            && enabled()
            && state.changed.is_some_and(|c| now.saturating_duration_since(c) < self.hold)
        {
            dev::log_frame(*now);
            shell.request_redraw_at(RedrawRequest::At(*now + FRAME));
        }
        self.content
            .as_widget_mut()
            .update(&mut tree.children[0], event, layout, cursor, renderer, clipboard, shell, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
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
        let state = tree.state.downcast_ref::<ScopeState>();
        let _guard = ScopeGuard::push(ScopeCtx { hovered: state.hovered, pressed: state.pressed, now: Instant::now() });
        self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation)
    }
}

/// What a leaf reacts to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Hover,
    Press,
}

/// How far (0..1) a leaf's transition is, given its spec; 0 outside any scope.
fn leaf_value(tree: &Tree, key: u64, trigger: Trigger, dur: Duration, ease: Ease) -> f32 {
    let cell = tree.state.downcast_ref::<Cell<Option<Tween>>>();
    let ctx = scope();
    let now = ctx.map_or_else(Instant::now, |c| c.now);
    let on = ctx.is_some_and(|c| match trigger {
        Trigger::Hover => c.hovered,
        Trigger::Press => c.pressed,
    });
    Tween::step(cell, key, if on { 1.0 } else { 0.0 }, now, dur, ease)
}

fn hovered_in_scope() -> bool {
    scope().is_some_and(|c| c.hovered)
}

/// A leaf wrapping `content`; `paint` draws it given the transition value.
struct Leaf<'a, Message> {
    key: u64,
    trigger: Trigger,
    dur: Duration,
    ease: Ease,
    content: Element<'a, Message>,
    paint: Paint,
}

/// The effect a [`Leaf`] applies to its content.
#[derive(Clone, Copy)]
enum Paint {
    /// Scaled around its center from 1 to `to`.
    Scale(f32),
    /// Text drawn in a color going from `from` to `to` (content must not set its own text color).
    TextColor(Color, Color),
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for Leaf<'_, Message> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Cell<Option<Tween>>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(Cell::<Option<Tween>>::new(None))
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &iced::Renderer, operation: &mut dyn Operation) {
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
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
        self.content
            .as_widget_mut()
            .update(&mut tree.children[0], event, layout, cursor, renderer, clipboard, shell, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
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
        let v = leaf_value(tree, self.key, self.trigger, self.dur, self.ease);
        let child = &tree.children[0];
        match self.paint {
            Paint::Scale(to) => {
                let s = lerp(1.0, to, v);
                if (s - 1.0).abs() < 1e-4 {
                    self.content.as_widget().draw(child, renderer, theme, style, layout, cursor, viewport);
                } else {
                    let c = layout.bounds().center();
                    let t = Transformation::translate(c.x, c.y) * Transformation::scale(s) * Transformation::translate(-c.x, -c.y);
                    renderer.with_transformation(t, |r| {
                        self.content.as_widget().draw(child, r, theme, style, layout, cursor, viewport);
                    });
                }
            }
            Paint::TextColor(from, to) => {
                let style = renderer::Style { text_color: lerp_color(from, to, v) };
                self.content.as_widget().draw(child, renderer, theme, &style, layout, cursor, viewport);
            }
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation)
    }
}

/// Scales `content` around its center to `to` while its scope is hovered or pressed.
pub fn scale_on<'a, Message: 'a>(
    trigger: Trigger,
    to: f32,
    dur_ms: u64,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    Element::new(Leaf {
        key: 0,
        trigger,
        dur: Duration::from_millis(dur_ms),
        ease: Ease::Linear,
        content: content.into(),
        paint: Paint::Scale(to),
    })
}

/// Fades the text color of `content` (text without its own color) from `from` to `to` on hover.
pub fn hover_text_color<'a, Message: 'a>(
    from: Color,
    to: Color,
    dur_ms: u64,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    Element::new(Leaf {
        key: 0,
        trigger: Trigger::Hover,
        dur: Duration::from_millis(dur_ms),
        ease: Ease::Linear,
        content: content.into(),
        paint: Paint::TextColor(from, to),
    })
}

// ---------------------------------------------------------------- drawn leaves

/// What a [`Drawn`] leaf paints.
enum Drawing<Message> {
    /// An album cover zooming to 1.05 on hover (500 ms cubic-out), clipped to its square.
    ZoomCover { handle: iced::widget::image::Handle },
    /// A Lucide-style icon whose tint fades `from` -> `to` and that moves by `shift` on hover.
    Icon { handle: svg::Handle, from: Color, to: Color, color_ms: u64, shift: Vector },
    /// A bar growing from the left over the full width on hover (the logo underline).
    Bar { color: Color },
    /// The card's play button: rises 32 px and fades in on hover, clipped `clip_below` px under it.
    CardPlay { normal: svg::Handle, hover: svg::Handle, on_press: Message, clip_below: f32 },
    /// The hexagonal transport button: scales to 1.05 on hover, white when hovered.
    Hex { normal: svg::Handle, hover: svg::Handle },
    /// Draws an image over the area with falling opacity: fades in whatever lies under it by
    /// covering it with the picture it sits on (the CHANGE_IMG overlay over a playlist cover).
    UnderImage { handle: iced::widget::image::Handle },
    /// The corner tick of a card: `rest` color, `hover` color when hovered (no transition in C#).
    Corner { rest: Color, hover: Color },
}

struct Drawn<Message> {
    key: u64,
    width: Length,
    height: Length,
    dur: Duration,
    ease: Ease,
    drawing: Drawing<Message>,
}

#[derive(Default)]
struct DrawnState {
    tween: Cell<Option<Tween>>,
    /// Secondary tween (icon color) next to the main one (icon shift).
    tween2: Cell<Option<Tween>>,
}

impl<Message: Clone> Widget<Message, iced::Theme, iced::Renderer> for Drawn<Message> {
    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.height)
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<DrawnState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(DrawnState::default())
    }

    fn layout(&mut self, _tree: &mut Tree, _renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        layout::atomic(limits, self.width, self.height)
    }

    fn update(
        &mut self,
        _tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        if let Drawing::CardPlay { on_press, .. } = &self.drawing
            && let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event
            && cursor.is_over(layout.bounds())
        {
            shell.publish(on_press.clone());
            shell.capture_event();
        }
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        if matches!(self.drawing, Drawing::CardPlay { .. }) && cursor.is_over(layout.bounds()) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::None
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &iced::Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<DrawnState>();
        let ctx = scope();
        let now = ctx.map_or_else(Instant::now, |c| c.now);
        let target = if ctx.is_some_and(|c| c.hovered) { 1.0 } else { 0.0 };
        let v = Tween::step(&state.tween, self.key, target, now, self.dur, self.ease);
        let bounds = layout.bounds();
        match &self.drawing {
            Drawing::ZoomCover { handle } => {
                iced::widget::image::draw(
                    renderer,
                    layout,
                    handle,
                    None,
                    iced::border::Radius::default(),
                    ContentFit::Cover,
                    iced::widget::image::FilterMethod::Linear,
                    iced::Rotation::default(),
                    1.0,
                    lerp(1.0, 1.05, v),
                );
            }
            Drawing::Icon { handle, from, to, color_ms, shift } => {
                let c = Tween::step(&state.tween2, self.key, target, now, Duration::from_millis(*color_ms), Ease::Linear);
                let at = Rectangle { x: bounds.x + shift.x * v, y: bounds.y + shift.y * v, ..bounds };
                renderer.draw_svg(
                    Svg { handle: handle.clone(), color: Some(lerp_color(*from, *to, c)), rotation: Radians(0.0), opacity: 1.0 },
                    at,
                    at,
                );
            }
            Drawing::Bar { color } => {
                if v > 0.001 {
                    renderer.fill_quad(
                        Quad { bounds: Rectangle { width: bounds.width * v, ..bounds }, ..Quad::default() },
                        Background::Color(*color),
                    );
                }
            }
            Drawing::CardPlay { normal, hover, clip_below, .. } => {
                if v > 0.001 {
                    let at = Rectangle { y: bounds.y + 32.0 * (1.0 - v), ..bounds };
                    // Clipped at the cover's bottom edge (the original's ClipToBounds), and to the
                    // list's viewport: a new layer does not inherit the enclosing clip.
                    let Some(clip) = Rectangle { height: bounds.height + clip_below, ..bounds }.intersection(viewport)
                    else {
                        return;
                    };
                    let own_hover = cursor.is_over(bounds);
                    renderer.with_layer(clip, |r| {
                        r.draw_svg(
                            Svg {
                                handle: if own_hover { hover.clone() } else { normal.clone() },
                                color: None,
                                rotation: Radians(0.0),
                                opacity: v,
                            },
                            at,
                            clip,
                        );
                    });
                }
            }
            Drawing::Hex { normal, hover } => {
                let s = lerp(1.0, 1.05, v);
                let at = Rectangle {
                    x: bounds.center_x() - bounds.width * s / 2.0,
                    y: bounds.center_y() - bounds.height * s / 2.0,
                    width: bounds.width * s,
                    height: bounds.height * s,
                };
                let handle = if hovered_in_scope() { hover } else { normal };
                renderer.draw_svg(
                    Svg { handle: handle.clone(), color: None, rotation: Radians(0.0), opacity: 1.0 },
                    at,
                    at,
                );
            }
            Drawing::UnderImage { handle } => {
                let cover = 1.0 - v;
                if cover > 0.001
                    && let Some(clip) = bounds.intersection(viewport)
                {
                    renderer.with_layer(clip, |r| {
                        iced::widget::image::draw(
                            r,
                            layout,
                            handle,
                            None,
                            iced::border::Radius::default(),
                            ContentFit::Cover,
                            iced::widget::image::FilterMethod::Linear,
                            iced::Rotation::default(),
                            cover,
                            1.0,
                        );
                    });
                }
            }
            Drawing::Corner { rest, hover } => {
                let color = if hovered_in_scope() { *hover } else { *rest };
                for b in [
                    Rectangle { width: 8.0, height: 1.0, ..bounds },
                    Rectangle { width: 1.0, height: 8.0, ..bounds },
                ] {
                    renderer.fill_quad(Quad { bounds: b, ..Quad::default() }, Background::Color(color));
                }
            }
        }
    }
}

fn drawn<'a, Message: Clone + 'a>(
    key: u64,
    width: Length,
    height: Length,
    dur_ms: u64,
    ease: Ease,
    drawing: Drawing<Message>,
) -> Element<'a, Message> {
    Element::new(Drawn { key, width, height, dur: Duration::from_millis(dur_ms), ease, drawing })
}

/// A card cover that zooms to 1.05 while its card is hovered (500 ms cubic-out).
pub fn zoom_cover<'a, Message: Clone + 'a>(key: u64, handle: iced::widget::image::Handle, side: f32) -> Element<'a, Message> {
    drawn(key, Length::Fixed(side), Length::Fixed(side), 500, Ease::CubicOut, Drawing::ZoomCover { handle })
}

/// An icon that changes tint over `color_ms` and moves by `shift` (200 ms cubic-out) on hover.
pub fn hover_icon<'a, Message: Clone + 'a>(
    handle: svg::Handle,
    size: f32,
    from: Color,
    to: Color,
    color_ms: u64,
    shift: Vector,
) -> Element<'a, Message> {
    drawn(0, Length::Fixed(size), Length::Fixed(size), 200, Ease::CubicOut, Drawing::Icon { handle, from, to, color_ms, shift })
}

/// A `height` px bar that grows from the left across its width on hover (500 ms cubic-in-out).
pub fn hover_bar<'a, Message: Clone + 'a>(color: Color, height: f32) -> Element<'a, Message> {
    drawn(0, Length::Fill, Length::Fixed(height), 500, Ease::CubicInOut, Drawing::Bar { color })
}

/// The 40 px play button of an album card (300 ms cubic-out rise and fade), clipped `clip_below`
/// px under its bottom edge (the cover's bottom).
pub fn card_play<'a, Message: Clone + 'a>(key: u64, on_press: Message, clip_below: f32) -> Element<'a, Message> {
    drawn(
        key,
        Length::Fixed(40.0),
        Length::Fixed(40.0),
        300,
        Ease::CubicOut,
        Drawing::CardPlay {
            normal: crate::icons::card_play_handle(false),
            hover: crate::icons::card_play_handle(true),
            on_press,
            clip_below,
        },
    )
}

/// The hexagonal play / pause button: 1.05 on hover (150 ms), white while hovered.
pub fn hex<'a, Message: Clone + 'a>(playing: bool, width: f32) -> Element<'a, Message> {
    drawn(
        0,
        Length::Fixed(width),
        Length::Fixed(width * 56.0 / 64.0),
        150,
        Ease::Linear,
        Drawing::Hex {
            normal: crate::icons::hex_play_handle(playing, false),
            hover: crate::icons::hex_play_handle(playing, true),
        },
    )
}

/// Laid over an overlay that should fade in on hover (200 ms): covers it with the image it sits on.
pub fn under_image<'a, Message: Clone + 'a>(handle: iced::widget::image::Handle, side: f32) -> Element<'a, Message> {
    drawn(0, Length::Fixed(side), Length::Fixed(side), 200, Ease::Linear, Drawing::UnderImage { handle })
}

/// The 8 px corner tick of a card, `hover` colored while the card is hovered.
pub fn corner_tick<'a, Message: Clone + 'a>(rest: Color, hover: Color) -> Element<'a, Message> {
    drawn(0, Length::Fixed(8.0), Length::Fixed(8.0), 0, Ease::Linear, Drawing::Corner { rest, hover })
}

// ---------------------------------------------------------------- entrances

/// How an entrance fades in.
#[derive(Debug, Clone, Copy)]
pub enum Fade {
    /// Covered by this (opaque) color with falling alpha: an exact fade over that solid background.
    Cover(Color),
    /// A dimming layer of this color drawn under the content, its alpha rising from 0 (a modal's backdrop).
    Scrim(Color),
}

impl EnterSpec {
    pub const fn new(dur_ms: u64, ease: Ease, dy: f32, fade: Fade) -> Self {
        Self { dur_ms, ease, dy, fade }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EnterSpec {
    pub dur_ms: u64,
    pub ease: Ease,
    /// Starts this many px lower and slides up.
    pub dy: f32,
    pub fade: Fade,
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    start: Instant,
    last_seen: u64,
    last_seen_at: Instant,
}

/// An entrance replays when its channel missed frames and was gone at least this long: the frame
/// counter comes from a subscription and can jump by a few at once when the executor lags.
const ABSENCE: Duration = Duration::from_millis(100);

thread_local! {
    static ENTRANCES: RefCell<HashMap<&'static str, Entry>> = RefCell::new(HashMap::new());
}

/// Plays `spec` each time `channel` comes on screen (after being absent for two frames or more).
pub fn enter<'a, Message: 'a>(
    channel: &'static str,
    spec: EnterSpec,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    Element::new(Enter { channel, spec, content: content.into() })
}

struct Enter<'a, Message> {
    channel: &'static str,
    spec: EnterSpec,
    content: Element<'a, Message>,
}

impl<Message> Enter<'_, Message> {
    fn value(&self, now: Instant) -> f32 {
        let start = ENTRANCES.with(|e| e.borrow().get(self.channel).map(|e| e.start));
        start.map_or(0.0, |s| progress(s, now, Duration::from_millis(self.spec.dur_ms), self.spec.ease))
    }
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for Enter<'_, Message> {
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

    fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &iced::Renderer, operation: &mut dyn Operation) {
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
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
        if let Event::Window(window::Event::RedrawRequested(now)) = event {
            let seq = FRAME_SEQ.load(Ordering::Relaxed);
            let start = ENTRANCES.with(|e| {
                let mut e = e.borrow_mut();
                let entry = e.entry(self.channel).or_insert(Entry { start: *now, last_seen: seq, last_seen_at: *now });
                if seq > entry.last_seen + 2 && now.saturating_duration_since(entry.last_seen_at) >= ABSENCE {
                    entry.start = *now;
                }
                entry.last_seen = seq;
                entry.last_seen_at = *now;
                entry.start
            });
            if enabled() && now.saturating_duration_since(start) < Duration::from_millis(self.spec.dur_ms) {
                dev::log_frame(*now);
                shell.request_redraw_at(RedrawRequest::At(*now + FRAME));
            }
        }
        self.content
            .as_widget_mut()
            .update(&mut tree.children[0], event, layout, cursor, renderer, clipboard, shell, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
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
        let v = self.value(Instant::now());
        let child = &tree.children[0];
        if let Fade::Scrim(color) = self.spec.fade {
            renderer.fill_quad(
                Quad { bounds: layout.bounds(), ..Quad::default() },
                Background::Color(Color { a: color.a * v, ..color }),
            );
        }
        if v >= 1.0 {
            self.content.as_widget().draw(child, renderer, theme, style, layout, cursor, viewport);
            return;
        }
        let dy = self.spec.dy * (1.0 - v);
        renderer.with_translation(Vector::new(0.0, dy), |r| {
            self.content.as_widget().draw(child, r, theme, style, layout, cursor, viewport);
        });
        // The cover slides with the content it covers; clipped to the viewport (a new layer does not
        // inherit the enclosing clip).
        if let Fade::Cover(color) = self.spec.fade {
            let moved = layout.bounds() + Vector::new(0.0, dy);
            if let Some(clip) = moved.intersection(viewport) {
                renderer.with_layer(clip, |r| {
                    r.fill_quad(Quad { bounds: moved, ..Quad::default() }, Background::Color(Color { a: 1.0 - v, ..color }));
                });
            }
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation)
    }
}

/// Developer hooks: a fake pointer for the RETRO_SHOT harness (hover without an input device), and
/// `RETRO_MOTION_LOG=<file>`, one line per animation frame requested (ms since the first).
pub mod dev {
    use std::io::Write;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::time::Instant;

    use iced::Point;

    static ACTIVE: AtomicBool = AtomicBool::new(false);
    static CURSOR: Mutex<Option<Point>> = Mutex::new(None);

    /// Sets (or clears) the fake pointer, in window coordinates.
    pub fn set_cursor(p: Option<Point>) {
        if let Ok(mut c) = CURSOR.lock() {
            *c = p;
            ACTIVE.store(p.is_some(), Ordering::Relaxed);
        }
    }

    pub(super) fn cursor() -> Option<Point> {
        if !ACTIVE.load(Ordering::Relaxed) {
            return None;
        }
        CURSOR.lock().ok().and_then(|c| *c)
    }

    pub(super) fn log_frame(now: Instant) {
        static FILE: OnceLock<Option<(Instant, Mutex<std::fs::File>)>> = OnceLock::new();
        let file = FILE.get_or_init(|| {
            let path = std::env::var_os("RETRO_MOTION_LOG")?;
            let f = std::fs::OpenOptions::new().create(true).append(true).open(path).ok()?;
            Some((Instant::now(), Mutex::new(f)))
        });
        if let Some((t0, f)) = file
            && let Ok(mut f) = f.lock()
        {
            let _ = writeln!(f, "{:.1}", now.saturating_duration_since(*t0).as_secs_f32() * 1000.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eases_hit_their_ends_and_midpoints() {
        for e in [Ease::Linear, Ease::CubicOut, Ease::CubicInOut] {
            assert_eq!(e.apply(0.0), 0.0);
            assert_eq!(e.apply(1.0), 1.0);
        }
        assert!((Ease::CubicOut.apply(0.5) - 0.875).abs() < 1e-6);
        assert!((Ease::CubicInOut.apply(0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn tween_retargets_from_current_value_and_resets_on_new_key() {
        set_enabled(true);
        let cell = Cell::new(None);
        let t0 = Instant::now();
        let dur = Duration::from_millis(100);
        assert_eq!(Tween::step(&cell, 1, 0.0, t0, dur, Ease::Linear), 0.0);
        assert_eq!(Tween::step(&cell, 1, 1.0, t0, dur, Ease::Linear), 0.0);
        let mid = Tween::step(&cell, 1, 1.0, t0 + Duration::from_millis(50), dur, Ease::Linear);
        assert!((mid - 0.5).abs() < 1e-3);
        // Leaving halfway animates back from 0.5 over the full duration.
        let back = Tween::step(&cell, 1, 0.0, t0 + Duration::from_millis(50), dur, Ease::Linear);
        assert!((back - 0.5).abs() < 1e-3);
        let later = Tween::step(&cell, 1, 0.0, t0 + Duration::from_millis(100), dur, Ease::Linear);
        assert!((later - 0.25).abs() < 1e-3);
        // Another album in the same slot starts settled.
        assert_eq!(Tween::step(&cell, 2, 1.0, t0 + Duration::from_millis(100), dur, Ease::Linear), 1.0);
    }
}
