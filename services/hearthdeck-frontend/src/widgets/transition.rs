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
        let (index, page, page_layout, phase) = if self.progress < 0.5 {
            (0, &self.from, from_layout, self.progress * 2.0)
        } else {
            (1, &self.to, to_layout, (1.0 - self.progress) * 2.0)
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
        let mut surface = match crate::style::root_background(theme).background {
            Some(Background::Color(color)) => color,
            _ => Color::BLACK,
        };
        surface.a = cosmic::anim::smootherstep(phase.clamp(0.0, 1.0));
        renderer.fill_quad(
            renderer::Quad {
                bounds,
                ..renderer::Quad::default()
            },
            surface,
        );
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
