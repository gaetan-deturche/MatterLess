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
    /// Where the reader stopped reading in the conversation they have open.
    ///
    /// The one square here that is a thing to *do* rather than a place to be,
    /// so it is never the chosen one and carries no count: what is waiting is
    /// what it takes you to, and saying so twice on one square would be a dot
    /// beside a picture of a dot.
    Unread,
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
    pub fn place(&self, within: Rect) -> Vec<(&Tile, Rect)> {
        self.teams
            .iter()
            .enumerate()
            .map(|(at, team)| {
                (
                    team,
                    Rect::new(
                        within.x + (within.width - TILE) / 2.0,
                        within.y + TOP + at as f32 * (TILE + GAP),
                        TILE,
                        TILE,
                    ),
                )
            })
            .collect()
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
            let wide = matterless_layout::extent_of(
                fonts,
                &label,
                f32::MAX,
                matterless_layout::Style {
                    size: mark.size,
                    line_height: mark.line_height,
                    bold: mark.bold,
                    italic: false,
                    mono: mark.mono,
                },
            )
            .width;
            let (at_x, at_y) = mark_at(rect, wide, mark.line_height);
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

/// Where a tile's mark goes, from how wide it actually is.
///
/// Measured rather than offset by a constant. Nine pixels in suited the two
/// letters most team names reduce to and nothing else: one letter sat right of
/// centre, and the envelope on the direct-messages button is wider than either
/// and ran off the edge of its tile.
fn mark_at(tile: Rect, wide: f32, tall: f32) -> (f32, f32) {
    (
        (tile.x + (tile.width - wide) / 2.0).round(),
        (tile.y + (tile.height - tall) / 2.0).round(),
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

    /// Whatever a tile carries sits in the middle of it.
    ///
    /// Every kind, because they are different widths and come out of different
    /// fonts: at the fixed offset this used to draw at, a team that reduced to
    /// one letter sat right of centre and the envelope -- wider than any pair
    /// of them -- ended past the right edge of its own tile.
    #[test]
    fn a_tile_carries_its_mark_in_the_middle() {
        let mut fonts = matterless_layout::Fonts::new();
        let tile = Rect::new(10.0, 40.0, TILE, TILE);
        let shapes = [
            (Kind::Team, "Northwind"),
            (Kind::Team, "Voyager Team"),
            (Kind::Directs, "Direct messages"),
            (Kind::Unread, "Unread"),
        ];
        for (kind, name) in shapes {
            let (label, run) = face(&Tile {
                id: "x".into(),
                name: name.into(),
                unread: 0,
                mentions: 0,
                kind,
            });
            let wide = matterless_layout::extent_of(
                &mut fonts,
                &label,
                f32::MAX,
                matterless_layout::Style {
                    size: run.size,
                    line_height: run.line_height,
                    bold: run.bold,
                    italic: false,
                    mono: run.mono,
                },
            )
            .width;
            let (x, y) = mark_at(tile, wide, run.line_height);
            let (left, right) = (x - tile.x, tile.right() - (x + wide));
            assert!(
                (left - right).abs() <= 1.0,
                "{label:?} sits {left} from the left and {right} from the right"
            );
            assert!(
                left >= 0.0 && right >= 0.0,
                "{label:?} is {wide} wide and runs off a {TILE} tile"
            );
            let (top, bottom) = (y - tile.y, tile.bottom() - (y + run.line_height));
            assert!(
                (top - bottom).abs() <= 1.0,
                "{label:?} sits {top} from the top and {bottom} from the bottom"
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
        assert_eq!(label, "CT");
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
        assert_eq!(initials("Northwind"), "S");
        assert_eq!(initials("Voyager Team"), "CT");
        assert_eq!(initials("one two three"), "OT");
        // Never empty: a blank square says nothing about which team it is, and
        // a team without a name is still a team the reader can press.
        assert_eq!(initials(""), "?");
        assert_eq!(initials("   "), "?");
    }
}
