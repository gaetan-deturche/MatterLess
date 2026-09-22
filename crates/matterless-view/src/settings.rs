//! What this copy of the program has been told to do.
//!
//! Settings, not preferences. A preference belongs to an account and follows
//! it between machines -- the server keeps those, and the sidebar reads them.
//! These belong to this install on this display, and the one in it so far is
//! about the panel the window is drawn on, which no account can have an
//! opinion about.
//!
//! Modal in the one sense that matters, as `whats_new` is: a press anywhere
//! outside shuts it, so there is no way to leave it open and forget it.

use matterless_paint::Run;
use matterless_ui::input::{Input, Key};
use matterless_ui::{Placed, Rect};
use matterless_widgets::{Canvas, Named, Panel};

pub const NAME: &str = "settings";

/// What this widget's hit boxes are called. The join and its inverse in one
/// place, so the box it registers and the press it answers cannot disagree.
fn named() -> Named {
    Named::new(NAME)
}

const WIDTH: f32 = 460.0;
const PAD: f32 = 20.0;
const CORNER: f32 = 10.0;
const DROP: f32 = 16.0;
const TITLE_SIZE: f32 = 16.0;
const LABEL_SIZE: f32 = 13.0;
const SAID_SIZE: f32 = 12.0;
/// One choice: its name, the line under it, and the room it takes.
const CHOICE_HEIGHT: f32 = 46.0;
const GAP: f32 = 14.0;
/// The mark that says which one is chosen, and the room kept for it.
const TICK: f32 = 18.0;

/// How text is drawn, which is the one thing in here so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Text {
    /// One coverage for the whole pixel.
    ///
    /// What every glyph in this window was until there was a choice, and what
    /// macOS and a good deal else ships. Right on any display, which is the
    /// case the other one cannot make.
    Smooth,
    /// A coverage per colour channel.
    ///
    /// Sharper on a display whose pixels are three stripes side by side, which
    /// is most desktop monitors: a stem landing on the red third is drawn as a
    /// third of a pixel rather than as a third of the light of a whole one.
    /// It assumes that layout, so on a rotated screen or a panel arranged
    /// another way it shows as colour along the edges of letters.
    Subpixel,
}

impl Text {
    /// What this is called in the settings table.
    pub const SETTING: &'static str = "text-antialiasing";

    pub fn stored(&self) -> &'static str {
        match self {
            Text::Smooth => "smooth",
            Text::Subpixel => "subpixel",
        }
    }

    /// Reads one back. Anything unrecognised is the default rather than an
    /// error: a settings table is written by a future version of this program
    /// as readily as by this one.
    pub fn read(said: Option<&str>) -> Self {
        match said {
            Some("smooth") => Text::Smooth,
            Some("subpixel") => Text::Subpixel,
            _ => Self::default(),
        }
    }

    fn title(&self) -> &'static str {
        match self {
            Text::Smooth => "Smooth",
            Text::Subpixel => "Sharp",
        }
    }

    fn said(&self) -> &'static str {
        match self {
            Text::Smooth => {
                "Even on every display. What this window drew before there was a choice."
            }
            Text::Subpixel => {
                "Sharper on an ordinary monitor. Can show colour along letters on a rotated one."
            }
        }
    }

    fn slug(&self) -> &'static str {
        match self {
            Text::Smooth => "text-smooth",
            Text::Subpixel => "text-subpixel",
        }
    }

    fn both() -> [Text; 2] {
        [Text::Smooth, Text::Subpixel]
    }
}

impl Default for Text {
    /// Smooth, on measurement rather than on principle.
    ///
    /// Subpixel is meant to be the sharper of the two and on this display it
    /// is not. Measured across a line of text -- the steepness of the sharpest
    /// edges, and how much of the run is neither ink nor panel:
    ///
    /// ```text
    ///   smooth                185  edges,  8.5% in transition
    ///   sharp, unfiltered     163  edges,  8.8%   -- and fringes badly
    ///   sharp, three-tap      146  edges, 11.3%
    ///   sharp, five-tap       142  edges, 12.4%
    /// ```
    ///
    /// Every way of filtering the colour down to something that does not fringe
    /// costs more edge than the subpixel resolution gives back, and a reader
    /// looking at it said it had gone blurry before any of this was measured.
    /// What subpixel buys is horizontal placement, which the eye reads as
    /// sharpness only on a panel striped the way it assumes -- so it is offered
    /// rather than assumed, and somebody who can see it helps on their own
    /// screen is one press from it.
    fn default() -> Self {
        Text::Smooth
    }
}

/// The panel, when it is up.
#[derive(Debug, Default)]
pub struct Settings {
    shown: bool,
    pub text: Text,
    /// The card and where each choice sits in it.
    placed: Option<(Rect, Vec<(Text, Rect)>)>,
}

/// What the window should do about what just happened in here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Did {
    /// Text is to be drawn the other way, and the caller has to say so to
    /// whatever holds the glyphs.
    Text(Text),
    Close,
}

impl Settings {
    pub fn open(&self) -> bool {
        self.shown
    }

    pub fn show(&mut self) {
        self.shown = true;
        self.placed = None;
    }

    pub fn hide(&mut self) {
        self.shown = false;
        self.placed = None;
    }

    fn height() -> f32 {
        PAD + TITLE_SIZE * 1.5 + GAP + CHOICE_HEIGHT * 2.0 + LABEL_SIZE * 1.5 + GAP + PAD
    }

    pub fn measure(&mut self, window: Rect) {
        if !self.shown {
            self.placed = None;
            return;
        }
        let width = WIDTH.min(window.width - PAD * 2.0).max(0.0);
        let height = Self::height();
        let card = Rect::new(
            window.x + (window.width - width) / 2.0,
            window.y + ((window.height - height) / 2.0).max(PAD),
            width,
            height,
        );
        // Under the heading for the group, one row each.
        let mut y = card.y + PAD + TITLE_SIZE * 1.5 + GAP + LABEL_SIZE * 1.5;
        let mut rows = Vec::new();
        for one in Text::both() {
            rows.push((
                one,
                Rect::new(card.x + PAD, y, width - PAD * 2.0, CHOICE_HEIGHT),
            ));
            y += CHOICE_HEIGHT;
        }
        self.placed = Some((card, rows));
    }

    fn laid(&self) -> Option<(Rect, &[(Text, Rect)])> {
        let (card, rows) = self.placed.as_ref()?;
        Some((*card, rows))
    }

    pub fn boxes(&self, window: Rect) -> Vec<Placed> {
        let Some((card, rows)) = self.laid() else {
            return Vec::new();
        };
        // The window first, so a press beside the card shuts it; the card over
        // it, so a press inside does not; then each choice.
        let mut placed = vec![
            Placed {
                name: NAME.to_string(),
                rect: window,
                depth: 60,
            },
            named().at("card", card, 61),
        ];
        for (one, rect) in rows {
            placed.push(named().at(one.slug(), *rect, 62));
        }
        placed
    }

    pub fn react(&mut self, input: &mut Input) -> Option<Did> {
        if !self.shown {
            return None;
        }
        if input.took(Key::Escape) {
            self.hide();
            return Some(Did::Close);
        }
        let pressed = named().clicked(input).map(str::to_string);
        if let Some(slug) = pressed.as_deref() {
            for one in Text::both() {
                if slug == one.slug() {
                    // Chosen even when it is already the one in use: the
                    // caller decides whether that is work, and answering
                    // nothing here would make a press on the current setting
                    // feel like a press that missed.
                    self.text = one;
                    return Some(Did::Text(one));
                }
            }
        }
        // A press on the card is somebody reading it. A press beside it is
        // somebody done.
        if input.clicked() == Some(NAME) {
            self.hide();
            return Some(Did::Close);
        }
        None
    }

    pub fn draw(&self, into: &mut Canvas<'_>, input: &Input, window: Rect) {
        let Some((card, rows)) = self.laid() else {
            return;
        };
        let rows: Vec<(Text, Rect)> = rows.to_vec();
        let chosen = self.text;
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        // Dimmed rather than hidden: this is a decision about how what is
        // behind it looks, so covering it entirely takes away what the reader
        // is deciding about.
        scene.fill(
            window.x,
            window.y,
            window.width,
            window.height,
            [0, 0, 0, 160],
        );
        Panel::floating(card, CORNER, DROP)
            .edge(palette.rule)
            .fill(palette.surface)
            .draw(scene);

        let glyphs = painter.run(
            fonts,
            "Settings",
            card.x + PAD,
            card.y + PAD,
            Run::label(card.width - PAD * 2.0).sized(TITLE_SIZE).bold(),
        );
        scene.glyphs(glyphs, palette.ink, palette.faint);

        let glyphs = painter.run(
            fonts,
            "Text",
            card.x + PAD,
            card.y + PAD + TITLE_SIZE * 1.5 + GAP,
            Run::label(card.width - PAD * 2.0).sized(LABEL_SIZE),
        );
        scene.glyphs(glyphs, palette.soft, palette.faint);

        for (one, rect) in rows {
            let under = named().under(input, one.slug());
            let picked = one == chosen;
            if under || picked {
                scene.rounded(
                    rect.x,
                    rect.y,
                    rect.width,
                    rect.height - 4.0,
                    match picked {
                        true => palette.signal_soft,
                        false => palette.hover,
                    },
                    6.0,
                );
            }
            // The tick, so which one is in use survives the row being hovered.
            if picked {
                let tick = painter.run(
                    fonts,
                    matterless_layout::marks::SAVED,
                    rect.right() - PAD - TICK,
                    rect.y + 8.0,
                    Run::mark(TICK),
                );
                scene.glyphs(tick, palette.signal, palette.signal);
            }
            let glyphs = painter.run(
                fonts,
                one.title(),
                rect.x + 10.0,
                rect.y + 5.0,
                Run::label(rect.width - TICK - PAD * 2.0).sized(LABEL_SIZE),
            );
            scene.glyphs(glyphs, palette.ink, palette.faint);
            let glyphs = painter.run(
                fonts,
                one.said(),
                rect.x + 10.0,
                rect.y + 5.0 + LABEL_SIZE * 1.4,
                Run::label(rect.width - TICK - PAD * 2.0).sized(SAID_SIZE),
            );
            scene.glyphs(glyphs, palette.faint, palette.faint);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_setting_survives_being_written_and_read() {
        for one in Text::both() {
            assert_eq!(Text::read(Some(one.stored())), one);
        }
    }

    /// A store written by a later version, or by nothing at all.
    #[test]
    fn anything_unrecognised_is_the_default() {
        assert_eq!(Text::read(None), Text::default());
        assert_eq!(Text::read(Some("")), Text::default());
        assert_eq!(Text::read(Some("lcd-v")), Text::default());
    }

    /// Every choice has a box, and they do not sit on each other.
    #[test]
    fn the_choices_tile_the_card() {
        let mut settings = Settings::default();
        settings.show();
        let window = Rect::new(0.0, 0.0, 1002.0, 792.0);
        settings.measure(window);
        let (card, rows) = settings.laid().expect("measured");
        assert_eq!(rows.len(), 2);
        let mut floor = card.y;
        for (one, rect) in rows {
            assert!(rect.y >= floor - 0.5, "{:?} sits on the row above", one);
            assert!(
                rect.bottom() <= card.bottom() - PAD + 0.5,
                "{:?} runs out of the card",
                one
            );
            floor = rect.bottom();
        }
        // And a box for each, plus the card and the window behind it.
        assert_eq!(settings.boxes(window).len(), 2 + rows.len());
    }
}
