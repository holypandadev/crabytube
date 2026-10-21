//! Dragging rows to another place in their list. A row says when the main button goes down on it,
//! before what is inside takes the press, so its button still plays it on a click. While a row is
//! held, the area around the list says where the pointer is, even past its edges, and shows a
//! closed hand. On a touch screen a finger held still on a row for a moment lifts it, and a finger
//! that moves sooner scrolls the list as before.

use iced::advanced::layout;
use iced::advanced::mouse;
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget};
use iced::time::{Duration, Instant};
use iced::{Element, Event, Length, Point, Rectangle, Size, Vector, touch, window};

type MoveHandler<'a, Message> = Box<dyn Fn(Point, f32) -> Message + 'a>;

/// A widget that reports presses and pointer movement over its content without taking them.
pub(super) struct DragZone<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    on_press: Option<Message>,
    on_hold: Option<Message>,
    on_move: Option<MoveHandler<'a, Message>>,
}

/// A row that can be dragged: `on_press` when the main button goes down on it, and `on_hold` when
/// a finger has been held still on it for `HOLD`.
pub(super) fn drag_row<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    on_press: Option<Message>,
    on_hold: Option<Message>,
) -> DragZone<'a, Message, Theme, Renderer> {
    DragZone {
        content: content.into(),
        on_press,
        on_hold,
        on_move: None,
    }
}

/// The area around a list with draggable rows. While `on_move` is set, a row is held: each move of
/// the pointer is told with its place over the area and the area's height, and the pointer is a
/// closed hand.
pub(super) fn drag_area<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    on_move: Option<impl Fn(Point, f32) -> Message + 'a>,
) -> DragZone<'a, Message, Theme, Renderer> {
    DragZone {
        content: content.into(),
        on_press: None,
        on_hold: None,
        on_move: on_move.map(|handler| Box::new(handler) as MoveHandler<'a, Message>),
    }
}

/// How long a finger stays still on a row before the row is lifted, and how far it may move
/// meanwhile.
pub(super) const HOLD: Duration = Duration::from_millis(450);
pub(super) const HOLD_SLOP: f32 = 10.0;

/// A finger on a row, until it is held long enough to lift the row, moves away or is lifted.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct LongPress {
    press: Option<(touch::Finger, Point, Instant)>,
}

impl LongPress {
    /// A finger went down at `position`, at `now`. Returns when the row would be lifted.
    pub(super) fn press(
        &mut self,
        finger: touch::Finger,
        position: Point,
        now: Instant,
    ) -> Instant {
        self.press = Some((finger, position, now));
        now + HOLD
    }

    /// The finger moved: past the slop it scrolls, and the row is not lifted.
    pub(super) fn moved(&mut self, finger: touch::Finger, position: Point) {
        if let Some((held, start, _)) = self.press
            && held == finger
            && start.distance(position) > HOLD_SLOP
        {
            self.press = None;
        }
    }

    /// The finger was lifted or lost.
    pub(super) fn released(&mut self, finger: touch::Finger) {
        if self.press.is_some_and(|(held, _, _)| held == finger) {
            self.press = None;
        }
    }

    /// Whether the finger has been held still long enough at `now`; it lifts the row once.
    pub(super) fn due(&mut self, now: Instant) -> bool {
        match self.press {
            Some((_, _, at)) if now >= at + HOLD => {
                self.press = None;
                true
            }
            _ => false,
        }
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for DragZone<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
    Message: Clone,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<LongPress>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(LongPress::default())
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
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if let Some(on_move) = &self.on_move {
                    let place = Point::new(position.x - bounds.x, position.y - bounds.y);
                    shell.publish(on_move(place, bounds.height));
                }
            }
            Event::Touch(touch::Event::FingerPressed { id, position })
                if self.on_hold.is_some() && bounds.contains(*position) =>
            {
                let due =
                    tree.state
                        .downcast_mut::<LongPress>()
                        .press(*id, *position, Instant::now());
                shell.request_redraw_at(due);
            }
            Event::Touch(touch::Event::FingerMoved { id, position }) => {
                tree.state.downcast_mut::<LongPress>().moved(*id, *position);
                // While a row is lifted the finger moves it, and the list under it does not
                // scroll.
                if let Some(on_move) = &self.on_move {
                    let place = Point::new(position.x - bounds.x, position.y - bounds.y);
                    shell.publish(on_move(place, bounds.height));
                    shell.capture_event();
                    return;
                }
            }
            Event::Touch(
                touch::Event::FingerLifted { id, .. } | touch::Event::FingerLost { id, .. },
            ) => tree.state.downcast_mut::<LongPress>().released(*id),
            Event::Window(window::Event::RedrawRequested(now)) => {
                if tree.state.downcast_mut::<LongPress>().due(*now)
                    && let Some(message) = &self.on_hold
                {
                    shell.publish(message.clone());
                }
            }
            _ => {}
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
        if self.on_move.is_some() {
            return mouse::Interaction::Grabbing;
        }
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

/// How far a held row's pointer must move before the row is dragged, so a click that shakes a
/// little stays a click.
pub(super) const DRAG_START: f32 = 6.0;

/// How near the top or bottom of the list the pointer starts the list scrolling.
pub(super) const EDGE: f32 = 48.0;

/// Where the pointer is against the list's edges: up, down, or neither, with how deep into the
/// edge it is, from 0 to 1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) enum Edge {
    Up(f32),
    #[default]
    None,
    Down(f32),
}

impl Edge {
    /// The edge of a list `height` tall that `y`, over it, is at.
    pub(super) fn of(y: f32, height: f32) -> Self {
        if y < EDGE {
            Self::Up(((EDGE - y) / EDGE).clamp(0.0, 1.0))
        } else if y > height - EDGE {
            Self::Down(((y - (height - EDGE)) / EDGE).clamp(0.0, 1.0))
        } else {
            Self::None
        }
    }

    /// How far the list scrolls on one tick: faster the deeper the pointer is in the edge.
    pub(super) fn step(self) -> f32 {
        let speed = |depth: f32| 4.0 + 22.0 * depth;
        match self {
            Self::Up(depth) => -speed(depth),
            Self::Down(depth) => speed(depth),
            Self::None => 0.0,
        }
    }
}

/// The place a row taken from `from` lands at for an insertion line before row `slot` of `count`
/// (`slot == count` is after the last).
pub(super) fn landing(from: usize, slot: usize, count: usize) -> usize {
    let slot = slot.min(count);
    if slot > from { slot - 1 } else { slot }
}

/// The insertion line nearest to `y`, a place in the list's content, for rows `pitch` apart that
/// start after `lead` rows that cannot move.
pub(super) fn slot_at(y: f32, pitch: f32, lead: usize, count: usize) -> usize {
    let row = (y / pitch).round().max(0.0) as usize;
    row.saturating_sub(lead).min(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_lands_on_the_line_nearest_the_pointer() {
        // Rows 50 apart: the line before row 2 is at 100.
        assert_eq!(slot_at(90.0, 50.0, 0, 5), 2);
        assert_eq!(slot_at(-30.0, 50.0, 0, 5), 0);
        assert_eq!(slot_at(900.0, 50.0, 0, 5), 5);
        // A first row that stays: its line is not offered.
        assert_eq!(slot_at(40.0, 50.0, 1, 5), 0);
        assert_eq!(slot_at(160.0, 50.0, 1, 5), 2);
        // Taken from 1: the lines just before and after it leave it where it is.
        assert_eq!(landing(1, 1, 5), 1);
        assert_eq!(landing(1, 2, 5), 1);
        assert_eq!(landing(1, 4, 5), 3);
        assert_eq!(landing(1, 5, 5), 4);
        assert_eq!(landing(3, 0, 5), 0);
    }

    #[test]
    fn a_finger_held_still_lifts_the_row_once_and_one_that_moves_scrolls() {
        let finger = touch::Finger(1);
        let start = Instant::now();
        let mut hold = LongPress::default();
        let due = hold.press(finger, Point::new(10.0, 10.0), start);
        assert_eq!(due, start + HOLD);
        assert!(!hold.due(start + HOLD / 2));
        // A little shake is still holding.
        hold.moved(finger, Point::new(14.0, 13.0));
        assert!(hold.due(due));
        assert!(!hold.due(due + HOLD), "a row is lifted once per press");
        // A finger that moves away before the time scrolls instead.
        hold.press(finger, Point::new(10.0, 10.0), start);
        hold.moved(finger, Point::new(10.0, 40.0));
        assert!(!hold.due(due));
        // Another finger, or one lifted, does not lift the row either.
        hold.press(finger, Point::new(10.0, 10.0), start);
        hold.moved(touch::Finger(2), Point::new(10.0, 90.0));
        hold.released(finger);
        assert!(!hold.due(due));
    }

    #[test]
    fn the_edges_scroll_faster_the_deeper_the_pointer_goes() {
        assert_eq!(Edge::of(200.0, 400.0), Edge::None);
        assert!(matches!(Edge::of(10.0, 400.0), Edge::Up(_)));
        assert!(matches!(Edge::of(-50.0, 400.0), Edge::Up(depth) if depth == 1.0));
        assert!(matches!(Edge::of(390.0, 400.0), Edge::Down(_)));
        assert!(Edge::of(399.0, 400.0).step() > Edge::of(360.0, 400.0).step());
        assert!(Edge::of(0.0, 400.0).step() < 0.0);
        assert_eq!(Edge::None.step(), 0.0);
    }
}
