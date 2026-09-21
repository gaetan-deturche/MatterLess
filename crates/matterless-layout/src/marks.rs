//! The interface's own marks.
//!
//! Lucide, bundled: an icon set drawn for this, on a 24-pixel grid with one
//! stroke weight, rather than whatever the system's symbol font happens to
//! have under a given codepoint. The marks before this came out of Segoe UI
//! Symbol, which is a drawer of miscellaneous Unicode and not an icon set --
//! its pushpin is a picture of a pushpin, next to a flag that is a solid
//! shape, next to three lines, and at sixteen pixels they did not read as one
//! family or, in places, as anything.
//!
//! Named rather than written as codepoints at the places that use them, and
//! generated from the font's own `codepoints.json` rather than typed: a mark
//! is four hex digits, and four hex digits typed by hand is a wrong icon that
//! compiles.
//!
//! ISC, with an MIT notice for the icons derived from Feather. Both travel
//! with the font in `LUCIDE-LICENSE` beside it.

/// What the font calls itself, which is how a run asks for it.
pub const FAMILY: &str = "lucide";

/// The followed threads, in the sidebar and on the strip.  `spool`
///
/// Not `menu`, which is what this was: three stacked lines is the hamburger,
/// and in every other application it means "the menu is in here". A reader
/// who has seen one before reads it as a menu that will not open. A spool is
/// thread, which is what the list is a list of.
pub const THREADS: &str = "\u{e677}";
/// Messages pinned to this conversation.  `pin`
pub const PINNED: &str = "\u{e259}";
/// Messages this reader has kept.  `bookmark`
pub const SAVED: &str = "\u{e060}";
/// Add somebody to this conversation.  `user-plus`
pub const ADD_PEOPLE: &str = "\u{e1a2}";
/// Notifications on.  `bell`
pub const BELL: &str = "\u{e059}";
/// Muted.  `bell-off`
pub const BELL_OFF: &str = "\u{e05a}";
/// Leave this conversation.  `log-out`
pub const LEAVE: &str = "\u{e10e}";
/// Shut whatever is open.  `x`
pub const CLOSE: &str = "\u{e1b2}";
/// Attach a file to what is being written.  `paperclip`
pub const ATTACH: &str = "\u{e12d}";
/// Add a reaction to a message.  `smile`
pub const REACT: &str = "\u{e164}";
/// Reply in a thread.  `corner-up-left`
pub const REPLY: &str = "\u{e0a7}";
/// Everything else that can be done to a message.  `ellipsis`
pub const MORE: &str = "\u{e0b6}";
/// Mark a conversation unread.  `mail`
pub const UNREAD: &str = "\u{e10f}";
/// The button that goes to where the reader stopped reading.  `message-square-dot`
///
/// A message with something on it, which is what the mark stands in front
/// of. Not `UNREAD`: that envelope is the message menu's "mark unread", and
/// on the rail it sat directly above a button that was also an envelope.
pub const TO_UNREAD: &str = "\u{e56e}";
/// The conversations that are with somebody rather than about a subject.  `messages-circle`
///
/// Qualified wherever it is used: the rail has a `DIRECTS` of its own, which
/// is what that button answers to rather than what it looks like.
pub const DIRECTS: &str = "\u{e773}";
/// Keep a conversation at the top.  `star`
pub const FAVOURITE: &str = "\u{e176}";
/// Move it to another category.  `folder`
pub const FOLDER: &str = "\u{e0d7}";
/// Copy a link to it.  `link`
pub const LINK: &str = "\u{e102}";
/// Start a conversation.  `plus`
pub const NEW: &str = "\u{e13d}";
/// The previous picture.  `chevron-left`
pub const BACK: &str = "\u{e06e}";
/// The next picture, and a submenu.  `chevron-right`
pub const NEXT: &str = "\u{e06f}";
/// Keep a file next to the reader’s other downloads.  `download`
pub const SAVE_FILE: &str = "\u{e0b2}";
/// Delete a message.  `trash-2`
pub const DELETE: &str = "\u{e18e}";
/// Change a message.  `pencil`
pub const EDIT: &str = "\u{e1f9}";
/// Copy the words.  `copy`
pub const COPY: &str = "\u{e09e}";
/// Be reminded about it later.  `clock`
pub const REMIND: &str = "\u{e087}";
/// Send it somewhere else.  `forward`
pub const FORWARD: &str = "\u{e229}";
/// Find a message already said.  `search`
pub const SEARCH: &str = "\u{e151}";
