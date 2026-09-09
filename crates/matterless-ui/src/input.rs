//! What the reader is doing, and to which box.
//!
//! The window hands over raw events -- the pointer moved, a button went down, a
//! wheel turned, a key was struck. This turns them into the questions a widget
//! actually asks: what is hovered, what was clicked, what has focus, how far
//! did the wheel turn over *this* panel.
//!
//! The distinction that matters is between a press and a click. A press is
//! where the button went down; a click is a press and a release on the *same*
//! box. Dragging off a button and letting go must not press it, and getting
//! that wrong is invisible until somebody complains that the app fires things
//! they meant to cancel.

use crate::Placed;
use crate::hit;

/// A raw event, as a window reports it.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    PointerMoved {
        x: f32,
        y: f32,
    },
    PointerPressed,
    PointerReleased,
    /// Positive `y` scrolls towards older content, matching a wheel's own sign.
    Wheel {
        x: f32,
        y: f32,
    },
    /// A key went down or came up. Repeats arrive as further presses.
    Key {
        key: Key,
        down: bool,
    },
    /// Text the platform composed, which is not the same as the keys struck:
    /// an accented letter or a CJK character is several keys and one insertion.
    Typed(String),
    /// The pointer left the window, so nothing is hovered.
    PointerLeft,
}

/// The keys the app acts on. Deliberately not every key: text arrives as
/// `Typed`, and a key here is one that *does* something.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Enter,
    Escape,
    Tab,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

/// What the reader has done since the last frame, and what is still true.
#[derive(Debug, Default, Clone)]
pub struct Input {
    pointer: Option<(f32, f32)>,
    hovered: Option<String>,
    /// Where a press began, held until the button is released.
    pressed_on: Option<String>,
    /// Set for one frame when a press and its release agreed.
    clicked: Option<String>,
    wheel: (f32, f32),
    focus: Option<String>,
    typed: String,
    keys: Vec<Key>,
}

impl Input {
    /// Folds one event in, against the boxes as they currently stand.
    pub fn apply(&mut self, event: Event, placed: &[Placed]) {
        match event {
            Event::PointerMoved { x, y } => {
                self.pointer = Some((x, y));
                self.hovered = hit::at(placed, x, y).map(|found| found.name.clone());
            }
            Event::PointerLeft => {
                self.pointer = None;
                self.hovered = None;
            }
            Event::PointerPressed => {
                self.pressed_on = self.hovered.clone();
                // Focus follows the press, not the release: a reader who holds
                // the button down on a field expects it to be theirs already.
                self.focus = self.hovered.clone();
            }
            Event::PointerReleased => {
                // Only when the release agrees with the press. Dragging off and
                // letting go is a cancellation, and the reader means it.
                if let (Some(from), Some(over)) = (&self.pressed_on, &self.hovered)
                    && from == over
                {
                    self.clicked = Some(from.clone());
                }
                self.pressed_on = None;
            }
            Event::Wheel { x, y } => {
                self.wheel.0 += x;
                self.wheel.1 += y;
            }
            Event::Key { key, down } => {
                if down {
                    self.keys.push(key);
                }
            }
            Event::Typed(text) => self.typed.push_str(&text),
        }
    }

    /// The box under the pointer.
    pub fn hovered(&self) -> Option<&str> {
        self.hovered.as_deref()
    }

    /// The box a press is currently held on, for drawing it pressed.
    pub fn pressed(&self) -> Option<&str> {
        self.pressed_on.as_deref()
    }

    pub fn clicked(&self) -> Option<&str> {
        self.clicked.as_deref()
    }

    /// True when this box was clicked, which is the question a button asks.
    pub fn clicked_on(&self, name: &str) -> bool {
        self.clicked.as_deref() == Some(name)
    }

    pub fn focus(&self) -> Option<&str> {
        self.focus.as_deref()
    }

    /// Focus without a click, for a field that opens already focused.
    pub fn focus_on(&mut self, name: impl Into<String>) {
        self.focus = Some(name.into());
    }

    /// How far the wheel turned, and over what.
    ///
    /// The panel is the caller's to decide: a pointer is over a row, inside a
    /// list, inside the window, and only the caller knows which of those
    /// scrolls.
    pub fn wheel_over(
        &self,
        placed: &[Placed],
        wanted: impl Fn(&str) -> bool,
    ) -> Option<(f32, f32)> {
        if self.wheel == (0.0, 0.0) {
            return None;
        }
        let (x, y) = self.pointer?;
        hit::at_matching(placed, x, y, wanted).map(|_| self.wheel)
    }

    pub fn keys(&self) -> &[Key] {
        &self.keys
    }

    pub fn struck(&self, key: Key) -> bool {
        self.keys.contains(&key)
    }

    /// Text composed since the last frame.
    pub fn typed(&self) -> &str {
        &self.typed
    }

    /// Clears what only lasted a frame, keeping what is still true.
    ///
    /// Hover, focus and a held press survive; a click, a wheel turn, keys and
    /// typed text do not. Forgetting this is how one click becomes many.
    pub fn settle(&mut self) {
        self.clicked = None;
        self.wheel = (0.0, 0.0);
        self.typed.clear();
        self.keys.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solve::solve;
    use crate::{Axis, Node, Rect, Size};

    fn shell() -> Vec<Placed> {
        let tree = Node::new("shell", Size::Grow(1.0))
            .axis(Axis::Row)
            .with(
                Node::new("sidebar", Size::Fixed(200.0))
                    .with(Node::new("sidebar/one", Size::Fixed(40.0)))
                    .with(Node::new("sidebar/two", Size::Fixed(40.0))),
            )
            .with(Node::new("stream", Size::Grow(1.0)));
        solve(&tree, Rect::new(0.0, 0.0, 800.0, 600.0))
    }

    fn at(input: &mut Input, placed: &[Placed], x: f32, y: f32) {
        input.apply(Event::PointerMoved { x, y }, placed);
    }

    #[test]
    fn a_press_and_release_on_one_box_is_a_click() {
        let placed = shell();
        let mut input = Input::default();
        at(&mut input, &placed, 100.0, 20.0);
        input.apply(Event::PointerPressed, &placed);
        input.apply(Event::PointerReleased, &placed);
        assert!(input.clicked_on("sidebar/one"));
    }

    /// The behaviour a reader relies on to change their mind.
    #[test]
    fn dragging_off_before_letting_go_is_not_a_click() {
        let placed = shell();
        let mut input = Input::default();
        at(&mut input, &placed, 100.0, 20.0);
        input.apply(Event::PointerPressed, &placed);
        at(&mut input, &placed, 100.0, 60.0);
        input.apply(Event::PointerReleased, &placed);
        assert_eq!(input.clicked(), None);
        assert!(!input.clicked_on("sidebar/one"));
        assert!(!input.clicked_on("sidebar/two"));
    }

    #[test]
    fn a_held_press_is_visible_while_it_is_held() {
        let placed = shell();
        let mut input = Input::default();
        at(&mut input, &placed, 100.0, 20.0);
        input.apply(Event::PointerPressed, &placed);
        assert_eq!(input.pressed(), Some("sidebar/one"));
        input.apply(Event::PointerReleased, &placed);
        assert_eq!(input.pressed(), None);
    }

    #[test]
    fn focus_follows_the_press() {
        let placed = shell();
        let mut input = Input::default();
        at(&mut input, &placed, 400.0, 300.0);
        input.apply(Event::PointerPressed, &placed);
        assert_eq!(input.focus(), Some("stream"));
    }

    /// A click lasts one frame; hover and focus outlive it.
    #[test]
    fn settling_forgets_only_what_was_momentary() {
        let placed = shell();
        let mut input = Input::default();
        at(&mut input, &placed, 100.0, 20.0);
        input.apply(Event::PointerPressed, &placed);
        input.apply(Event::PointerReleased, &placed);
        input.apply(Event::Wheel { x: 0.0, y: 40.0 }, &placed);
        input.apply(Event::Typed("hi".into()), &placed);
        input.apply(
            Event::Key {
                key: Key::Enter,
                down: true,
            },
            &placed,
        );

        assert!(input.clicked_on("sidebar/one"));
        assert_eq!(input.typed(), "hi");
        assert!(input.struck(Key::Enter));

        input.settle();
        assert_eq!(input.clicked(), None);
        assert_eq!(input.typed(), "");
        assert!(!input.struck(Key::Enter));
        // Still true, because the pointer has not moved and nothing was clicked
        // elsewhere.
        assert_eq!(input.hovered(), Some("sidebar/one"));
        assert_eq!(input.focus(), Some("sidebar/one"));
    }

    #[test]
    fn the_wheel_belongs_to_the_panel_the_caller_names() {
        let placed = shell();
        let mut input = Input::default();
        at(&mut input, &placed, 100.0, 20.0);
        input.apply(Event::Wheel { x: 0.0, y: 40.0 }, &placed);
        // Over a row, but the sidebar is what scrolls.
        let turned = input.wheel_over(&placed, |name| name == "sidebar");
        assert_eq!(turned, Some((0.0, 40.0)));
        // Nothing turned over the stream, because the pointer is not there.
        assert_eq!(input.wheel_over(&placed, |name| name == "stream"), None);
    }

    #[test]
    fn a_pointer_that_left_hovers_nothing() {
        let placed = shell();
        let mut input = Input::default();
        at(&mut input, &placed, 100.0, 20.0);
        input.apply(Event::PointerLeft, &placed);
        assert_eq!(input.hovered(), None);
        assert_eq!(input.wheel_over(&placed, |_| true), None);
    }
}
