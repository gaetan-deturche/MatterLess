//! Who is in a conversation, and whether they are there -- with what the
//! channel says about itself above them.
//!
//! An aside like the lists of messages, but of people: a face, the presence
//! dot the sidebar draws, and a name. Those who are here come first, then
//! the away, the busy and the gone, each alphabetically.

use crate::aside::{self, PADDING};
use crate::sidebar::Canvas;
use matterless_paint::Run;
use matterless_ui::input::Input;
use matterless_ui::{Placed, Rect};
use std::collections::HashMap;

pub const NAME: &str = "members";

const ROW: f32 = 36.0;
const FACE: f32 = 20.0;
const DOT: f32 = 7.0;
const RING: f32 = 1.5;
/// Between the channel's own words and the people under them.
const GAP: f32 = 12.0;
/// The room the people's heading takes above them.
const HEADING: f32 = 20.0;

/// One person in the channel.
#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    pub user_id: String,
    pub username: String,
    /// First and last name, when they gave one.
    pub full_name: String,
    pub avatar_at: i64,
}

/// What a frame's input came to.
#[derive(Debug, Clone, PartialEq)]
pub enum Did {
    Close,
    /// Somebody's card, from the row they were pressed on.
    Person {
        username: String,
        at: Rect,
    },
    /// A link in what the channel says about itself.
    Link(String),
}

#[derive(Debug, Default)]
pub struct Members {
    /// The channel shown; empty while shut.
    channel_id: String,
    channel_name: String,
    /// The name its links use, `~name`.
    handle: String,
    /// The channel's purpose and header: a title, its words, its links.
    about: Vec<(&'static str, Vec<crate::flow::Segment>, Vec<String>)>,
    /// Where those links landed when last drawn, and where each leads.
    link_boxes: std::cell::RefCell<Vec<(Rect, String)>>,
    people: Vec<Member>,
    waiting: bool,
    scroll: f32,
    bar: crate::scrollbar::Scrollbar,
    /// How tall the channel's words came out at the width last drawn: rows
    /// are placed under them, and only drawing has the fonts to measure.
    about_height: std::cell::Cell<f32>,
}

impl Members {
    pub fn open(&self) -> bool {
        !self.channel_id.is_empty()
    }

    pub fn channel(&self) -> &str {
        &self.channel_id
    }

    /// Opens on a channel, empty until its members arrive.
    pub fn show(
        &mut self,
        channel_id: &str,
        channel_name: &str,
        handle: &str,
        purpose: &str,
        header: &str,
    ) {
        self.channel_id = channel_id.to_string();
        self.channel_name = channel_name.to_string();
        self.handle = handle.to_string();
        self.say(purpose, header);
        self.people.clear();
        self.waiting = true;
        self.scroll = 0.0;
        self.about_height.set(0.0);
    }

    /// What the channel says about itself, again: it was edited.
    pub fn say(&mut self, purpose: &str, header: &str) {
        self.about = [("Purpose", purpose), ("Header", header)]
            .into_iter()
            .filter(|(_, said)| !said.trim().is_empty())
            .map(|(title, said)| {
                let (runs, links) = crate::flow::segments(said);
                (title, runs, links)
            })
            .collect();
        self.about_height.set(0.0);
        self.link_boxes.borrow_mut().clear();
    }

    /// The members, when they are for the channel still shown.
    pub fn fill(&mut self, channel_id: &str, people: Vec<Member>) {
        if channel_id != self.channel_id {
            return;
        }
        self.people = people;
        self.waiting = false;
    }

    pub fn hide(&mut self) {
        self.channel_id.clear();
        self.people.clear();
        self.about.clear();
        self.waiting = false;
        self.scroll = 0.0;
    }

    /// Here first, then away, busy and gone; by name within each.
    fn in_order<'a>(&'a self, presence: &HashMap<String, String>) -> Vec<&'a Member> {
        let rank = |member: &Member| match presence.get(&member.user_id).map(String::as_str) {
            Some("online") => 0,
            Some("away") => 1,
            Some("dnd" | "ooo") => 2,
            _ => 3,
        };
        let mut people: Vec<&Member> = self.people.iter().collect();
        people.sort_by_cached_key(|member| (rank(member), member.username.to_lowercase()));
        people
    }

    /// The faces the rows draw, for whoever collects the frame's pictures.
    pub fn wants(&self) -> Vec<(String, u32, u32)> {
        self.people
            .iter()
            .map(|member| {
                (
                    crate::stream::avatar_key(&member.user_id, member.avatar_at),
                    crate::stream::AVATAR as u32,
                    crate::stream::AVATAR as u32,
                )
            })
            .collect()
    }

    /// Everybody listed, so their presence is asked for.
    pub fn who_is_here(&self) -> Vec<String> {
        self.people
            .iter()
            .map(|member| member.user_id.clone())
            .collect()
    }

    fn title(&self) -> &'static str {
        "Channel info"
    }

    /// What the list of people is headed with.
    fn heading(&self) -> String {
        match self.waiting {
            true => "Members".to_string(),
            false => format!("Members \u{b7} {}", self.people.len()),
        }
    }

    /// Where the rows begin, below the channel's words and their heading.
    fn rows_top(&self, body: Rect) -> f32 {
        let about = self.about_height.get();
        body.y + PADDING + if about > 0.0 { about + GAP } else { 0.0 } + HEADING
    }

    fn reach(&self, body: Rect) -> f32 {
        let wanted = self.rows_top(body) - body.y + self.people.len() as f32 * ROW + PADDING;
        (wanted - body.height).max(0.0)
    }

    fn row_rect(&self, body: Rect, at: usize) -> Rect {
        Rect::new(
            body.x,
            self.rows_top(body) + at as f32 * ROW - self.scroll,
            body.width,
            ROW,
        )
    }

    pub fn boxes(&self, pane: Rect) -> Vec<Placed> {
        if !self.open() {
            return Vec::new();
        }
        let body = aside::body(pane);
        let mut placed = vec![
            Placed {
                name: NAME.to_string(),
                rect: pane,
                depth: 8,
            },
            aside::close_box(pane, NAME),
        ];
        placed.extend(self.bar.boxes(NAME, body, self.reach(body)));
        for (at, (rect, _)) in self.link_boxes.borrow().iter().enumerate() {
            if rect.bottom() <= body.y || rect.y >= body.bottom() {
                continue;
            }
            placed.push(Placed {
                name: format!("{NAME}/link/{at}"),
                rect: *rect,
                depth: 10,
            });
        }
        for at in 0..self.people.len() {
            let row = self.row_rect(body, at);
            if row.bottom() <= body.y || row.y >= body.bottom() {
                continue;
            }
            placed.push(Placed {
                name: format!("{NAME}/{at}"),
                rect: row,
                depth: 9,
            });
        }
        placed
    }

    pub fn react(
        &mut self,
        input: &Input,
        placed: &[Placed],
        pane: Rect,
        presence: &HashMap<String, String>,
    ) -> Option<Did> {
        if !self.open() {
            return None;
        }
        let body = aside::body(pane);
        let reach = self.reach(body);
        if let Some(scroll) = self.bar.react(NAME, input, body, self.scroll, reach) {
            self.scroll = scroll.clamp(0.0, reach);
            return None;
        }
        if let Some((_, y)) = input.wheel_over(placed, |name| name == NAME) {
            self.scroll = (self.scroll - y).clamp(0.0, reach);
        }
        let clicked = input.clicked()?;
        if clicked == format!("{NAME}/close") {
            return Some(Did::Close);
        }
        if let Some(at) = clicked.strip_prefix(&format!("{NAME}/link/")) {
            let at: usize = at.parse().ok()?;
            return self
                .link_boxes
                .borrow()
                .get(at)
                .map(|(_, href)| Did::Link(href.clone()));
        }
        let at: usize = clicked.strip_prefix(&format!("{NAME}/"))?.parse().ok()?;
        let member = self.in_order(presence).get(at).copied()?.clone();
        Some(Did::Person {
            username: member.username,
            at: self.row_rect(body, at),
        })
    }

    pub fn draw(
        &self,
        into: &mut Canvas<'_>,
        input: &Input,
        pane: Rect,
        presence: &HashMap<String, String>,
    ) {
        if !self.open() {
            return;
        }
        let head = aside::header(pane);
        aside::ground(into, pane);
        aside::draw_close(into, pane);
        let heading = into.painter.run(
            into.fonts,
            self.title(),
            head.x + PADDING,
            head.y + (aside::HEADER - 18.0) / 2.0,
            Run::label(f32::MAX).bold(),
        );
        into.scene
            .glyphs(heading, into.palette.ink, into.palette.faint);

        let body = aside::body(pane);
        let room = body.width - PADDING * 2.0 - crate::scrollbar::TRACK;
        into.scene.clip_to(body.x, body.y, body.width, body.height);

        // The channel's own words first, wrapped to the pane, with its links
        // followable. Underlined whole when any word of one is under the
        // pointer, which is how the last frame placed them.
        let mut y = body.y + PADDING - self.scroll;
        let words = crate::listing::label(false);
        let hovered = input
            .hovered()
            .and_then(|name| name.strip_prefix(&format!("{NAME}/link/")))
            .and_then(|at| at.parse::<usize>().ok())
            .and_then(|at| {
                self.link_boxes
                    .borrow()
                    .get(at)
                    .map(|(_, href)| href.clone())
            });
        let mut links = Vec::new();
        // Centred, all of it: the channel's name over what it says about
        // itself. Each line of words is centred on its own.
        let left = body.x + PADDING;
        let centred = |fonts: &mut matterless_layout::Fonts, text: &str, style| {
            let wide = matterless_layout::extent_of(fonts, text, f32::MAX, style).width;
            left + ((room - wide) / 2.0).max(0.0)
        };
        let big = matterless_layout::Style {
            size: 17.0,
            line_height: 24.0,
            bold: true,
            italic: false,
            mono: false,
        };
        let name = matterless_layout::elided(into.fonts, &self.channel_name, room, big);
        let x = centred(into.fonts, &name, big);
        let glyphs = into.painter.run(
            into.fonts,
            &name,
            x,
            y,
            Run::label(f32::MAX).sized(17.0).bold(),
        );
        into.scene
            .glyphs(glyphs, into.palette.ink, into.palette.faint);
        y += 32.0;
        let small_bold = matterless_layout::Style {
            size: 11.0,
            line_height: 15.4,
            bold: true,
            italic: false,
            mono: false,
        };
        for (title, runs, hrefs) in &self.about {
            let x = centred(into.fonts, title, small_bold);
            let glyphs = into.painter.run(
                into.fonts,
                title,
                x,
                y,
                Run::label(f32::MAX).sized(11.0).bold(),
            );
            into.scene
                .glyphs(glyphs, into.palette.faint, into.palette.faint);
            y += 16.0;
            let (laid, tall, _) = crate::flow::lay(into.fonts, runs, hrefs, room, words, false);
            // How wide each line came out, to centre it.
            let mut widths: Vec<(f32, f32)> = Vec::new();
            for word in &laid {
                let end = word.x + word.width;
                match widths.iter_mut().find(|(line, _)| *line == word.y) {
                    Some((_, wide)) => *wide = wide.max(end),
                    None => widths.push((word.y, end)),
                }
            }
            for word in &laid {
                let wide = widths
                    .iter()
                    .find(|(line, _)| *line == word.y)
                    .map_or(room, |(_, wide)| *wide);
                let shift = ((room - wide) / 2.0).max(0.0);
                let (x, top) = (left + shift + word.x, y + word.y);
                let href = word.link.and_then(|at| hrefs.get(at));
                let ink = match href {
                    Some(_) => into.palette.signal,
                    None => into.palette.soft,
                };
                let glyphs = into
                    .painter
                    .run(into.fonts, &word.text, x, top, Run::label(f32::MAX));
                into.scene.glyphs(glyphs, ink, into.palette.faint);
                if let Some(href) = href {
                    if hovered.as_ref() == Some(href) {
                        let [red, green, blue] = into.palette.signal;
                        into.scene
                            .fill(x, top + 16.0, word.width, 1.0, [red, green, blue, 255]);
                    }
                    links.push((Rect::new(x, top, word.width, 18.0), href.clone()));
                }
            }
            y += tall + 8.0;
        }
        // How links name it, and the id the server knows it by.
        for said in [
            format!("Channel handle: {}", self.handle),
            format!("ID: {}", self.channel_id),
        ] {
            let x = centred(
                into.fonts,
                &said,
                matterless_layout::Style {
                    size: 11.5,
                    line_height: 16.1,
                    bold: false,
                    italic: false,
                    mono: false,
                },
            );
            let glyphs =
                into.painter
                    .run(into.fonts, &said, x, y, Run::label(f32::MAX).sized(11.5));
            into.scene
                .glyphs(glyphs, into.palette.faint, into.palette.faint);
            y += 18.0;
        }
        *self.link_boxes.borrow_mut() = links;
        self.about_height
            .set((y + self.scroll - body.y - PADDING).max(0.0));

        let glyphs = into.painter.run(
            into.fonts,
            &self.heading(),
            body.x + PADDING,
            self.rows_top(body) - HEADING - self.scroll,
            Run::label(room).sized(11.0).bold(),
        );
        into.scene
            .glyphs(glyphs, into.palette.faint, into.palette.faint);
        if self.people.is_empty() {
            let said = match self.waiting {
                true => "still looking",
                false => "nobody here",
            };
            let glyphs = into.painter.run(
                into.fonts,
                said,
                body.x + PADDING,
                self.rows_top(body) - self.scroll,
                Run::label(f32::MAX),
            );
            into.scene
                .glyphs(glyphs, into.palette.faint, into.palette.faint);
            return;
        }

        for (at, member) in self.in_order(presence).into_iter().enumerate() {
            let row = self.row_rect(body, at);
            if row.bottom() <= body.y || row.y >= body.bottom() {
                continue;
            }
            if input.hovered() == Some(format!("{NAME}/{at}").as_str()) {
                into.scene
                    .fill(row.x, row.y, row.width, row.height, into.palette.ground);
            }
            let face = Rect::new(row.x + PADDING, row.y + (ROW - FACE) / 2.0, FACE, FACE);
            into.scene.extend([matterless_paint::Piece::Image {
                x: face.x,
                y: face.y,
                width: FACE,
                height: FACE,
                key: crate::stream::avatar_key(&member.user_id, member.avatar_at),
                radius: FACE / 2.0,
            }]);
            // After the face, or the picture lands on top of it.
            if let Some(lit) = presence
                .get(&member.user_id)
                .and_then(|status| crate::sidebar::dot(status, into.palette))
            {
                let dot = (face.x + FACE - DOT + 1.0, face.y + FACE - DOT + 1.0);
                into.scene.rounded(
                    dot.0 - RING,
                    dot.1 - RING,
                    DOT + RING * 2.0,
                    DOT + RING * 2.0,
                    into.palette.surface,
                    (DOT + RING * 2.0) / 2.0,
                );
                into.scene.rounded(dot.0, dot.1, DOT, DOT, lit, DOT / 2.0);
            }
            let left = face.right() + 10.0;
            let wide = (row.right() - crate::scrollbar::TRACK - left).max(10.0);
            let name = matterless_layout::elided(
                into.fonts,
                &member.username,
                wide,
                crate::listing::label(true),
            );
            let glyphs = into.painter.run(
                into.fonts,
                &name,
                left,
                row.y + (ROW - 18.0) / 2.0,
                Run::label(f32::MAX).bold(),
            );
            let used = matterless_layout::extent_of(
                into.fonts,
                &name,
                f32::MAX,
                crate::listing::label(true),
            )
            .width;
            into.scene
                .glyphs(glyphs, into.palette.ink, into.palette.faint);
            // Their real name beside it, faint, in what room is left.
            let rest = wide - used - 8.0;
            if !member.full_name.is_empty() && rest > 30.0 {
                let full = matterless_layout::elided(into.fonts, &member.full_name, rest, words);
                let glyphs = into.painter.run(
                    into.fonts,
                    &full,
                    left + used + 8.0,
                    row.y + (ROW - 18.0) / 2.0,
                    Run::label(f32::MAX),
                );
                into.scene
                    .glyphs(glyphs, into.palette.faint, into.palette.faint);
            }
        }
        let reach = self.reach(body);
        self.bar.draw(into, NAME, input, body, self.scroll, reach);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: &str, name: &str) -> Member {
        Member {
            user_id: id.into(),
            username: name.into(),
            full_name: String::new(),
            avatar_at: 0,
        }
    }

    #[test]
    fn here_first_then_away_busy_and_gone() {
        let mut members = Members::default();
        members.show("c1", "dev", "~dev", "", "");
        members.fill(
            "c1",
            vec![
                member("u1", "zoe"),
                member("u2", "bob"),
                member("u3", "amy"),
                member("u4", "carl"),
                member("u5", "dan"),
            ],
        );
        let presence: HashMap<String, String> = [
            ("u1", "online"),
            ("u2", "offline"),
            ("u3", "away"),
            ("u4", "dnd"),
            ("u5", "online"),
        ]
        .into_iter()
        .map(|(id, status)| (id.to_string(), status.to_string()))
        .collect();
        let order: Vec<&str> = members
            .in_order(&presence)
            .into_iter()
            .map(|member| member.username.as_str())
            .collect();
        assert_eq!(order, ["dan", "zoe", "amy", "carl", "bob"]);
    }

    #[test]
    fn members_for_another_channel_are_not_shown() {
        let mut members = Members::default();
        members.show("c1", "dev", "~dev", "Ship it", "");
        members.fill("c2", vec![member("u1", "zoe")]);
        assert!(members.people.is_empty() && members.waiting);
        assert_eq!(members.about.len(), 1);
        assert_eq!(members.about[0].0, "Purpose");
    }
}
