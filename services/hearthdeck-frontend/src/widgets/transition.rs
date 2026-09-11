//! A horizontal push transition between two full-window pages.
//!
//! Both pages are laid out at the window bounds once, then drawn inside a
//! clipped layer and offset horizontally by an eased progress value. The
//! offset is applied at *draw* time, so the transition never rebuilds the
//! widget tree or re-runs layout: a frame is just another `draw`.
//!
//! The widget drives its own frames. While the transition is in flight it
//! asks for a redraw on every `RedrawRequested` event; once the duration has
//! elapsed it publishes `on_finished` so the caller can drop it. An idle
//! window therefore schedules no redraws and pays nothing for this widget.

use std::time::{Duration, Instant};

use cosmic::iced::core::Renderer as _;
use cosmic::iced::core::widget::{Operation, Tree};
use cosmic::iced::core::{
    Clipboard, Event, Length, Rectangle, Shell, Size, Vector, Widget, layout, mouse, overlay,
    renderer, window,
};
use cosmic::{Element, Renderer, Theme};

/// How long the push takes. Short enough to feel immediate on a TV, long
/// enough for the motion to read as a slide rather than a cut.
pub const DURATION: Duration = Duration::from_millis(220);

/// Slides `to` over `from`, then hands control back to the caller.
#[allow(missing_debug_implementations)]
pub struct PageTransition<'a, Message> {
    from: Element<'a, Message>,
    to: Element<'a, Message>,
    started_at: Instant,
    /// `+1.0` when the incoming page enters from the right (forward
    /// navigation), `-1.0` when it enters from the left (back navigation).
    direction: f32,
    on_finished: Message,
}

impl<'a, Message> PageTransition<'a, Message> {
    pub fn new(
        from: Element<'a, Message>,
        to: Element<'a, Message>,
        started_at: Instant,
        direction: f32,
        on_finished: Message,
    ) -> Self {
        Self {
            from,
            to,
            started_at,
            direction,
            on_finished,
        }
    }

    /// Eased progress in `0.0..=1.0` from the outgoing to the incoming page.
    fn progress(&self) -> f32 {
        let elapsed = Instant::now().saturating_duration_since(self.started_at);
        let raw = (elapsed.as_secs_f32() / DURATION.as_secs_f32()).clamp(0.0, 1.0);
        cosmic::anim::smootherstep(raw)
    }
}

impl<'a, Message> Widget<Message, Theme, Renderer> for PageTransition<'a, Message>
where
    Message: Clone + 'a,
{
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.from), Tree::new(&self.to)]
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(&mut [&mut self.from, &mut self.to]);
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let from = self
            .from
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        let to = self
            .to
            .as_widget_mut()
            .layout(&mut tree.children[1], renderer, limits);
        let size = Size::new(
            from.size().width.max(to.size().width),
            from.size().height.max(to.size().height),
        );
        layout::Node::with_children(size, vec![from, to])
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: layout::Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        // Only the incoming page takes input; the outgoing one is on its way
        // out and must not react to a stray event mid-slide.
        let to_layout = layout.children().nth(1).unwrap();
        self.to.as_widget_mut().update(
            &mut tree.children[1],
            event,
            to_layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );

        if let Event::Window(window::Event::RedrawRequested(now)) = event {
            if now.saturating_duration_since(self.started_at) < DURATION {
                shell.request_redraw();
            } else {
                shell.publish(self.on_finished.clone());
            }
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: layout::Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let travelled = bounds.width * self.progress();

        let mut children = layout.children();
        let from_layout = children.next().unwrap();
        let to_layout = children.next().unwrap();

        // Clip the pair to the window, then translate each page: the outgoing
        // one leaves in `-direction`, the incoming one arrives from
        // `+direction`. Both move in lockstep, so they stay edge-to-edge.
        renderer.with_layer(bounds, |renderer| {
            renderer.with_translation(Vector::new(-self.direction * travelled, 0.0), |renderer| {
                self.from.as_widget().draw(
                    &tree.children[0],
                    renderer,
                    theme,
                    style,
                    from_layout,
                    cursor,
                    viewport,
                );
            });

            renderer.with_translation(
                Vector::new(self.direction * (bounds.width - travelled), 0.0),
                |renderer| {
                    self.to.as_widget().draw(
                        &tree.children[1],
                        renderer,
                        theme,
                        style,
                        to_layout,
                        cursor,
                        viewport,
                    );
                },
            );
        });
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: layout::Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation<()>,
    ) {
        // Focus operations target the page that will survive the transition.
        let to_layout = layout.children().nth(1).unwrap();
        self.to
            .as_widget_mut()
            .operate(&mut tree.children[1], to_layout, renderer, operation);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: layout::Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let to_layout = layout.children().nth(1).unwrap();
        self.to.as_widget_mut().overlay(
            &mut tree.children[1],
            to_layout,
            renderer,
            viewport,
            translation,
        )
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: layout::Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let to_layout = layout.children().nth(1).unwrap();
        self.to.as_widget().mouse_interaction(
            &tree.children[1],
            to_layout,
            cursor,
            viewport,
            renderer,
        )
    }
}

impl<'a, Message> From<PageTransition<'a, Message>> for Element<'a, Message>
where
    Message: Clone + 'a,
{
    fn from(widget: PageTransition<'a, Message>) -> Self {
        Element::new(widget)
    }
}
