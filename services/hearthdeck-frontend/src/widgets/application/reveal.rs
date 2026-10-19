//! A tile layer that shows itself only while it matters.
//!
//! The tile's name rides on the cover art as a scrim. It waits for the tile to
//! be picked out or pointed at rather than sitting on the artwork permanently.
//! That is the accent border's own rule - the ring appears when the tile is
//! selected and under the pointer - applied to the label, so the name and the
//! border always appear together.
//!
//! The gate lives in `draw` rather than in the view: the layer keeps its place
//! in the layout either way, so revealing it never moves the artwork or the
//! badges around it.

use cosmic::iced::core::widget::{Operation, Tree};
use cosmic::iced::core::{
    Clipboard, Event, Length, Rectangle, Shell, Size, Vector, Widget, layout, mouse, overlay,
    renderer,
};
use cosmic::{Element, Renderer, Theme};

/// Draws its content only while the tile is selected or the pointer is on it.
pub struct Reveal<'a, Message> {
    content: Element<'a, Message>,
    /// Whether the tile is picked out - the context menu's target or the card
    /// the cursor ring is on - so its content shows without the pointer.
    selected: bool,
}

impl<'a, Message> Reveal<'a, Message> {
    /// Gates `content` behind `selected` and the pointer.
    pub fn new(content: Element<'a, Message>, selected: bool) -> Self {
        Self { content, selected }
    }
}

/// Whether the layer draws at `pointer_over`: a selected tile always shows its
/// content, an unselected one only under the pointer - the same span the
/// border covers when it lights up on hover.
fn revealed(selected: bool, pointer_over: bool) -> bool {
    selected || pointer_over
}

impl<'a, Message> Widget<Message, Theme, Renderer> for Reveal<'a, Message>
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
        // The layer lays out whether or not it draws, so revealing it is a
        // paint-time change and the tile never shifts.
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
        if !revealed(self.selected, cursor.is_over(layout.bounds())) {
            return;
        }
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
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

impl<'a, Message> From<Reveal<'a, Message>> for Element<'a, Message>
where
    Message: Clone + 'a,
{
    fn from(widget: Reveal<'a, Message>) -> Self {
        Element::new(widget)
    }
}

#[cfg(test)]
mod tests {
    use super::revealed;

    #[test]
    fn an_unselected_tile_keeps_its_layer_hidden_without_the_pointer() {
        assert!(!revealed(false, false));
    }

    #[test]
    fn the_pointer_reveals_an_unselected_tiles_layer() {
        assert!(revealed(false, true));
    }

    #[test]
    fn a_selected_tiles_layer_needs_no_pointer() {
        assert!(revealed(true, false));
        assert!(revealed(true, true));
    }
}
