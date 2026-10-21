//! Dragging rows to another place in their list. A row says when the main button goes down on it,
//! before what is inside takes the press, so its button still plays it on a click; while a drag is
//! on, it says when the pointer comes onto it. The area around the list says when the pointer
//! nears its top or bottom edge, so the list scrolls on its own.

use iced::advanced::layout;
use iced::advanced::mouse;
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget};
use iced::{Element, Event, Length, Point, Rectangle, Size, Vector};

/// How near the top or bottom of the list the pointer starts the list scrolling.
const EDGE: f32 = 40.0;

/// Where the pointer is against the list's edges: up, down, or neither.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Edge {
    Up,
    #[default]
    None,
    Down,
}

impl Edge {
    fn of(position: Point, bounds: Rectangle) -> Self {
        if position.y < bounds.y + EDGE {
            Self::Up
        } else if position.y > bounds.y + bounds.height - EDGE {
            Self::Down
        } else {
            Self::None
        }
    }
}

type EdgeHandler<'a, Message> = Box<dyn Fn(Edge, f32) -> Message + 'a>;

/// A widget that reports presses and pointer movement over its content without taking them.
pub(super) struct DragZone<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    on_press: Option<Message>,
    on_enter: Option<Message>,
    on_edge: Option<EdgeHandler<'a, Message>>,
}

/// A row that can be dragged: `on_press` when the main button goes down on it, and `on_enter`,
/// set only during a drag, when the pointer comes onto it.
pub(super) fn drag_row<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    on_press: Option<Message>,
    on_enter: Option<Message>,
) -> DragZone<'a, Message, Theme, Renderer> {
    DragZone {
        content: content.into(),
        on_press,
        on_enter,
        on_edge: None,
    }
}

/// The area around a list with draggable rows. During a drag, `on_edge` is told the edge the
/// pointer is at, and the area's height, each time that edge changes.
pub(super) fn drag_area<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    on_edge: Option<impl Fn(Edge, f32) -> Message + 'a>,
) -> DragZone<'a, Message, Theme, Renderer> {
    DragZone {
        content: content.into(),
        on_press: None,
        on_enter: None,
        on_edge: on_edge.map(|handler| Box::new(handler) as EdgeHandler<'a, Message>),
    }
}

#[derive(Default)]
struct State {
    hovered: bool,
    edge: Edge,
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for DragZone<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
    Message: Clone,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
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

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
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
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let state: &mut State = tree.state.downcast_mut();
        let bounds = layout.bounds();
        match event {
            // Seen before the content: a button inside takes the press for itself.
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if cursor.is_over(bounds) =>
            {
                if let Some(message) = &self.on_press {
                    shell.publish(message.clone());
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let hovered = cursor.is_over(bounds);
                if hovered
                    && !state.hovered
                    && let Some(message) = &self.on_enter
                {
                    shell.publish(message.clone());
                }
                state.hovered = hovered;
                if let Some(on_edge) = &self.on_edge {
                    let edge = cursor
                        .position()
                        .map_or(Edge::None, |position| Edge::of(position, bounds));
                    if edge != state.edge {
                        state.edge = edge;
                        shell.publish(on_edge(edge, bounds.height));
                    }
                }
            }
            _ => {}
        }
        if self.on_edge.is_none() {
            state.edge = Edge::None;
        }
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

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
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

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
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

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
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
}

impl<'a, Message, Theme, Renderer> From<DragZone<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a + Clone,
    Theme: 'a,
    Renderer: 'a + renderer::Renderer,
{
    fn from(zone: DragZone<'a, Message, Theme, Renderer>) -> Self {
        Element::new(zone)
    }
}

/// The place of each shown row's item while one is dragged from `from` to `to`: the dragged item
/// shows at `to` and the ones between close up behind it.
pub(super) fn shown_item(shown: usize, from: usize, to: usize) -> usize {
    use std::cmp::Ordering;
    match from.cmp(&to) {
        Ordering::Less if (from..to).contains(&shown) => shown + 1,
        Ordering::Greater if (to + 1..=from).contains(&shown) => shown - 1,
        _ if shown == to => from,
        _ => shown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(count: usize, from: usize, to: usize) -> Vec<usize> {
        (0..count).map(|row| shown_item(row, from, to)).collect()
    }

    #[test]
    fn the_dragged_item_shows_where_it_would_land() {
        assert_eq!(shown(5, 1, 1), [0, 1, 2, 3, 4]);
        assert_eq!(shown(5, 1, 3), [0, 2, 3, 1, 4]);
        assert_eq!(shown(5, 3, 1), [0, 3, 1, 2, 4]);
        assert_eq!(shown(5, 0, 4), [1, 2, 3, 4, 0]);
        assert_eq!(shown(5, 4, 0), [4, 0, 1, 2, 3]);
    }

    #[test]
    fn the_edges_are_the_top_and_bottom_strips() {
        let bounds = Rectangle::new(Point::new(0.0, 100.0), Size::new(300.0, 400.0));
        assert_eq!(Edge::of(Point::new(10.0, 110.0), bounds), Edge::Up);
        assert_eq!(Edge::of(Point::new(10.0, 50.0), bounds), Edge::Up);
        assert_eq!(Edge::of(Point::new(10.0, 300.0), bounds), Edge::None);
        assert_eq!(Edge::of(Point::new(10.0, 480.0), bounds), Edge::Down);
        assert_eq!(Edge::of(Point::new(10.0, 900.0), bounds), Edge::Down);
    }
}
