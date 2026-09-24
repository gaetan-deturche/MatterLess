//! A list of messages from somewhere other than a channel.
//!
//! Saved and pinned are the same shape, and search is a third: all three are
//! lists of posts drawn the same way. One panel rather than three, for the
//! reason the app gives for sharing its own -- three copies would be three
//! places for a channel label or an author name to be resolved differently.
//!
//! Drawn down the right-hand column rather than over the conversation. See
//! `aside`, which holds that decision and the shape the three share.

use crate::sidebar::Canvas;
use matterless_paint::Run;
use matterless_store::Store;
use matterless_ui::input::{Input, Key};
use matterless_ui::{Placed, Rect};

pub const NAME: &str = "listing";

use crate::aside;

const ROW: f32 = aside::ROW;
const PADDING: f32 = aside::PADDING;

/// What a frame of input did to the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Did {
    /// Go to this message.
    Open(Box<Found>),
    /// Throw this row's thing away -- a draft, from the Drafts list.
    Clear(Box<Found>),
    /// Shut the pane.
    Close,
}

/// The cross at the end of a row that can be thrown away.
const CROSS: f32 = 28.0;

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
/// The words somebody is looking for, out of what they typed.
///
/// Modifiers are dropped: `from:ada` filters the answer rather than appearing
/// in it, so lighting up the letters "from" in a message would be pointing at
/// the wrong thing.
pub fn wanted(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .filter(|word| !word.contains(':'))
        .map(|word| word.trim_matches('"').to_lowercase())
        .filter(|word| !word.is_empty())
        .collect()
}

/// A line split into the stretches that matched and the stretches that did
/// not.
///
/// Split at word boundaries and compared word by word rather than by searching
/// the line for the query: lowercasing can change how many bytes a character
/// takes, so an offset found in a lowercased copy does not always point at the
/// same place in the original. Whole words are also what the reader is looking
/// at -- and matching a prefix is what the database did to find this row, so
/// `budg` lighting up `budget` is the truth about why it is here.
pub fn picked_out(text: &str, wanted: &[String]) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    let mut word = String::new();
    let mut between = String::new();

    let push = |piece: &str, lit: bool, out: &mut Vec<(String, bool)>| {
        if piece.is_empty() {
            return;
        }
        match out.last_mut() {
            Some((held, was)) if *was == lit => held.push_str(piece),
            _ => out.push((piece.to_string(), lit)),
        }
    };

    for character in text.chars() {
        if character.is_alphanumeric() {
            if !between.is_empty() {
                push(&between, false, &mut out);
                between.clear();
            }
            word.push(character);
            continue;
        }
        if !word.is_empty() {
            let lit = lights(&word, wanted);
            push(&word, lit, &mut out);
            word.clear();
        }
        between.push(character);
    }
    if !word.is_empty() {
        let lit = lights(&word, wanted);
        push(&word, lit, &mut out);
    }
    push(&between, false, &mut out);
    out
}

fn lights(word: &str, wanted: &[String]) -> bool {
    let word = word.to_lowercase();
    wanted.iter().any(|term| word.starts_with(term.as_str()))
}

/// How much of the message to show either side of what was found.
///
/// A result row is one line, and a message is not: something matched deep in a
/// long one used to leave the row showing its opening words, which say nothing
/// about why it is a result at all.
const AROUND: usize = 110;

/// How much room to leave before the match, so it reads as a piece of a
/// sentence rather than as the start of one.
const LEAD: usize = 24;

/// The part of a message worth showing, given what somebody was looking for.
///
/// With nothing to look for -- saved, pinned, a thread's own list -- it is the
/// opening of the message, which is what it always was.
pub fn preview_around(said: &str, wanted: &[String]) -> String {
    let line: String = said
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let letters: Vec<char> = line.chars().collect();

    let found = found_at(&line, wanted);
    // Back off a little so the match has a run-up, and to a word boundary so
    // the line does not start mid-word.
    let from = match found {
        Some(at) => back_to_a_space(&letters, at.saturating_sub(LEAD)),
        None => 0,
    };
    let to = (from + AROUND).min(letters.len());
    let mut out = String::new();
    if from > 0 {
        out.push('\u{2026}');
    }
    out.extend(&letters[from..to]);
    if to < letters.len() {
        out.push('\u{2026}');
    }
    out
}

/// Where the first word that matched starts, in characters.
fn found_at(line: &str, wanted: &[String]) -> Option<usize> {
    if wanted.is_empty() {
        return None;
    }
    let mut at = 0;
    for (piece, lit) in picked_out(line, wanted) {
        if lit {
            return Some(at);
        }
        at += piece.chars().count();
    }
    None
}

/// The last space at or before `at`, so a window starts on a word.
fn back_to_a_space(letters: &[char], at: usize) -> usize {
    if at == 0 {
        return 0;
    }
    letters[..at]
        .iter()
        .rposition(|character| character.is_whitespace())
        .map(|space| space + 1)
        .unwrap_or(at)
}

pub fn found_for(
    store: &Store,
    posts: Vec<matterless_core::Post>,
    me: &str,
    wanted: &[String],
) -> Vec<Found> {
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
    // The reader too, so a group conversation's name can have their own out
    // of it: it is a list of who is in the conversation, and reading your own
    // name there says nothing.
    people.push(me.to_string());
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
            // The message as it reads, not as it was written: a result row
            // is one line of prose and the marks are instructions for a
            // renderer -- a heading that came back as `### **Florent**:` was
            // showing the reader the punctuation instead of the sentence. And
            // the part of it worth showing, which is the part around what was
            // found.
            preview: preview_around(
                &matterless_render::markdown::plain_lines(&matterless_render::markdown::parse(
                    &post.message,
                )),
                wanted,
            ),
            root_id: post.root_id,
            note: String::new(),
            post_id: post.id,
            channel_id: post.channel_id,
        })
        .collect()
}

/// What a row's two lines are set in, for measuring where to cut one.
///
/// Shared with search, whose results are the same two lines.
pub fn label(bold: bool) -> matterless_layout::Style {
    matterless_layout::Style {
        size: 13.0,
        line_height: 18.0,
        bold,
        italic: false,
        mono: false,
    }
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
    // The reader too, so a group conversation's name can have their own out
    // of it: it is a list of who is in the conversation, and reading your own
    // name there says nothing.
    people.push(me.to_string());
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
#[derive(Debug)]
pub struct Listing {
    /// What this one answers to, so a press can tell two of them apart.
    ///
    /// Two are on screen at once: the followed threads, which are a place to
    /// go and so fill the conversation's own column, and saved or pinned,
    /// which are asides read against whatever is open.
    pub name: String,
    /// What it is a list of -- "Saved", "Pinned". Empty when it is shut.
    pub title: String,
    pub found: Vec<Found>,
    chosen: usize,
    /// True while the answer is still on its way, so an empty list can be told
    /// from one that came back empty.
    pub waiting: bool,
    /// Whether each row carries a cross that throws it away. The Drafts list
    /// only: a saved message or a pinned one is not this list's to discard.
    pub clearable: bool,
    scroll: f32,
    bar: crate::scrollbar::Scrollbar,
}

impl Default for Listing {
    fn default() -> Self {
        Self::new(NAME)
    }
}

impl Listing {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            title: String::new(),
            found: Vec::new(),
            chosen: 0,
            waiting: false,
            clearable: false,
            scroll: 0.0,
            bar: crate::scrollbar::Scrollbar::default(),
        }
    }

    pub fn open(&self) -> bool {
        !self.title.is_empty()
    }

    /// Opens it empty, with the answer still to come.
    pub fn expect(&mut self, title: &str) {
        self.title = title.to_string();
        self.found.clear();
        self.chosen = 0;
        self.waiting = true;
        // Whoever opens the one list that can discard says so after this.
        self.clearable = false;
    }

    pub fn fill(&mut self, found: Vec<Found>) {
        self.found = found;
        self.chosen = 0;
        self.waiting = false;
        self.scroll = 0.0;
    }

    pub fn hide(&mut self) {
        self.title.clear();
        self.found.clear();
        self.waiting = false;
        self.scroll = 0.0;
    }

    /// How far the list can travel: what it would occupy, less the room it has.
    fn reach(&self, body: Rect) -> f32 {
        let wanted = self.found.len() as f32 * ROW + PADDING * 2.0;
        (wanted - body.height).max(0.0)
    }

    /// Where one row's cross sits, at its right end, clear of the bar.
    fn cross_rect(&self, body: Rect, at: usize) -> Rect {
        let row = self.row_rect(body, at);
        Rect::new(
            row.right() - crate::scrollbar::TRACK - CROSS,
            row.y + (row.height - CROSS) / 2.0,
            CROSS,
            CROSS,
        )
    }

    /// Where one row sits, wherever the list has been scrolled to.
    fn row_rect(&self, body: Rect, at: usize) -> Rect {
        Rect::new(
            body.x,
            body.y + PADDING + at as f32 * ROW - self.scroll,
            body.width,
            ROW,
        )
    }

    /// Applies a frame's input to a list drawn as an aside.
    pub fn react(&mut self, input: &Input, placed: &[Placed], pane: Rect) -> Option<Did> {
        self.react_in(input, placed, aside::body(pane))
    }

    /// The same, for a list that fills a column of its own.
    pub fn react_in(&mut self, input: &Input, placed: &[Placed], body: Rect) -> Option<Did> {
        if !self.open() {
            return None;
        }
        let reach = self.reach(body);
        // The bar first: while it is held nothing else may write the scroll.
        if let Some(scroll) = self.bar.react(&self.name, input, body, self.scroll, reach) {
            self.scroll = scroll.clamp(0.0, reach);
            return None;
        }
        if let Some((_, y)) = input.wheel_over(placed, |name| name == self.name) {
            self.scroll = (self.scroll - y).clamp(0.0, reach);
        }
        // A list that shrank under a reader who had scrolled down would
        // otherwise leave them below the end of it, looking at nothing.
        self.scroll = self.scroll.clamp(0.0, reach);
        if input.struck(Key::Down) && !self.found.is_empty() {
            self.chosen = (self.chosen + 1).min(self.found.len() - 1);
        }
        if input.struck(Key::Up) {
            self.chosen = self.chosen.saturating_sub(1);
        }
        if let Some(clicked) = input.clicked() {
            if clicked == format!("{}/close", self.name) {
                return Some(Did::Close);
            }
            if let Some(at) = clicked
                .strip_prefix(&format!("{}/", self.name))
                .and_then(|rest| rest.strip_suffix("/clear"))
                .and_then(|at| at.parse::<usize>().ok())
            {
                return self.found.get(at).cloned().map(Box::new).map(Did::Clear);
            }
            if let Some(at) = clicked
                .strip_prefix(&format!("{}/", self.name))
                .and_then(|at| at.parse::<usize>().ok())
            {
                return self.found.get(at).cloned().map(Box::new).map(Did::Open);
            }
        }
        if input.struck(Key::Enter) {
            return self
                .found
                .get(self.chosen)
                .cloned()
                .map(Box::new)
                .map(Did::Open);
        }
        None
    }

    /// Where each row sits when it is drawn as an aside.
    pub fn boxes(&self, pane: Rect) -> Vec<Placed> {
        if !self.open() {
            return Vec::new();
        }
        let mut placed = vec![
            Placed {
                name: self.name.clone(),
                rect: pane,
                depth: 8,
            },
            aside::close_box(pane, &self.name),
        ];
        placed.extend(self.boxes_in(aside::body(pane), 9));
        placed
    }

    /// The rows themselves, at whatever depth the caller draws them.
    ///
    /// A column of its own is behind the panels that float over it, and an
    /// aside is in front of the conversation, so the depth is the caller's.
    pub fn boxes_in(&self, body: Rect, depth: usize) -> Vec<Placed> {
        if !self.open() {
            return Vec::new();
        }
        let mut placed = vec![Placed {
            name: self.name.clone(),
            rect: body,
            depth: depth.saturating_sub(1),
        }];
        placed.extend(self.bar.boxes(&self.name, body, self.reach(body)));
        for at in 0..self.found.len() {
            let row = self.row_rect(body, at);
            // Only what is on screen. A row scrolled above the top or past the
            // foot is still in the list, and a click where it would have been
            // must not choose it.
            if row.bottom() <= body.y || row.y >= body.bottom() {
                continue;
            }
            placed.push(Placed {
                name: format!("{}/{at}", self.name),
                rect: row,
                depth,
            });
            // After the row it sits on: a hit takes the last box it lands in.
            if self.clearable {
                placed.push(Placed {
                    name: format!("{}/{at}/clear", self.name),
                    rect: self.cross_rect(body, at),
                    depth,
                });
            }
        }
        placed
    }

    /// Drawn as an aside: its own ground, a title, and a way out.
    pub fn draw(&self, into: &mut Canvas<'_>, input: &Input, pane: Rect) {
        if !self.open() {
            return;
        }
        let head = aside::header(pane);
        aside::ground(into, pane);
        aside::draw_close(into, pane);
        let heading = into.painter.run(
            into.fonts,
            &self.title,
            head.x + PADDING,
            head.y + (aside::HEADER - 18.0) / 2.0,
            Run::label(f32::MAX).bold(),
        );
        into.scene
            .glyphs(heading, into.palette.ink, into.palette.faint);
        self.draw_in(into, input, aside::body(pane));
    }

    /// The list itself, filling whatever it is given.
    ///
    /// No ground of its own and no title: a column carries the conversation's
    /// own, and an aside has drawn both already.
    pub fn draw_in(&self, into: &mut Canvas<'_>, input: &Input, body: Rect) {
        if !self.open() {
            return;
        }
        let reach = self.reach(body);
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;

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
                body.x + PADDING,
                body.y + PADDING,
                Run::label(f32::MAX),
            );
            scene.glyphs(glyphs, palette.faint, palette.faint);
            return;
        }

        // Clipped to the body, so a row scrolled halfway off the top is cut at
        // the header rather than drawn across it.
        scene.clip_to(body.x, body.y, body.width, body.height);
        // The room one line gets: the pane less its padding both sides and the
        // bar that floats over the right of it.
        // And the cross, where there is one, so a long line stops short of it.
        let room = body.width
            - PADDING * 2.0
            - crate::scrollbar::TRACK
            - if self.clearable { CROSS } else { 0.0 };
        for (at, found) in self.found.iter().enumerate() {
            let row = self.row_rect(body, at);
            if row.bottom() <= body.y || row.y >= body.bottom() {
                continue;
            }
            if at == self.chosen {
                scene.fill(row.x, row.y, row.width, row.height, palette.ground);
            }
            let said = match found.note.as_str() {
                "" => format!("{} in {}", found.author, found.channel),
                note => format!("{} in {} \u{2014} {note}", found.author, found.channel),
            };
            // Cut with an ellipsis rather than by the clip: a name that ends
            // mid-letter gives a reader no way to tell a long one from one
            // that happens to end there.
            let said = matterless_layout::elided(fonts, &said, room, label(true));
            let who = painter.run(
                fonts,
                &said,
                row.x + PADDING,
                row.y + 6.0,
                Run::label(f32::MAX).bold(),
            );
            scene.glyphs(who, palette.ink, palette.faint);
            let preview = matterless_layout::elided(fonts, &found.preview, room, label(false));
            let what = painter.run(
                fonts,
                &preview,
                row.x + PADDING,
                row.y + 26.0,
                Run::label(f32::MAX),
            );
            scene.glyphs(what, palette.faint, palette.faint);
            if self.clearable {
                let cross = self.cross_rect(body, at);
                let under = input.hovered() == Some(format!("{}/{at}/clear", self.name).as_str());
                let mark = painter.run(
                    fonts,
                    matterless_layout::marks::CLOSE,
                    cross.x + 8.0,
                    cross.y + 6.0,
                    Run::mark(12.0),
                );
                scene.glyphs(
                    mark,
                    if under { palette.ink } else { palette.faint },
                    palette.faint,
                );
            }
        }
        let mut canvas = Canvas {
            scene,
            painter,
            fonts,
            palette,
        };
        self.bar
            .draw(&mut canvas, &self.name, input, body, self.scroll, reach);
        // Back to the whole window, so whatever is drawn after this is not
        // clipped to a pane it has nothing to do with.
        canvas.scene.clip_to(0.0, 0.0, f32::MAX, f32::MAX);
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

    fn pane() -> Rect {
        aside::rect(Rect::new(300.0, 0.0, 1000.0, 700.0), aside::WIDTH)
    }

    /// A click where the pointer is, as the window's input sees one.
    fn click(input: &mut Input, placed: &[Placed], x: f32, y: f32) {
        input.apply(matterless_ui::input::Event::PointerMoved { x, y }, placed);
        input.apply(matterless_ui::input::Event::PointerPressed, placed);
        input.apply(matterless_ui::input::Event::PointerReleased, placed);
    }

    /// A draft is thrown away from its own row: the cross at its end answers
    /// that row, and it wins over the row it sits on -- a press on it must
    /// not also open the conversation.
    #[test]
    fn a_draft_is_thrown_away_from_its_own_row() {
        let mut listing = Listing::default();
        listing.expect("Drafts");
        listing.clearable = true;
        listing.fill(found(3));
        let placed = listing.boxes(pane());
        let cross = placed
            .iter()
            .find(|one| one.name == format!("{NAME}/1/clear"))
            .expect("a cross on the second row")
            .rect;
        let mut input = Input::default();
        click(
            &mut input,
            &placed,
            cross.x + cross.width / 2.0,
            cross.y + cross.height / 2.0,
        );
        match listing.react(&input, &placed, pane()) {
            Some(Did::Clear(row)) => assert_eq!(row.post_id, "p1", "the wrong row"),
            other => panic!("pressing the cross did {other:?}"),
        }
    }

    /// Only the list that says so has crosses: a saved or pinned message is
    /// not this list's to discard, and the pane is the same pane.
    #[test]
    fn only_a_list_that_says_so_has_crosses() {
        let mut listing = Listing::default();
        listing.expect("Drafts");
        listing.clearable = true;
        listing.fill(found(2));
        assert!(
            listing
                .boxes(pane())
                .iter()
                .any(|one| one.name.ends_with("/clear"))
        );
        listing.expect("Saved");
        listing.fill(found(2));
        assert!(
            !listing
                .boxes(pane())
                .iter()
                .any(|one| one.name.ends_with("/clear")),
            "the Saved list kept the Drafts list's crosses"
        );
    }

    /// A shut panel answers nothing and places nothing, whatever is pressed.
    #[test]
    fn a_shut_listing_answers_nothing() {
        let mut listing = Listing::default();
        assert!(listing.react(&Input::default(), &[], pane()).is_none());
        assert!(listing.boxes(pane()).is_empty());
    }

    /// A long list can be reached: it scrolls rather than stopping at whatever
    /// happened to fit.
    ///
    /// The floating panel it used to be simply cut the list off at the foot of
    /// the window, so the fortieth saved message was in the list and on no
    /// screen. A column is full height and still not tall enough for five
    /// hundred.
    #[test]
    fn a_list_longer_than_the_column_can_be_scrolled_to_its_end() {
        let mut listing = Listing::default();
        listing.expect("Saved");
        listing.fill(found(500));
        let body = aside::body(pane());
        assert!(
            listing.reach(body) > 0.0,
            "500 rows do not fit in one column"
        );

        // At the top, the first row is inside the body and the last is far
        // below it.
        assert!(listing.row_rect(body, 0).y >= body.y);
        assert!(listing.row_rect(body, 499).y > body.bottom());

        // Scrolled to the end, the last row is the one on screen.
        listing.scroll = listing.reach(body);
        let last = listing.row_rect(body, 499);
        assert!(
            last.bottom() <= body.bottom() + 0.5 && last.y >= body.y,
            "the last row sits at {last:?} in a body ending at {}",
            body.bottom()
        );
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

    /// Only the rows on screen are placed, or a click would land on a row
    /// scrolled out of sight.
    #[test]
    fn no_row_is_placed_outside_the_column() {
        let mut listing = Listing::default();
        listing.expect("Saved");
        listing.fill(found(500));
        let pane = pane();
        let body = aside::body(pane);
        let rows: Vec<Placed> = listing
            .boxes(pane)
            .into_iter()
            .filter(|placed| {
                placed
                    .name
                    .strip_prefix(&format!("{NAME}/"))
                    .is_some_and(|rest| rest.parse::<usize>().is_ok())
            })
            .collect();
        assert!(!rows.is_empty(), "some rows are reachable");
        for row in rows {
            assert!(
                row.rect.bottom() > body.y && row.rect.y < body.bottom(),
                "{} is placed at {:?}, outside the body {body:?}",
                row.name,
                row.rect
            );
        }
    }

    /// The way out is a button, not only a key nobody has been told about.
    #[test]
    fn the_column_offers_a_way_out() {
        let mut listing = Listing::default();
        listing.expect("Saved");
        listing.fill(found(3));
        let pane = pane();
        assert!(
            listing
                .boxes(pane)
                .iter()
                .any(|placed| placed.name == format!("{NAME}/close")),
            "no close button"
        );
    }

    /// A result has to say why it is a result, so what was searched for is
    /// lit up in it.
    #[test]
    fn the_words_somebody_searched_for_are_picked_out() {
        let looking = wanted("budget");
        let split = picked_out("the budget looks wrong", &looking);
        assert_eq!(
            split,
            vec![
                ("the ".to_string(), false),
                ("budget".to_string(), true),
                (" looks wrong".to_string(), false),
            ]
        );
    }

    /// Whatever case it was said in, and a prefix counts: matching a prefix is
    /// what the database did to find the row, so lighting one is the truth
    /// about why it is here.
    #[test]
    fn a_prefix_in_any_case_lights_the_whole_word() {
        let looking = wanted("Budg");
        let split = picked_out("the BUDGET line", &looking);
        assert_eq!(split[1], ("BUDGET".to_string(), true));
    }

    /// A modifier filters the answer rather than appearing in it, so lighting
    /// up the letters of `from` would be pointing at the wrong thing.
    #[test]
    fn a_modifier_is_not_one_of_the_words() {
        assert_eq!(wanted("from:ada in:dev budget"), vec!["budget".to_string()]);
        let split = picked_out("from ada, the budget", &wanted("from:ada budget"));
        assert_eq!(split[0], ("from ada, the ".to_string(), false));
        assert_eq!(split[1], ("budget".to_string(), true));
    }

    /// Nothing typed lights nothing, and the line comes back whole rather than
    /// in pieces -- one run, which is what it was before any of this.
    #[test]
    fn a_line_with_nothing_to_light_is_one_piece() {
        let split = picked_out("the budget looks wrong", &wanted(""));
        assert_eq!(split, vec![("the budget looks wrong".to_string(), false)]);
    }

    /// Splitting is by word rather than by searching the line, because
    /// lowercasing can change how many bytes a character takes -- so an offset
    /// found in a lowercased copy does not always point at the same place in
    /// the original. Nothing may be lost or invented whatever is in the line.
    #[test]
    fn the_line_survives_being_split() {
        for said in [
            "the budget looks wrong",
            "\u{c9}t\u{e9} 2026 -- le budget",
            "\u{1f4a1} budget?",
            "",
            "   ",
            "budget",
        ] {
            let whole: String = picked_out(said, &wanted("budget"))
                .into_iter()
                .map(|(piece, _)| piece)
                .collect();
            assert_eq!(whole, said, "for {said:?}");
        }
    }

    /// A window onto the message rather than its opening.
    ///
    /// Something that matched deep in a long message used to leave the row
    /// showing the first hundred characters, which say nothing about why it is
    /// a result -- so the reader was handed a row whose own words disagreed
    /// with the search that found it.
    #[test]
    fn the_preview_is_cut_around_what_was_found() {
        let long = format!(
            "{} the budget looks wrong {}",
            "a word ".repeat(40),
            "b".repeat(200)
        );
        let shown = preview_around(&long, &wanted("budget"));

        assert!(shown.contains("budget"), "what was found is in it: {shown}");
        assert!(shown.starts_with('\u{2026}'), "and it says it is a middle");
        assert!(shown.ends_with('\u{2026}'), "at both ends");
        assert!(
            shown.chars().count() <= AROUND + 2,
            "one line's worth: {}",
            shown.chars().count()
        );
        // With a run-up, so it reads as a piece of a sentence.
        assert!(
            !shown.starts_with("\u{2026}budget"),
            "the match has room before it: {shown}"
        );
    }

    /// Nothing to look for -- saved, pinned, a thread's list -- is the opening
    /// of the message, which is what those lists always showed.
    #[test]
    fn with_nothing_to_look_for_it_is_the_opening() {
        let said = "the budget looks wrong";
        assert_eq!(preview_around(said, &[]), said);
        assert!(!preview_around(said, &[]).starts_with('\u{2026}'));
    }

    /// A short message is left whole rather than given ellipses it has not
    /// earned.
    #[test]
    fn a_short_message_keeps_its_own_ends() {
        let shown = preview_around("the budget looks wrong", &wanted("budget"));
        assert_eq!(shown, "the budget looks wrong");
    }

    /// The lines of a message become one line, because a row is one line.
    #[test]
    fn a_message_written_in_lines_comes_back_as_one() {
        let shown = preview_around("first line\n\nsecond line", &[]);
        assert_eq!(shown, "first line second line");
    }
}
