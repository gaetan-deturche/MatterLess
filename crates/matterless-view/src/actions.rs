//! The things a reader can do to one message.
//!
//! Three controls float into the corner of the message under the pointer --
//! react, reply, more -- and everything else is behind the third. That is what
//! the app offers, and the port had it wrong: eight flat labelled buttons
//! across the top of every hovered message, which is the menu spilled into the
//! row. A toolbar says what it offers without being opened, but only while it
//! is small enough to read at a glance.

use matterless_ui::Rect;

/// One of the three controls on a hovered message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    /// Opens the whole picker.
    ///
    /// It used to open a row of seven faces with the picker behind them. The
    /// reader's own most-used sit on the strip now, which answers "the one I
    /// always reach for" better and in no presses at all -- so the shortlist
    /// was a second answer to a question already answered beside it, and a
    /// press to reach the grid that was not.
    React,
    /// Opens this message's thread.
    Reply,
    /// Opens the menu, which is where the rest of it lives.
    More,
}

impl Tool {
    pub fn slug(self) -> &'static str {
        match self {
            Tool::React => "react",
            Tool::Reply => "reply",
            Tool::More => "more",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        Some(match slug {
            "react" => Tool::React,
            "reply" => Tool::Reply,
            "more" => Tool::More,
            _ => return None,
        })
    }

    /// What it is for, said after the pointer has rested on it.
    ///
    /// Three glyphs across the corner of a message say nothing on their own,
    /// and the app carries exactly these three words on them.
    pub fn explains(self) -> &'static str {
        match self {
            Tool::React => "Add a reaction",
            Tool::Reply => "Reply in thread",
            Tool::More => "More actions",
        }
    }

    /// What is drawn on it. One glyph each, the app's own.
    pub fn mark(self) -> &'static str {
        match self {
            Tool::React => matterless_layout::marks::REACT,
            Tool::Reply => matterless_layout::marks::REPLY,
            Tool::More => matterless_layout::marks::MORE,
        }
    }
}

pub const TOOLS: [Tool; 3] = [Tool::React, Tool::Reply, Tool::More];

/// A 24px square, 3px between two of them, in a strip padded 3px.
const SQUARE: f32 = 24.0;
const GAP: f32 = 3.0;
const PADDING: f32 = 3.0;
/// `border-radius: 8px` on the strip, 5 on a square inside it.
pub const CORNER: f32 = 8.0;
pub const SQUARE_CORNER: f32 = 5.0;
/// How tall the strip is, which is what a row has to leave room for.
pub const HEIGHT: f32 = PADDING * 2.0 + SQUARE;

/// How many of the reader's most-used reactions sit on the strip itself.
///
/// The official client puts three there, and they are worth their room: the
/// reaction anybody makes most often is one press rather than a press, a row
/// of faces and another press. Not a `Tool`, because a tool is a fixed button
/// with a static name and a glyph of its own, and these are data -- whichever
/// three the reader has used most.
pub const FAVOURITES: usize = 3;

/// Where the strip floats: `top: -2px; right: 4px` of the message.
pub fn strip(within: Rect, favourites: usize) -> Rect {
    let buttons = TOOLS.len() + favourites;
    let width = PADDING * 2.0 + buttons as f32 * SQUARE + (buttons - 1) as f32 * GAP;
    Rect::new(within.right() - 4.0 - width, within.y - 2.0, width, HEIGHT)
}

/// Where each of the reader's most-used sits, before the controls.
///
/// First on the strip because they are what gets pressed: a reaction the
/// reader makes every day should not be behind the button for the ones they
/// do not.
pub fn favourites(within: Rect, many: usize) -> Vec<(usize, Rect)> {
    let strip = strip(within, many);
    (0..many).map(|at| (at, square(strip, at))).collect()
}

/// Where each control sits inside it, after the faces.
pub fn tools(within: Rect, favourites: usize) -> Vec<(Tool, Rect)> {
    let strip = strip(within, favourites);
    TOOLS
        .iter()
        .enumerate()
        .map(|(at, tool)| (*tool, square(strip, favourites + at)))
        .collect()
}

/// The nth square along the strip.
fn square(strip: Rect, at: usize) -> Rect {
    Rect::new(
        strip.x + PADDING + at as f32 * (SQUARE + GAP),
        strip.y + PADDING,
        SQUARE,
        SQUARE,
    )
}

/// What one item in the menu behind `...` does.
///
/// Not every one of them travels to the socket thread: opening an editor or a
/// forward picker happens in the window, and only the ones that change
/// something on the server become an `Ask`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Open the picker, to add a reaction that is not on the message already.
    React,
    /// Open this message's thread.
    Thread,
    /// Keep hearing about the thread, or stop.
    Follow,
    /// Mark the channel unread from this message down.
    Unread,
    /// Keep it, or stop keeping it.
    Save,
    /// Pin it to the channel, for everyone.
    Pin,
    /// Copy a permalink.
    Link,
    /// Copy what it says.
    CopyText,
    /// Send a link to it into another conversation.
    Forward,
    /// Be told about it again later.
    Remind,
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
            Action::Follow => "follow",
            Action::Unread => "unread",
            Action::Save => "save",
            Action::Pin => "pin",
            Action::Link => "link",
            Action::CopyText => "copytext",
            Action::Forward => "forward",
            Action::Remind => "remind",
            Action::Edit => "edit",
            Action::Delete => "delete",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        Some(match slug {
            "react" => Action::React,
            "thread" => Action::Thread,
            "follow" => Action::Follow,
            "unread" => Action::Unread,
            "save" => Action::Save,
            "pin" => Action::Pin,
            "link" => Action::Link,
            "copytext" => Action::CopyText,
            "forward" => Action::Forward,
            "remind" => Action::Remind,
            "edit" => Action::Edit,
            "delete" => Action::Delete,
            _ => return None,
        })
    }

    /// What the menu row says, given how the message already stands.
    ///
    /// A toggle names what pressing it will do, not what is true now: "Save
    /// Message" on one that is not saved and "Remove from Saved" on one that
    /// is. Getting that backwards is how a reader unsaves something twice.
    pub fn said(self, on: bool) -> &'static str {
        match (self, on) {
            (Action::React, _) => "Add a reaction",
            (Action::Thread, _) => "Reply in thread",
            (Action::Follow, false) => "Follow thread",
            (Action::Follow, true) => "Unfollow thread",
            (Action::Unread, _) => "Mark as Unread",
            (Action::Save, false) => "Save Message",
            (Action::Save, true) => "Remove from Saved",
            (Action::Pin, false) => "Pin to Channel",
            (Action::Pin, true) => "Unpin from Channel",
            (Action::Link, _) => "Copy Link",
            (Action::CopyText, _) => "Copy Text",
            (Action::Forward, _) => "Forward\u{2026}",
            (Action::Remind, _) => "Remind me\u{2026}",
            (Action::Edit, _) => "Edit",
            (Action::Delete, _) => "Delete\u{2026}",
        }
    }
}

/// How long each "remind me" is, in seconds from now.
///
/// Tomorrow is nine in the morning rather than twenty-four hours out, which is
/// what somebody choosing "tomorrow" means by it.
pub fn reminders(now_minutes_past_midnight: i64) -> Vec<(&'static str, i64)> {
    // Nine hundred minutes into the day, or the next one if that is past.
    let morning = 9 * 60;
    let until = if now_minutes_past_midnight < morning {
        morning - now_minutes_past_midnight
    } else {
        24 * 60 - now_minutes_past_midnight + morning
    };
    vec![
        ("30 mins", 30 * 60),
        ("1 hour", 60 * 60),
        ("2 hours", 2 * 60 * 60),
        ("Tomorrow", (until * 60).max(60)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every slug survives the round trip, or a click would land on nothing.
    #[test]
    fn a_slug_names_exactly_one_action() {
        for action in [
            Action::React,
            Action::Thread,
            Action::Follow,
            Action::Unread,
            Action::Save,
            Action::Pin,
            Action::Link,
            Action::CopyText,
            Action::Forward,
            Action::Remind,
            Action::Edit,
            Action::Delete,
        ] {
            assert_eq!(Action::from_slug(action.slug()), Some(action));
        }
        assert_eq!(Action::from_slug("nonsense"), None);
        for tool in TOOLS {
            assert_eq!(Tool::from_slug(tool.slug()), Some(tool));
        }
    }

    /// A toggle has to name both directions, or pressing it twice looks like
    /// nothing happened.
    #[test]
    fn a_toggle_says_which_way_it_is() {
        assert_ne!(Action::Save.said(false), Action::Save.said(true));
        assert_ne!(Action::Pin.said(false), Action::Pin.said(true));
        assert_ne!(Action::Follow.said(false), Action::Follow.said(true));
    }

    /// The strip floats into the corner of the message, inside it, in order.
    #[test]
    fn the_three_tools_sit_in_the_corner_of_the_row() {
        let row = Rect::new(100.0, 50.0, 600.0, 80.0);
        let placed = tools(row, 0);
        assert_eq!(placed.len(), 3);
        assert_eq!(placed[0].0, Tool::React);
        assert_eq!(placed[2].0, Tool::More);
        for pair in placed.windows(2) {
            assert!(pair[0].1.right() <= pair[1].1.x, "they overlap");
        }
        let strip = strip(row, 0);
        assert!(strip.right() <= row.right(), "it stays inside the row");
        assert!(
            strip.x > row.x + row.width / 2.0,
            "and keeps out of the message's own first words"
        );
    }

    /// "Tomorrow" is the next nine in the morning, whichever side of it the
    /// reader is on -- and never so close that it fires before they look away.
    #[test]
    fn tomorrow_means_the_next_morning() {
        let early = reminders(7 * 60);
        assert_eq!(early[3].1, 2 * 60 * 60, "two hours later the same day");
        let late = reminders(22 * 60);
        assert_eq!(late[3].1, 11 * 60 * 60, "eleven hours, overnight");
        // On the hour itself, the next one rather than this instant.
        assert!(reminders(9 * 60)[3].1 >= 60);
    }
}
