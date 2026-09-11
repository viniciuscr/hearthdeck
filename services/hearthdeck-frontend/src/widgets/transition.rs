//! A fade-through transition between two full-window pages.
//!
//! See `docs/frontend-animations.md` for the animation model and the design
//! rationale. The widget is purely presentational: the caller supplies eased
//! `progress`. Over the first half the outgoing page fades out under a cover of
//! the surface colour; the incoming page is swapped in under full cover and
//! fades back out over the second half. The app owns the timing and the frames.

use cosmic::iced::core::Renderer as _;
use cosmic::iced::core::widget::{Operation, Tree};
use cosmic::iced::core::{
    Background, Clipboard, Color, Event, Length, Rectangle, Shell, Size, Vector, Widget, layout,
    mouse, overlay, renderer,
};
use cosmic::{Element, Renderer, Theme};

/// Cross-fades `to` in while `from` fades out, through the surface colour.
#[allow(missing_debug_implementations)]
pub struct PageTransition<'a, Message> {
    from: Element<'a, Message>,
    to: Element<'a, Message>,
    /// Eased progress in `0.0..=1.0` from the outgoing to the incoming page.
    progress: f32,
}

impl<'a, Message> PageTransition<'a, Message> {
    pub fn new(from: Element<'a, Message>, to: Element<'a, Message>, progress: f32) -> Self {
        Self { from, to, progress }
    }
}

/// Whether the outgoing page is still on the near side of the swap.
fn showing_from(progress: f32) -> bool {
    progress < 0.5
}

/// Cover opacity for an eased `progress` in `0.0..=1.0`: clear at both ends,
/// fully opaque at the midpoint where the pages swap.
fn cover_alpha(progress: f32) -> f32 {
    let phase = if showing_from(progress) {
        progress * 2.0
    } else {
        (1.0 - progress) * 2.0
    };

    cosmic::anim::smootherstep(phase.clamp(0.0, 1.0))
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
        // out and must not react to a stray event mid-transition.
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

        // Only the page on the near side of the swap is ever partly visible, so
        // one is drawn per frame rather than both.
        let mut children = layout.children();
        let from_layout = children.next().unwrap();
        let to_layout = children.next().unwrap();
        let (index, page, page_layout) = if showing_from(self.progress) {
            (0, &self.from, from_layout)
        } else {
            (1, &self.to, to_layout)
        };

        page.as_widget().draw(
            &tree.children[index],
            renderer,
            theme,
            style,
            page_layout,
            cursor,
            viewport,
        );

        // The cover is the page's own surface colour, so a full cover is
        // indistinguishable from the page background and the swap is hidden.
        surface_cover(renderer, theme, bounds, cover_alpha(self.progress));
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

/// A directional content change, used when switching tabs.
///
/// Tabs are peers with a spatial relationship, so this is a shared-axis slide
/// rather than a fade. The current content slides out in `direction`, the
/// content is swapped under a surface-colour cover, then the new content slides
/// back in. Only one child exists, so no two grids are ever laid out at once.
#[allow(missing_debug_implementations)]
pub struct TabTransition<'a, Message> {
    content: Element<'a, Message>,
    /// Eased progress in `0.0..=1.0` from the outgoing to the incoming tab.
    progress: f32,
    /// `+1.0` when the incoming tab sits to the right of the outgoing one.
    direction: f32,
}

impl<'a, Message> TabTransition<'a, Message> {
    pub fn new(content: Element<'a, Message>, progress: f32, direction: f32) -> Self {
        Self {
            content,
            progress,
            direction,
        }
    }
}

/// Horizontal offset and cover opacity for a tab transition at `progress`.
///
/// Returns `(offset_x, cover_alpha)`: the outgoing content reaches a full-width
/// offset exactly as the cover becomes opaque, and the incoming content starts
/// at the opposite full-width offset, so the swap always happens under cover.
fn slide(progress: f32, direction: f32, width: f32) -> (f32, f32) {
    if showing_from(progress) {
        let t = cosmic::anim::smootherstep((progress * 2.0).clamp(0.0, 1.0));
        (-direction * t * width, t)
    } else {
        let t = cosmic::anim::smootherstep(((progress - 0.5) * 2.0).clamp(0.0, 1.0));
        (direction * (1.0 - t) * width, 1.0 - t)
    }
}

impl<'a, Message> Widget<Message, Theme, Renderer> for TabTransition<'a, Message>
where
    Message: Clone + 'a,
{
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(&mut [&mut self.content]);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
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
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
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
        let (offset, cover) = slide(self.progress, self.direction, bounds.width);

        // Clip the sliding content to the window, then cover the swap.
        renderer.with_layer(bounds, |renderer| {
            renderer.with_translation(Vector::new(offset, 0.0), |renderer| {
                self.content.as_widget().draw(
                    &tree.children[0],
                    renderer,
                    theme,
                    style,
                    layout,
                    cursor,
                    viewport,
                );
            });
        });

        surface_cover(renderer, theme, bounds, cover);
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: layout::Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation<()>,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: layout::Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
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
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }
}

impl<'a, Message> From<TabTransition<'a, Message>> for Element<'a, Message>
where
    Message: Clone + 'a,
{
    fn from(widget: TabTransition<'a, Message>) -> Self {
        Element::new(widget)
    }
}

/// Fills `bounds` with the page's surface colour at `alpha`.
///
/// The cover must live in its own layer, allocated after the content's layers:
/// iced_wgpu renders layers in allocation order, so a quad added to the current
/// layer would paint *under* any clipped child such as a `scrollable`.
fn surface_cover(renderer: &mut Renderer, theme: &Theme, bounds: Rectangle, alpha: f32) {
    let mut surface = match crate::style::root_background(theme).background {
        Some(Background::Color(color)) => color,
        _ => Color::BLACK,
    };
    surface.a = alpha.clamp(0.0, 1.0);

    renderer.with_layer(bounds, |renderer| {
        renderer.fill_quad(
            renderer::Quad {
                bounds,
                ..renderer::Quad::default()
            },
            surface,
        );
    });
}

#[cfg(test)]
mod tests {
    use super::{cover_alpha, showing_from, slide};

    #[test]
    fn the_page_swaps_at_the_midpoint() {
        assert!(showing_from(0.0));
        assert!(showing_from(0.49));
        assert!(!showing_from(0.5));
        assert!(!showing_from(1.0));
    }

    #[test]
    fn the_cover_is_opaque_only_at_the_swap() {
        assert_eq!(cover_alpha(0.0), 0.0);
        assert_eq!(cover_alpha(0.5), 1.0);
        assert_eq!(cover_alpha(1.0), 0.0);

        assert!((0.0..1.0).contains(&cover_alpha(0.25)));
        assert!((0.0..1.0).contains(&cover_alpha(0.75)));
    }

    #[test]
    fn tab_content_is_off_stage_exactly_when_covered() {
        let width = 100.0;

        // At rest: centred, no cover.
        assert_eq!(slide(0.0, 1.0, width), (0.0, 0.0));
        assert_eq!(slide(1.0, 1.0, width), (0.0, 0.0));

        // At the swap the incoming content waits a full width to the right,
        // still fully covered, so the change is never visible.
        assert_eq!(slide(0.5, 1.0, width), (width, 1.0));

        // Just after the swap it is still covered and still near the far edge.
        let (dx, cover) = slide(0.5 + 0.01, 1.0, width);
        assert!(dx > 0.0 && dx < width);
        assert!(cover > 0.9);
    }

    #[test]
    fn tab_direction_flips_the_slide() {
        let width = 100.0;

        let (dx, _) = slide(0.25, 1.0, width);
        assert!(dx < 0.0, "forward should exit left");

        let (dx, _) = slide(0.25, -1.0, width);
        assert!(dx > 0.0, "backward should exit right");
    }
}
