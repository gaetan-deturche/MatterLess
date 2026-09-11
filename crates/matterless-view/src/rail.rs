//! The teams, down the far left.
//!
//! Its own strip rather than a row among the conversations: a team is a
//! different kind of thing from a channel, and a list holding both means two
//! things at once. The app gives it a column of its own and so does this.
//!
//! A team is drawn as its initials on a square. The server has an icon for
//! each, but a team icon is a picture behind the session token like any other,
//! and initials say which team it is without a fetch.

use crate::sidebar::Canvas;
use matterless_paint::Run;
use matterless_ui::input::Input;
use matterless_ui::{Placed, Rect};

/// A square, and the gap between two of them.
const TILE: f32 = 34.0;
const GAP: f32 = 8.0;
const TOP: f32 = 10.0;
/// How far the corners are cut, which the stylesheet gives as 8px.
const CORNER: f32 = 8.0;

/// One team on the rail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    pub id: String,
    pub name: String,
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
            palette.ground,
        );
        for (team, rect) in placed {
            let here = self.chosen.as_deref() == Some(team.id.as_str());
            let under = input.hovered() == Some(format!("rail/{}", team.id).as_str());
            scene.rounded(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                if here || under {
                    palette.raised
                } else {
                    palette.surface
                },
                CORNER,
            );
            // The team the reader is in wears a bar down its left, which is
            // what says "here" without a second colour for the square itself.
            if here {
                scene.rounded(
                    within.x + 1.0,
                    rect.y + 4.0,
                    3.0,
                    rect.height - 8.0,
                    [palette.signal[0], palette.signal[1], palette.signal[2], 255],
                    1.5,
                );
            }
            let glyphs = painter.run(
                fonts,
                &initials(&team.name),
                rect.x + 9.0,
                rect.y + 8.0,
                Run::label(f32::MAX).bold(),
            );
            scene.glyphs(
                glyphs,
                if here { palette.ink } else { palette.soft },
                palette.faint,
            );
        }
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
                },
                Tile {
                    id: "t2".into(),
                    name: "Northwind".into(),
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
