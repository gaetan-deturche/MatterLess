//! The things a reader can do to one message.
//!
//! A small row of buttons that appears on the message under the pointer. It is
//! the first widget here that is *inside* a row rather than beside one -- the
//! reaction pills proved the hit-testing, and this is what that was for.
//!
//! Hover rather than a menu, because a menu is two clicks for something that
//! wants to be one, and because a toolbar can say what it offers without being
//! opened.

use matterless_ui::Rect;

/// What one button does.
///
/// Deliberately not every action the app has: an edit needs an editor in the
/// row and a forward needs a channel picker, and half a feature drawn as a
/// button that does nothing is worse than a button that is not there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Open the picker, to add a reaction that is not already on the message.
    React,
    /// Open this message's thread.
    Thread,
    /// Keep it, or stop keeping it.
    Save,
    /// Pin it to the channel, for everyone.
    Pin,
    /// Copy a permalink.
    Link,
    /// Change what it says. Only ever offered on the reader's own.
    Edit,
    /// Delete it. Only ever offered on the reader's own.
    Delete,
}

impl Action {
    /// The short name it answers to, which is also what a hit test carries.
    pub fn slug(self) -> &'static str {
        match self {
            Action::React => "react",
            Action::Thread => "thread",
            Action::Save => "save",
            Action::Pin => "pin",
            Action::Link => "link",
            Action::Edit => "edit",
            Action::Delete => "delete",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        Some(match slug {
            "react" => Action::React,
            "thread" => Action::Thread,
            "save" => Action::Save,
            "pin" => Action::Pin,
            "link" => Action::Link,
            "edit" => Action::Edit,
            "delete" => Action::Delete,
            _ => return None,
        })
    }

    /// What the button says. Words rather than icons: an icon needs a font this
    /// window does not ship and a legend nobody reads, and five short words fit
    /// in the room a message leaves at its top right.
    pub fn label(self, on: bool) -> &'static str {
        match (self, on) {
            (Action::React, _) => "react",
            (Action::Thread, _) => "reply",
            (Action::Save, false) => "save",
            (Action::Save, true) => "saved",
            (Action::Pin, false) => "pin",
            (Action::Pin, true) => "pinned",
            (Action::Link, _) => "link",
            (Action::Edit, _) => "edit",
            (Action::Delete, _) => "delete",
        }
    }
}

/// The height of the strip, and the room one button takes.
pub const HEIGHT: f32 = 22.0;
const GAP: f32 = 6.0;
const PADDING: f32 = 8.0;

/// What is offered on this message.
///
/// Delete only on the reader's own: the server would refuse it on anybody
/// else's, and offering something that will be refused is a worse answer than
/// not offering it.
pub fn offered(mine: bool) -> Vec<Action> {
    let mut offered = vec![
        Action::React,
        Action::Thread,
        Action::Save,
        Action::Pin,
        Action::Link,
    ];
    if mine {
        offered.push(Action::Edit);
        offered.push(Action::Delete);
    }
    offered
}

/// Where each button sits, laid out from the right edge inwards.
///
/// Right-aligned because the left is where the message is: the eye reads from
/// there, and a toolbar in the way of the first word would be worse than no
/// toolbar.
pub fn place(
    within: Rect,
    actions: &[Action],
    width_of: impl Fn(Action) -> f32,
) -> Vec<(Action, Rect)> {
    let mut placed = Vec::new();
    let mut right = within.right() - PADDING;
    // Reversed, so the order reads left to right once every width is known.
    for action in actions.iter().rev() {
        let width = width_of(*action) + PADDING * 2.0;
        right -= width;
        placed.push((*action, Rect::new(right, within.y, width, HEIGHT)));
        right -= GAP;
    }
    placed.reverse();
    placed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Offering a delete on somebody else's message would be offering something
    /// the server refuses.
    #[test]
    fn delete_is_only_offered_on_your_own() {
        assert!(!offered(false).contains(&Action::Delete));
        assert!(offered(true).contains(&Action::Delete));
    }

    /// Same for editing: the server refuses somebody else's message, and a
    /// button that will be refused is worse than one that is not there.
    #[test]
    fn edit_is_only_offered_on_your_own() {
        assert!(!offered(false).contains(&Action::Edit));
        assert!(offered(true).contains(&Action::Edit));
    }

    /// Every slug survives the round trip, or a click would land on nothing.
    #[test]
    fn a_slug_names_exactly_one_action() {
        for action in offered(true) {
            assert_eq!(Action::from_slug(action.slug()), Some(action));
        }
        assert_eq!(Action::from_slug("nonsense"), None);
    }

    /// Laid out inside the row, in order, without overlapping.
    #[test]
    fn the_buttons_sit_in_order_inside_the_row() {
        let row = Rect::new(100.0, 50.0, 600.0, 40.0);
        let placed = place(row, &offered(true), |_| 40.0);
        assert_eq!(placed.len(), 7);
        assert_eq!(placed[0].0, Action::React);
        for pair in placed.windows(2) {
            assert!(
                pair[0].1.right() <= pair[1].1.x,
                "{:?} overlaps {:?}",
                pair[0].0,
                pair[1].0
            );
        }
        let last = placed.last().expect("a button");
        assert!(last.1.right() <= row.right(), "the last one stays inside");
    }

    /// A state a button can be in has to read differently, or pressing it twice
    /// looks like nothing happened.
    #[test]
    fn a_toggled_button_says_which_way_it_is() {
        assert_ne!(Action::Save.label(false), Action::Save.label(true));
        assert_ne!(Action::Pin.label(false), Action::Pin.label(true));
    }
}
