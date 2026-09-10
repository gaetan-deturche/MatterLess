//! A list of messages from somewhere other than a channel.
//!
//! Saved and pinned are the same shape, and search is a third: all three are
//! lists of posts drawn the same way. One panel rather than three, for the
//! reason the app gives for sharing its own -- three copies would be three
//! places for a channel label or an author name to be resolved differently.

use crate::sidebar::Canvas;
use matterless_paint::Run;
use matterless_store::Store;
use matterless_ui::input::{Input, Key};
use matterless_ui::{Placed, Rect};

pub const NAME: &str = "listing";

const ROW: f32 = 52.0;
const PADDING: f32 = 10.0;
const TITLE: f32 = 26.0;
const WIDTH: f32 = 620.0;

/// One message in a list, resolved to what a row needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub post_id: String,
    pub channel_id: String,
    /// The conversation it was said in, named the way the sidebar names it: a
    /// result out of context is unreadable without it.
    pub channel: String,
    pub author: String,
    /// One line of it, which is all a row has room for.
    pub preview: String,
    /// The thread this belongs to, so choosing it can open the conversation
    /// *and* the thread. Empty when the message is not in one.
    pub root_id: String,
    /// A short trailing fact -- "4 replies". Empty when there is none, rather
    /// than a placeholder nobody reads.
    pub note: String,
}

/// Resolves posts into rows, naming their authors and conversations.
///
/// In one place, because the alternative is each panel resolving a direct
/// message's label slightly differently -- a DM has no display name of its
/// own, so labelling one means looking up the other person.
pub fn found_for(store: &Store, posts: Vec<matterless_core::Post>, me: &str) -> Vec<Found> {
    let mut people: Vec<String> = posts.iter().map(|post| post.user_id.clone()).collect();
    let mut channels: std::collections::HashMap<String, matterless_core::model::Channel> =
        std::collections::HashMap::new();
    for post in &posts {
        if let Ok(Some(channel)) = store.channel(&post.channel_id) {
            if channel.channel_type == "D"
                && let Some(other) = matterless_sidebar::counterpart(&channel.name, me)
            {
                people.push(other);
            }
            channels.insert(post.channel_id.clone(), channel);
        }
    }
    people.sort();
    people.dedup();
    let known = store.users_by_ids(&people).unwrap_or_default();
    let names: std::collections::HashMap<String, String> = known
        .iter()
        .map(|(id, user)| (id.clone(), user.username.clone()))
        .collect();

    posts
        .into_iter()
        .map(|post| Found {
            channel: match channels.get(&post.channel_id) {
                Some(channel) => matterless_sidebar::label(channel, me, &names),
                // A conversation this store has never met is still an answer,
                // and an id says more than a blank.
                None => post.channel_id.clone(),
            },
            author: names.get(&post.user_id).cloned().unwrap_or(post.user_id),
            preview: matterless_sync::notify::preview_of(&post.message),
            root_id: post.root_id,
            note: String::new(),
            post_id: post.id,
            channel_id: post.channel_id,
        })
        .collect()
}

/// The threads this reader follows, newest reply first.
///
/// From the local store rather than the server: the sync engine already keeps
/// this table current, and a list of conversations you are in should open
/// instantly rather than after a round trip.
pub fn followed(store: &Store, me: &str, limit: u32) -> Vec<Found> {
    let threads = store.followed_threads(limit).unwrap_or_default();
    let mut people: Vec<String> = threads
        .iter()
        .map(|thread| thread.author_id.clone())
        .collect();
    let mut channels: std::collections::HashMap<String, matterless_core::model::Channel> =
        std::collections::HashMap::new();
    for thread in &threads {
        if let Ok(Some(channel)) = store.channel(&thread.channel_id) {
            if channel.channel_type == "D"
                && let Some(other) = matterless_sidebar::counterpart(&channel.name, me)
            {
                people.push(other);
            }
            channels.insert(thread.channel_id.clone(), channel);
        }
    }
    people.sort();
    people.dedup();
    let known = store.users_by_ids(&people).unwrap_or_default();
    let names: std::collections::HashMap<String, String> = known
        .iter()
        .map(|(id, user)| (id.clone(), user.username.clone()))
        .collect();

    threads
        .into_iter()
        .map(|thread| Found {
            channel: match channels.get(&thread.channel_id) {
                Some(channel) => matterless_sidebar::label(channel, me, &names),
                None => thread.channel_id.clone(),
            },
            author: names
                .get(&thread.author_id)
                .cloned()
                .unwrap_or(thread.author_id),
            preview: matterless_sync::notify::preview_of(&thread.message),
            note: note_for(thread.reply_count, thread.unread_replies),
            // The root is the thread, so choosing the row opens it.
            root_id: thread.root_id.clone(),
            post_id: thread.root_id,
            channel_id: thread.channel_id,
        })
        .collect()
}

/// What a thread row says about itself, in as few words as it can.
fn note_for(replies: i64, unread: i64) -> String {
    let plural = |count: i64| if count == 1 { "reply" } else { "replies" };
    if unread > 0 {
        // The unread count first: it is the reason to open this one rather
        // than the one under it.
        return format!("{unread} new of {replies} {}", plural(replies));
    }
    format!("{replies} {}", plural(replies))
}

/// A titled list, open or shut.
#[derive(Debug, Default)]
pub struct Listing {
    /// What it is a list of -- "Saved", "Pinned". Empty when it is shut.
    pub title: String,
    pub found: Vec<Found>,
    chosen: usize,
    /// True while the answer is still on its way, so an empty list can be told
    /// from one that came back empty.
    pub waiting: bool,
}

impl Listing {
    pub fn open(&self) -> bool {
        !self.title.is_empty()
    }

    /// Opens it empty, with the answer still to come.
    pub fn expect(&mut self, title: &str) {
        self.title = title.to_string();
        self.found.clear();
        self.chosen = 0;
        self.waiting = true;
    }

    pub fn fill(&mut self, found: Vec<Found>) {
        self.found = found;
        self.chosen = 0;
        self.waiting = false;
    }

    pub fn hide(&mut self) {
        self.title.clear();
        self.found.clear();
        self.waiting = false;
    }

    pub fn rect(&self, within: Rect) -> Rect {
        let height = PADDING * 2.0 + TITLE + self.found.len() as f32 * ROW;
        let width = WIDTH.min(within.width - 40.0);
        Rect::new(
            within.x + (within.width - width) / 2.0,
            within.y + 60.0,
            width,
            height.min(within.height - 120.0).max(TITLE + PADDING * 2.0),
        )
    }

    /// Applies a frame's input. Answers the message a reader chose.
    pub fn react(&mut self, input: &Input) -> Option<Found> {
        if !self.open() {
            return None;
        }
        if input.struck(Key::Down) && !self.found.is_empty() {
            self.chosen = (self.chosen + 1).min(self.found.len() - 1);
        }
        if input.struck(Key::Up) {
            self.chosen = self.chosen.saturating_sub(1);
        }
        if let Some(clicked) = input.clicked()
            && let Some(at) = clicked
                .strip_prefix(&format!("{NAME}/"))
                .and_then(|at| at.parse::<usize>().ok())
        {
            return self.found.get(at).cloned();
        }
        if input.struck(Key::Enter) {
            return self.found.get(self.chosen).cloned();
        }
        None
    }

    /// Where each row sits, so a click can land on one.
    pub fn boxes(&self, within: Rect) -> Vec<Placed> {
        if !self.open() {
            return Vec::new();
        }
        let panel = self.rect(within);
        let mut placed = vec![Placed {
            name: NAME.to_string(),
            rect: panel,
            depth: 8,
        }];
        for at in 0..self.found.len() {
            let y = panel.y + PADDING + TITLE + at as f32 * ROW;
            if y + ROW > panel.bottom() {
                break;
            }
            placed.push(Placed {
                name: format!("{NAME}/{at}"),
                rect: Rect::new(panel.x, y, panel.width, ROW),
                depth: 9,
            });
        }
        placed
    }

    pub fn draw(&self, into: &mut Canvas<'_>, within: Rect) {
        if !self.open() {
            return;
        }
        let panel = self.rect(within);
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        scene.fill(panel.x, panel.y, panel.width, panel.height, palette.surface);
        let heading = painter.run(
            fonts,
            &self.title,
            panel.x + PADDING + 4.0,
            panel.y + PADDING,
            Run::label(f32::MAX).bold(),
        );
        scene.glyphs(heading, palette.ink, palette.faint);

        // An empty list has to say which kind of empty it is, or a slow request
        // and a genuinely empty list look identical.
        if self.found.is_empty() {
            // Not "asking the server": threads are answered from the store,
            // and a line that names the wrong source is worse than a vague one.
            let said = if self.waiting {
                "still looking"
            } else {
                "nothing here yet"
            };
            let glyphs = painter.run(
                fonts,
                said,
                panel.x + PADDING + 4.0,
                panel.y + PADDING + TITLE,
                Run::label(f32::MAX),
            );
            scene.glyphs(glyphs, palette.faint, palette.faint);
            return;
        }

        for (at, found) in self.found.iter().enumerate() {
            let y = panel.y + PADDING + TITLE + at as f32 * ROW;
            // Only what fits: the panel is capped so a long list does not make
            // a box taller than the window.
            if y + ROW > panel.bottom() {
                break;
            }
            if at == self.chosen {
                scene.fill(panel.x, y, panel.width, ROW, palette.ground);
            }
            let said = match found.note.as_str() {
                "" => format!("{} in {}", found.author, found.channel),
                note => format!("{} in {} -- {note}", found.author, found.channel),
            };
            let who = painter.run(
                fonts,
                &said,
                panel.x + PADDING + 4.0,
                y + 4.0,
                Run::label(f32::MAX).bold(),
            );
            scene.glyphs(who, palette.ink, palette.faint);
            let what = painter.run(
                fonts,
                &found.preview,
                panel.x + PADDING + 4.0,
                y + 24.0,
                // Cut by the panel rather than wrapped: a row is one line, and
                // a wrapped one would run into the row beneath it.
                Run::label(f32::MAX),
            );
            scene.glyphs(what, palette.faint, palette.faint);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(count: usize) -> Vec<Found> {
        (0..count)
            .map(|at| Found {
                post_id: format!("p{at}"),
                channel_id: "c1".into(),
                channel: "Dev".into(),
                author: "ada".into(),
                preview: "something".into(),
                root_id: String::new(),
                note: String::new(),
            })
            .collect()
    }

    /// The unread count leads, because it is the reason to open one thread
    /// rather than the one beneath it. And one reply is not "1 replies".
    #[test]
    fn a_thread_says_what_is_waiting_in_it() {
        assert_eq!(note_for(4, 2), "2 new of 4 replies");
        assert_eq!(note_for(4, 0), "4 replies");
        assert_eq!(note_for(1, 0), "1 reply");
        assert_eq!(note_for(1, 1), "1 new of 1 reply");
    }

    /// The panel never grows past the window, however long the list.
    #[test]
    fn the_panel_stays_inside_the_window() {
        let mut listing = Listing::default();
        listing.expect("Saved");
        listing.fill(found(500));
        let window = Rect::new(0.0, 0.0, 1000.0, 700.0);
        let panel = listing.rect(window);
        assert!(panel.height <= window.height - 120.0);
        assert!(panel.bottom() <= window.bottom());
    }

    /// A shut panel answers nothing and places nothing, whatever is pressed.
    #[test]
    fn a_shut_listing_answers_nothing() {
        let mut listing = Listing::default();
        let window = Rect::new(0.0, 0.0, 800.0, 600.0);
        assert!(listing.react(&Input::default()).is_none());
        assert!(listing.boxes(window).is_empty());
    }

    /// Waiting for an answer and having none are different things: one is a
    /// slow request and the other is a list with nothing in it.
    #[test]
    fn an_empty_list_says_which_kind_of_empty_it_is() {
        let mut listing = Listing::default();
        listing.expect("Pinned");
        assert!(listing.open());
        assert!(listing.waiting);
        listing.fill(Vec::new());
        assert!(listing.open());
        assert!(!listing.waiting);
    }

    /// Only the rows that fit are placed, or a click below the panel would
    /// land on a row drawn outside it.
    #[test]
    fn no_row_is_placed_outside_the_panel() {
        let mut listing = Listing::default();
        listing.expect("Saved");
        listing.fill(found(500));
        let window = Rect::new(0.0, 0.0, 1000.0, 700.0);
        let panel = listing.rect(window);
        for placed in listing.boxes(window).iter().skip(1) {
            assert!(placed.rect.bottom() <= panel.bottom());
        }
    }
}
