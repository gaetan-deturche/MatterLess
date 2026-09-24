//! The teams, down the far left.
//!
//! Its own strip rather than a row among the conversations: a team is a
//! different kind of thing from a channel, and a list holding both means two
//! things at once. The app gives it a column of its own and so does this.
//!
//! A team is drawn as its initials on a square. The server has an icon for
//! each, but a team icon is a picture behind the session token like any other,
//! and initials say which team it is without a fetch.

use matterless_paint::Run;
use matterless_ui::input::Input;
use matterless_ui::{Placed, Rect};
use matterless_widgets::{Canvas, Panel};

/// A square and the gap between two of them, from the stylesheet: a 30px
/// button with a 4px gap, in a strip padded `8px 4px`.
const TILE: f32 = 30.0;
const GAP: f32 = 4.0;
const TOP: f32 = 8.0;
/// `border-radius: 7px` on the button, 6 on the icon inside it.
const CORNER: f32 = 7.0;
const ICON: f32 = 6.0;
/// The dot in the corner that says a team has something waiting, and the
/// larger one that carries a mention count.
const PIP: f32 = 8.0;
const MENTION_PIP: f32 = 14.0;

/// What the direct-messages button answers to. Not a team, and the one
/// destination in the list that no team icon reaches.
pub const DIRECTS: &str = "directs";

/// What the unread button answers to.
pub const UNREAD: &str = "unread";
/// What the gear answers to.
pub const SETTINGS: &str = "settings";

/// What a square on the rail is.
///
/// One state rather than a flag per kind. Two booleans with both set is a
/// square nobody can draw, and each of these answers three questions at once
/// -- what picture it carries, whether there is an icon to fetch for it, and
/// whether it is somewhere to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A team: an icon to fetch, and its initials until that arrives.
    Team,
    /// Direct messages, which are not a team and so have no icon of their own.
    Directs,
    /// The conversations with something waiting, brought into view in the
    /// list.
    ///
    /// The one square here that is a thing to *do* rather than a place to be,
    /// so it is never the chosen one and carries no count: what is waiting is
    /// what it takes you to, and saying so twice on one square would be a dot
    /// beside a picture of a dot.
    Unread,
    /// What this copy of the program has been told to do.
    ///
    /// Like `Unread`, a thing to do rather than a place to be: it is never
    /// the chosen square and carries no count.
    Settings,
}

impl Kind {
    /// Whether this square belongs at the foot of the strip.
    ///
    /// Everything above the line is somewhere to go. This is about the
    /// program itself, which is not one of the places in it.
    fn at_the_foot(self) -> bool {
        matches!(self, Kind::Settings)
    }
}

/// One square on the rail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    pub id: String,
    pub name: String,
    /// What is waiting in it, which the corner shows as a dot or a count.
    pub unread: i64,
    pub mentions: i64,
    pub kind: Kind,
}

/// The teams, and which one the reader is in.
#[derive(Debug, Default)]
pub struct Rail {
    pub teams: Vec<Tile>,
    pub chosen: Option<String>,
}

impl Rail {
    /// Where each square sits.
    ///
    /// Two stacks rather than one: the places to go fill down from the top,
    /// and whatever belongs to the program rather than to a conversation
    /// fills up from the foot. The gear sat under the last team, where it read
    /// as one more place to be -- and every client that has one puts it at the
    /// bottom for that reason.
    pub fn place(&self, within: Rect) -> Vec<(&Tile, Rect)> {
        let square = |y: f32| Rect::new(within.x + (within.width - TILE) / 2.0, y, TILE, TILE);
        let feet = self
            .teams
            .iter()
            .filter(|team| team.kind.at_the_foot())
            .count();
        let mut top = within.y + TOP;
        // Where the first of the foot squares goes, so the last of them ends
        // one padding off the bottom.
        let mut foot =
            within.bottom() - TOP - TILE * feet as f32 - GAP * feet.saturating_sub(1) as f32;
        let mut placed = Vec::with_capacity(self.teams.len());
        for team in &self.teams {
            if team.kind.at_the_foot() {
                // Never above what came down from the top: on a strip too
                // short for both, the foot gives way rather than the two
                // stacks being drawn over each other.
                let y = foot.max(top);
                placed.push((team, square(y)));
                foot = y + TILE + GAP;
            } else {
                placed.push((team, square(top)));
                top += TILE + GAP;
            }
        }
        placed
    }

    /// Where the foot squares begin, for the rule drawn over them.
    fn foot_begins(&self, within: Rect) -> Option<f32> {
        self.place(within)
            .into_iter()
            .find(|(team, _)| team.kind.at_the_foot())
            .map(|(_, rect)| rect.y)
    }

    pub fn boxes(&self, within: Rect) -> Vec<Placed> {
        let mut placed = vec![Placed {
            name: "rail".to_string(),
            rect: within,
            depth: 1,
        }];
        for (team, rect) in self.place(within) {
            placed.push(Placed {
                name: format!("rail/{}", team.id),
                rect,
                depth: 2,
            });
        }
        placed
    }

    /// Which team was chosen, if one was.
    pub fn react(&self, input: &Input) -> Option<String> {
        let clicked = input.clicked()?;
        let id = clicked.strip_prefix("rail/")?;
        Some(id.to_string())
    }

    /// The pictures it wants, so the caller can fetch them the usual way.
    pub fn wants(&self) -> Vec<(String, u32, u32)> {
        self.teams
            .iter()
            // Only a team has a picture to fetch; the other two carry a
            // glyph the font already has.
            .filter(|team| team.kind == Kind::Team)
            .map(|team| (icon_key(&team.id), ICON_FETCHED, ICON_FETCHED))
            .collect()
    }

    pub fn draw(&self, into: &mut Canvas<'_>, within: Rect, input: &Input) {
        let placed = self.place(within);
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        scene.fill(
            within.x,
            within.y,
            within.width,
            within.height,
            palette.surface,
        );
        // The rule between the rail and the sidebar, which is the only thing
        // saying they are two lists rather than one long one.
        scene.fill(
            within.right() - 1.0,
            within.y,
            1.0,
            within.height,
            palette.rule_soft,
        );
        // A line over the foot, so the gear reads as a group of its own
        // rather than as the last team with a lot of air above it.
        if let Some(begins) = self.foot_begins(within) {
            scene.fill(
                within.x + (within.width - TILE) / 2.0,
                (begins - GAP * 2.0).round(),
                TILE,
                1.0,
                palette.rule_soft,
            );
        }

        for (team, rect) in placed {
            let here = self.chosen.as_deref() == Some(team.id.as_str());
            let under = input.hovered() == Some(format!("rail/{}", team.id).as_str());
            // A border rather than a fill: `.rail button.current` takes the
            // signal colour on its edge and leaves the ground behind it, and a
            // filled square would read as selected even when it is not.
            let edge = if here {
                [palette.signal[0], palette.signal[1], palette.signal[2], 255]
            } else if under {
                palette.rule
            } else {
                palette.ground
            };
            Panel::flat(rect, CORNER)
                .edge(edge)
                .fill(palette.ground)
                .draw(scene);

            // The initials go down first and the icon over them, which is what
            // `z-index: -1` on `.initial` does: a team whose icon the server
            // has none of still reads as something rather than as a blank.
            //
            // Measured and centred rather than put at a fixed offset. Nine
            // pixels in suits the two letters most teams reduce to and nothing
            // else: a team that reduces to one sat right of centre, and the
            // envelope on the direct-messages button -- which is not initials
            // at all and is a different width again -- sat further right still.
            let (label, mark) = face(team);
            let (at_x, at_y) = mark_at(fonts, rect, &label, mark);
            let glyphs = painter.run(fonts, &label, at_x, at_y, mark);
            scene.glyphs(
                glyphs,
                if here || under {
                    palette.ink
                } else {
                    palette.soft
                },
                palette.faint,
            );
            if team.kind == Kind::Team {
                scene.extend([matterless_paint::Piece::Image {
                    x: rect.x + 1.0,
                    y: rect.y + 1.0,
                    width: rect.width - 2.0,
                    height: rect.height - 2.0,
                    key: icon_key(&team.id),
                    radius: ICON,
                }]);
            }

            // What is waiting, in the corner. A count for a mention and a bare
            // dot for anything else, which is the difference between "somebody
            // asked you" and "something happened".
            if team.mentions > 0 {
                let width = MENTION_PIP.max(10.0 + team.mentions.to_string().len() as f32 * 6.0);
                scene.rounded(
                    rect.right() - width + 3.0,
                    rect.y - 3.0,
                    width,
                    MENTION_PIP,
                    [palette.flag[0], palette.flag[1], palette.flag[2], 255],
                    MENTION_PIP / 2.0,
                );
                let count = painter.run(
                    fonts,
                    &team.mentions.to_string(),
                    rect.right() - width + 7.0,
                    rect.y - 4.0,
                    Run::label(f32::MAX),
                );
                scene.glyphs(
                    count,
                    [palette.ground[0], palette.ground[1], palette.ground[2]],
                    palette.faint,
                );
            } else if team.unread > 0 {
                scene.rounded(
                    rect.right() - PIP + 3.0,
                    rect.y - 3.0,
                    PIP,
                    PIP,
                    palette.faint_fill(),
                    PIP / 2.0,
                );
            }
        }
    }
}

/// The size a team's icon is fetched at.
///
/// One size for one key: the sidebar draws the same picture smaller, and an
/// atlas holds one image per name -- so whichever asked first would decide the
/// size for both. Fetched at the larger of the two, which is this one, and
/// scaled down where it is drawn small.
pub const ICON_FETCHED: u32 = TILE as u32;

/// What a team's icon is called in the atlas.
pub fn icon_key(team_id: &str) -> String {
    format!("team/{team_id}")
}

/// Where a tile's mark goes.
///
/// Measured rather than offset by a constant. Nine pixels in suited the two
/// letters most team names reduce to and nothing else: one letter sat right of
/// centre, and the envelope on the direct-messages button is wider than either
/// and ran off the edge of its tile.
///
/// A mark is not measured at all, because there is nothing to measure: the
/// icon family draws every picture on one square grid and centres it there, so
/// the square it is set at *is* its width. Measuring it was the fault, not the
/// fix -- `Style` has no way to say "the icon family", so a private-use
/// codepoint went through ordinary fallback and came back as whatever font
/// claims that block. Segoe's icons claim the same one. The gear measured
/// 18.38 where lucide's own em is 15, which put it a pixel and a half left of
/// centre; the three marks that did reach lucide measured exactly 15, with
/// their ink centred in it to within a quarter of a pixel.
fn mark_at(fonts: &mut matterless_layout::Fonts, tile: Rect, label: &str, mark: Run) -> (f32, f32) {
    let wide = match mark.icon {
        true => mark.size,
        false => {
            matterless_layout::extent_of(
                fonts,
                label,
                f32::MAX,
                matterless_layout::Style {
                    size: mark.size,
                    line_height: mark.line_height,
                    bold: mark.bold,
                    italic: false,
                    mono: mark.mono,
                },
            )
            .width
        }
    };
    (
        (tile.x + (tile.width - wide) / 2.0).round(),
        (tile.y + (tile.height - mark.line_height) / 2.0).round(),
    )
}

/// The size a mark on a tile is drawn at.
const MARK: f32 = 15.0;

/// How the letter or mark on a tile is set.
fn mark_run() -> Run {
    Run::label(f32::MAX).bold()
}

/// What a tile shows, and what it has to be set in.
///
/// A team is a letter and the unread button is a picture, and they come out of
/// different fonts: asking the text font for a lucide codepoint draws the box
/// that means "no such glyph", and asking the icon font for a letter draws
/// nothing at all.
fn face(tile: &Tile) -> (String, Run) {
    use matterless_layout::marks;
    match tile.kind {
        // A conversation with somebody, because a direct message is who it
        // is with rather than what it is about.
        Kind::Directs => (marks::DIRECTS.to_string(), Run::mark(MARK)),
        // A message with something on it, which is what the mark this goes
        // to stands in front of.
        Kind::Unread => (marks::TO_UNREAD.to_string(), Run::mark(MARK)),
        Kind::Settings => (marks::SETTINGS.to_string(), Run::mark(MARK)),
        Kind::Team => (initials(&tile.name), mark_run()),
    }
}

/// A team name reduced to what fits on a square.
///
/// The first letter of each word, up to two: Northwind becomes S and Voyager
/// Team becomes CT, which is what every client does with a name it has no room
/// for.
fn initials(name: &str) -> String {
    let letters: String = name
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .collect();
    if letters.is_empty() {
        "?".to_string()
    } else {
        letters.to_uppercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rail() -> Rail {
        Rail {
            teams: vec![
                Tile {
                    id: "t1".into(),
                    name: "Voyager".into(),
                    unread: 0,
                    mentions: 0,
                    kind: Kind::Team,
                },
                Tile {
                    id: "t2".into(),
                    name: "Northwind".into(),
                    unread: 3,
                    mentions: 1,
                    kind: Kind::Team,
                },
            ],
            chosen: Some("t1".into()),
        }
    }

    /// Two squares, stacked, inside the strip.
    #[test]
    fn the_teams_stack_inside_the_rail() {
        let strip = Rect::new(0.0, 0.0, 48.0, 800.0);
        let rail = rail();
        let placed = rail.place(strip);
        assert_eq!(placed.len(), 2);
        assert!(placed[0].1.bottom() <= placed[1].1.y);
        for (_, rect) in &placed {
            assert!(rect.x >= strip.x && rect.right() <= strip.right());
        }
    }

    /// The gear goes to the foot, under everything that is a place to be.
    ///
    /// It was pushed on the end of the list and so drawn directly under the
    /// last team, where it read as one more team.
    #[test]
    fn the_gear_sits_at_the_foot() {
        let strip = Rect::new(0.0, 0.0, 48.0, 800.0);
        let mut rail = rail();
        rail.teams.push(Tile {
            id: SETTINGS.into(),
            name: "Settings".into(),
            unread: 0,
            mentions: 0,
            kind: Kind::Settings,
        });
        let placed = rail.place(strip);
        let gear = placed
            .iter()
            .find(|(team, _)| team.kind == Kind::Settings)
            .expect("placed")
            .1;
        assert!(
            (gear.bottom() - (strip.bottom() - TOP)).abs() < 0.5,
            "the gear sits at {} on a strip ending at {}",
            gear.bottom(),
            strip.bottom()
        );
        // And well clear of the teams, which still stack from the top.
        for (team, rect) in &placed {
            if team.kind != Kind::Settings {
                assert!(rect.bottom() <= gear.y, "{} runs into the gear", team.id);
                assert!(rect.y < strip.height / 2.0, "{} left the top", team.id);
            }
        }
    }

    /// A strip too short for both stacks gives way at the foot rather than
    /// drawing one square over another.
    #[test]
    fn a_short_strip_stacks_rather_than_overlaps() {
        let strip = Rect::new(0.0, 0.0, 48.0, 80.0);
        let mut rail = rail();
        rail.teams.push(Tile {
            id: SETTINGS.into(),
            name: "Settings".into(),
            unread: 0,
            mentions: 0,
            kind: Kind::Settings,
        });
        let placed = rail.place(strip);
        for pair in placed.windows(2) {
            assert!(
                pair[0].1.bottom() <= pair[1].1.y,
                "{} sits on {}",
                pair[1].0.id,
                pair[0].0.id
            );
        }
    }

    /// Whatever a tile carries sits in the middle of it.
    ///
    /// Every kind, because they are different widths and come out of different
    /// fonts: at the fixed offset this used to draw at, a team that reduced to
    /// one letter sat right of centre and the envelope -- wider than any pair
    /// of them -- ended past the right edge of its own tile.
    ///
    /// Asked of the *ink*, which is the second version of this test. The first
    /// asked whether the reserved box was centred, and passed while the gear
    /// sat a pixel and a half left of centre on screen -- because the box was
    /// centred and the picture inside it was not. Nothing a reader can see was
    /// being asked about.
    ///
    /// Where the mark is put comes from the grid and needs no measuring; that
    /// the ink then lands in the middle is what this checks, and it is the one
    /// thing that would notice the grid being wrong again.
    #[test]
    fn a_tile_carries_its_mark_in_the_middle() {
        let mut fonts = matterless_layout::Fonts::new();
        let mut painter = matterless_paint::Painter::new();
        let tile = Rect::new(10.0, 40.0, TILE, TILE);
        let shapes = [
            (Kind::Team, "Northwind"),
            (Kind::Team, "Voyager Team"),
            (Kind::Directs, "Direct messages"),
            (Kind::Unread, "Unread"),
            (Kind::Settings, "Settings"),
        ];
        for (kind, name) in shapes {
            let (label, run) = face(&Tile {
                id: "x".into(),
                name: name.into(),
                unread: 0,
                mentions: 0,
                kind,
            });
            let (x, y) = mark_at(&mut fonts, tile, &label, run);
            let ink = painter
                .ink_of(&mut fonts, &label, run)
                .expect("a mark with ink in it");
            // Middle against middle, rather than the two gaps against each
            // other: a gap counts the same pixel twice, so half a pixel of
            // rounding reads as a whole one out of place.
            let across = (x + ink.x + ink.width / 2.0) - (tile.x + tile.width / 2.0);
            let down = (y + ink.y + ink.height / 2.0) - (tile.y + tile.height / 2.0);
            assert!(
                across.abs() <= 1.0,
                "{name} sits {across} off the middle across"
            );
            assert!(down.abs() <= 1.0, "{name} sits {down} off the middle down");
            let (left, right) = (x + ink.x - tile.x, tile.right() - (x + ink.x + ink.width));
            assert!(
                left >= 0.0 && right >= 0.0,
                "{name} is {} wide and runs off a {TILE} tile",
                ink.width
            );
        }
    }

    /// Neither the envelope nor the unread button is a team, and neither has
    /// an icon to fetch: asking the server for one would be a request that can
    /// only 404.
    #[test]
    fn only_a_team_asks_for_a_picture() {
        let mut rail = rail();
        rail.teams.push(Tile {
            id: DIRECTS.into(),
            name: "Direct messages".into(),
            unread: 2,
            mentions: 0,
            kind: Kind::Directs,
        });
        rail.teams.insert(
            0,
            Tile {
                id: UNREAD.into(),
                name: "Unread".into(),
                unread: 0,
                mentions: 0,
                kind: Kind::Unread,
            },
        );
        let wanted = rail.wants();
        assert_eq!(wanted.len(), 2, "only the two real teams");
        assert!(
            wanted
                .iter()
                .all(|(key, _, _)| key != "team/directs" && key != "team/unread")
        );
    }

    /// Each kind of square is set in the font that has its picture. The
    /// unread button borrowed `marks::UNREAD` first, which is an envelope --
    /// beside the direct-messages button, which is also an envelope.
    #[test]
    fn each_square_is_set_in_the_font_that_has_it() {
        let team = Tile {
            id: "t1".into(),
            name: "Voyager Team".into(),
            unread: 0,
            mentions: 0,
            kind: Kind::Team,
        };
        let (label, run) = face(&team);
        assert_eq!(label, "VT");
        assert!(!run.icon, "a letter comes out of the text font");

        let directs = Tile {
            kind: Kind::Directs,
            name: "Direct messages".into(),
            ..team.clone()
        };
        let (label, run) = face(&directs);
        assert_eq!(label, matterless_layout::marks::DIRECTS);
        assert!(run.icon, "a mark comes out of the icon font");

        let unread = Tile {
            kind: Kind::Unread,
            name: "Unread".into(),
            ..team.clone()
        };
        let (label, run) = face(&unread);
        assert_eq!(label, matterless_layout::marks::TO_UNREAD);
        assert!(run.icon);
        assert_ne!(
            label,
            face(&directs).0,
            "and the two marks are not the same picture"
        );
    }

    /// A name has to become something that fits on a square, whatever it is.
    #[test]
    fn a_name_becomes_something_that_fits() {
        assert_eq!(initials("Northwind"), "N");
        assert_eq!(initials("Voyager Team"), "VT");
        assert_eq!(initials("one two three"), "OT");
        // Never empty: a blank square says nothing about which team it is, and
        // a team without a name is still a team the reader can press.
        assert_eq!(initials(""), "?");
        assert_eq!(initials("   "), "?");
    }
}
