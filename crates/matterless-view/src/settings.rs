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

use matterless_layout::{Fonts, Style};
use matterless_paint::Run;
use matterless_ui::input::{Input, Key};
use matterless_ui::{Placed, Rect};
use matterless_widgets::{Button, Canvas, Laid, Named, Panel, Row};

pub const NAME: &str = "settings";

/// What this widget's hit boxes are called. The join and its inverse in one
/// place, so the box it registers and the press it answers cannot disagree.
fn named() -> Named {
    Named::new(NAME)
}

const WIDTH: f32 = 440.0;
const PAD: f32 = 20.0;
const CORNER: f32 = 10.0;
const DROP: f32 = 16.0;
const TITLE_SIZE: f32 = 15.0;
/// The heading over a group of choices.
const GROUP_SIZE: f32 = 11.5;
/// What one choice is called, and the line under it.
const LABEL_SIZE: f32 = 13.0;
const SAID_SIZE: f32 = 12.0;
const GAP: f32 = 14.0;
/// Between two choices in the same group.
const ROW_GAP: f32 = 4.0;
/// Inside one choice, around what it says.
const ROW_PAD_X: f32 = 10.0;
const ROW_PAD_Y: f32 = 9.0;
const ROW_CORNER: f32 = 7.0;
/// The mark saying which one is chosen, and the column kept for it.
const DOT: f32 = 16.0;
const DOT_COLUMN: f32 = DOT + 12.0;
/// How tall the row at the foot is. What the button in it is set in comes
/// from `matterless_widgets::Metrics`, as everywhere else.
const BUTTON_HEIGHT: f32 = 26.0;

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

/// How much disk the pictures opened in the viewer may keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kept {
    Quarter,
    #[default]
    Half,
    One,
    Two,
    /// Everything, for good: stored as zero.
    Unlimited,
}

impl Kept {
    /// What this is called in the settings table.
    pub const SETTING: &'static str = "looked-pictures-bytes";

    pub fn bytes(&self) -> u64 {
        const MB: u64 = 1024 * 1024;
        match self {
            Kept::Quarter => 256 * MB,
            Kept::Half => 512 * MB,
            Kept::One => 1024 * MB,
            Kept::Two => 2048 * MB,
            Kept::Unlimited => 0,
        }
    }

    pub fn stored(&self) -> String {
        self.bytes().to_string()
    }

    /// Reads one back. A number this build does not offer is the default: the
    /// buttons show one of these, and a size none of them names would show
    /// none chosen.
    pub fn read(said: Option<&str>) -> Self {
        let bytes = said.and_then(|said| said.trim().parse::<u64>().ok());
        Self::all()
            .into_iter()
            .find(|one| Some(one.bytes()) == bytes)
            .unwrap_or_default()
    }

    fn title(&self) -> &'static str {
        match self {
            Kept::Quarter => "256 MB",
            Kept::Half => "512 MB",
            Kept::One => "1 GB",
            Kept::Two => "2 GB",
            Kept::Unlimited => "No limit",
        }
    }

    fn slug(&self) -> &'static str {
        match self {
            Kept::Quarter => "kept-256",
            Kept::Half => "kept-512",
            Kept::One => "kept-1024",
            Kept::Two => "kept-2048",
            Kept::Unlimited => "kept-all",
        }
    }

    fn all() -> [Kept; 5] {
        [
            Kept::Quarter,
            Kept::Half,
            Kept::One,
            Kept::Two,
            Kept::Unlimited,
        ]
    }
}

/// One choice, measured: where it sits, and where the two lines in it do.
#[derive(Debug, Clone, Copy)]
struct Choice {
    which: Text,
    rect: Rect,
    /// Where the name goes, and where the line under it does.
    title: Rect,
    said: Rect,
}

/// The card and everything in it, measured together.
#[derive(Debug, Clone)]
struct Card {
    rect: Rect,
    /// The heading over the group of choices.
    group: Rect,
    choices: Vec<Choice>,
    /// The heading over how much opened pictures keep, and its buttons.
    kept: Rect,
    sizes: Laid,
    /// The heading over the build this is, and the line under it naming it.
    about: Rect,
    build: Rect,
    /// What the last look for a newer build came back with, when there is
    /// something to say.
    said: Option<Rect>,
    /// The button that goes and looks, and the one that shuts the panel.
    look: Laid,
    foot: Laid,
}

/// The panel, when it is up.
#[derive(Debug, Default)]
pub struct Settings {
    shown: bool,
    pub text: Text,
    pub kept: Kept,
    /// Whether a look for a newer build is out, so it is not asked for twice
    /// and the button can say what it is doing.
    looking: bool,
    /// What that look came back with, in the reader's words rather than the
    /// program's.
    said: String,
    placed: Option<Card>,
}

/// What the window should do about what just happened in here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Did {
    /// Text is to be drawn the other way, and the caller has to say so to
    /// whatever holds the glyphs.
    Text(Text),
    /// Opened pictures may keep this much on disk from now on.
    Kept(Kept),
    /// Go and see whether there is a newer build.
    ///
    /// Asked for rather than done here: this widget knows where its buttons
    /// are and nothing about the network, which is the window's business.
    Look,
    Close,
}

/// How a run of this panel's text is set, for measuring it.
///
/// The same arithmetic `Run::sized` does, because a height measured against
/// one line spacing and drawn at another is a description that runs into the
/// choice below it -- which is what a fixed row height did here.
fn style(size: f32) -> Style {
    Style {
        size,
        line_height: size * 1.4,
        bold: false,
        italic: false,
        mono: false,
    }
}

fn tall(fonts: &mut Fonts, text: &str, wrap: f32, size: f32) -> f32 {
    matterless_layout::extent_of(fonts, text, wrap, style(size)).height
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

    pub fn measure(&mut self, fonts: &mut Fonts, window: Rect) {
        self.placed = self.shown.then(|| self.place(fonts, window));
    }

    /// The card, grown to whatever its own words come out as.
    ///
    /// Every height here is measured. A description is two lines on this
    /// window and one on a wider one, and a constant tall enough for the first
    /// is a gap on the second -- or, as it was, a constant tall enough for
    /// neither, with the second line of one choice printed over the name of
    /// the next.
    fn place(&self, fonts: &mut Fonts, window: Rect) -> Card {
        let width = WIDTH.min(window.width - PAD * 2.0).max(0.0);
        let inner = (width - PAD * 2.0).max(0.0);
        let wrap = (inner - ROW_PAD_X * 2.0 - DOT_COLUMN).max(0.0);

        let head = tall(fonts, "Settings", inner, TITLE_SIZE);
        let heading = tall(fonts, "Text", inner, GROUP_SIZE);

        // Each choice is sized before any of them is placed: the card's height
        // is the sum of them and cannot be known before they are.
        let sizes: Vec<(Text, f32, f32)> = Text::both()
            .into_iter()
            .map(|which| {
                (
                    which,
                    tall(fonts, which.title(), wrap, LABEL_SIZE),
                    tall(fonts, which.said(), wrap, SAID_SIZE),
                )
            })
            .collect();
        let rows: f32 = sizes
            .iter()
            .map(|(_, title, said)| ROW_PAD_Y * 2.0 + title + said)
            .sum::<f32>()
            + ROW_GAP * (sizes.len().saturating_sub(1) as f32);

        let kept = tall(fonts, KEPT_HEADING, inner, GROUP_SIZE);

        // The build this is, and what the last look for a newer one said. The
        // line is only measured into the card when there is something on it:
        // an empty row of air under a button reads as something missing.
        let about = tall(fonts, "About", inner, GROUP_SIZE);
        let build = tall(fonts, &self.build(), inner, LABEL_SIZE);
        let band = BUTTON_HEIGHT.max(build);
        let said = match self.said.is_empty() {
            true => 0.0,
            false => ROW_GAP + tall(fonts, &self.said, inner, SAID_SIZE),
        };

        let height = PAD
            + head
            + GAP
            + heading
            + ROW_GAP
            + rows
            + GAP
            + kept
            + ROW_GAP
            + BUTTON_HEIGHT
            + GAP
            + about
            + ROW_GAP
            + band
            + said
            + GAP
            + BUTTON_HEIGHT
            + PAD;
        let rect = Rect::new(
            window.x + (window.width - width) / 2.0,
            window.y + ((window.height - height) / 2.0).max(PAD),
            width,
            height,
        );

        let group = Rect::new(rect.x + PAD, rect.y + PAD + head + GAP, inner, heading);
        let mut y = group.bottom() + ROW_GAP;
        let mut choices = Vec::with_capacity(sizes.len());
        for (which, title, said) in sizes {
            let row = Rect::new(rect.x + PAD, y, inner, ROW_PAD_Y * 2.0 + title + said);
            let text_x = row.x + ROW_PAD_X + DOT_COLUMN;
            choices.push(Choice {
                which,
                rect: row,
                title: Rect::new(text_x, row.y + ROW_PAD_Y, wrap, title),
                said: Rect::new(text_x, row.y + ROW_PAD_Y + title, wrap, said),
            });
            y = row.bottom() + ROW_GAP;
        }

        let kept = Rect::new(rect.x + PAD, y - ROW_GAP + GAP, inner, kept);
        let chosen = self.kept;
        let sizes = Kept::all()
            .into_iter()
            .fold(
                Row::new(
                    NAME,
                    Rect::new(rect.x, kept.bottom() + ROW_GAP, rect.width, BUTTON_HEIGHT),
                )
                .against(matterless_widgets::Against::Left)
                .pad(PAD)
                .depth(63),
                |row, one| {
                    row.button(match one == chosen {
                        true => Button::primary(one.slug(), one.title()),
                        false => Button::plain(one.slug(), one.title()),
                    })
                },
            )
            .measure(fonts);
        let y = kept.bottom() + ROW_GAP + BUTTON_HEIGHT + ROW_GAP;

        let about = Rect::new(rect.x + PAD, y - ROW_GAP + GAP, inner, about);
        let band = Rect::new(rect.x, about.bottom() + ROW_GAP, rect.width, band);
        // The name of the build on the left, the button that goes looking on
        // the right, on one line: they are the same subject, and a button
        // under the sentence it belongs to reads as a second thing.
        let look = Row::new(NAME, band)
            .pad(PAD)
            .depth(63)
            .enabled(!self.looking)
            .button(Button::plain("look", self.looking_label()))
            .measure(fonts);
        let build = Rect::new(
            band.x + PAD,
            band.y + (band.height - build) / 2.0,
            (look.spare() - PAD).max(0.0),
            build,
        );
        let said = (!self.said.is_empty()).then(|| {
            Rect::new(
                rect.x + PAD,
                band.bottom() + ROW_GAP,
                inner,
                tall(fonts, &self.said, inner, SAID_SIZE),
            )
        });

        let foot = Row::new(
            NAME,
            Rect::new(
                rect.x,
                rect.bottom() - PAD - BUTTON_HEIGHT,
                rect.width,
                BUTTON_HEIGHT,
            ),
        )
        .pad(PAD)
        .depth(63)
        .button(Button::primary("done", "Done"))
        .measure(fonts);

        Card {
            rect,
            group,
            choices,
            kept,
            sizes,
            about,
            build,
            said,
            look,
            foot,
        }
    }

    /// Which build this is, which is the question the button beside it
    /// answers for.
    fn build(&self) -> String {
        format!("MatterLess {}", crate::update::running())
    }

    fn looking_label(&self) -> &'static str {
        match self.looking {
            true => "Looking...",
            false => "Check for updates",
        }
    }

    /// A look has gone out: the button is spent until it comes back.
    ///
    /// What was measured is kept, though it is now a card of the wrong height.
    /// Dropping it is what a widget wants to do -- the placement is stale the
    /// moment the words change -- but nothing measures a panel except the
    /// window, and only when something is pressed. A dropped placement is a
    /// card that is not drawn at all until the next press, which is the panel
    /// blinking out under the hand that pressed it. A card one frame out of
    /// date is a card nobody can see is out of date.
    pub fn looking(&mut self) {
        self.looking = true;
        self.said = "Asking for the newest build...".to_string();
    }

    /// And it came back, with whatever there is to say about it.
    ///
    /// Said here rather than only on the strip along the top, because this is
    /// where it was asked for -- a button that answers somewhere else is a
    /// button that did nothing.
    pub fn looked(&mut self, said: impl Into<String>) {
        self.looking = false;
        self.said = said.into();
    }

    fn laid(&self) -> Option<&Card> {
        self.placed.as_ref()
    }

    pub fn boxes(&self, window: Rect) -> Vec<Placed> {
        let Some(card) = self.laid() else {
            return Vec::new();
        };
        // The window first, so a press beside the card shuts it; the card over
        // it, so a press inside does not; then each choice, then the foot.
        let mut placed = vec![
            Placed {
                name: NAME.to_string(),
                rect: window,
                depth: 60,
            },
            named().at("card", card.rect, 61),
        ];
        for choice in &card.choices {
            placed.push(named().at(choice.which.slug(), choice.rect, 62));
        }
        placed.extend(card.sizes.boxes());
        placed.extend(card.look.boxes());
        placed.extend(card.foot.boxes());
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
        if self
            .laid()
            .and_then(|card| card.foot.clicked(input))
            .is_some()
        {
            self.hide();
            return Some(Did::Close);
        }
        if self
            .laid()
            .and_then(|card| card.look.clicked(input))
            .is_some()
        {
            // The panel says it is looking before anything has been asked:
            // the answer is a round trip away, and a button that sits there
            // unchanged reads as a button that missed.
            self.looking();
            return Some(Did::Look);
        }
        if let Some(slug) = self.laid().and_then(|card| card.sizes.clicked(input))
            && let Some(one) = Kept::all().into_iter().find(|one| one.slug() == slug)
        {
            self.kept = one;
            // Measured again, so the chosen button is drawn as chosen.
            self.placed = None;
            return Some(Did::Kept(one));
        }
        let clicked = input.clicked()?.to_string();
        if let Some(slug) = named().slug(&clicked) {
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
        // A press on the card is somebody reading it. A press on the window
        // behind it is somebody done.
        //
        // By that exact name, not "anything this widget cannot name": the
        // press that opens the panel is the gear's, and the frame it opens on
        // hands that same press straight back to here. Anything looser shut
        // the panel on the click that asked for it, so it never appeared at
        // all.
        if clicked == NAME {
            self.hide();
            return Some(Did::Close);
        }
        None
    }

    pub fn draw(&self, into: &mut Canvas<'_>, input: &Input, window: Rect) {
        let Some(card) = self.laid() else {
            return;
        };
        let card = card.clone();
        let chosen = self.text;
        {
            let Canvas {
                scene,
                painter,
                fonts,
                palette,
            } = into;
            // Dimmed rather than hidden: this is a decision about how what is
            // behind it looks, so covering it entirely takes away what the
            // reader is deciding about.
            scene.fill(
                window.x,
                window.y,
                window.width,
                window.height,
                [0, 0, 0, 160],
            );
            Panel::floating(card.rect, CORNER, DROP)
                .edge(palette.rule)
                .fill(palette.surface)
                .draw(scene);

            let glyphs = painter.run(
                fonts,
                "Settings",
                card.rect.x + PAD,
                card.rect.y + PAD,
                Run::label(card.rect.width - PAD * 2.0)
                    .sized(TITLE_SIZE)
                    .bold(),
            );
            scene.glyphs(glyphs, palette.ink, palette.faint);

            let glyphs = painter.run(
                fonts,
                "Text",
                card.group.x,
                card.group.y,
                Run::label(card.group.width).sized(GROUP_SIZE),
            );
            scene.glyphs(glyphs, palette.faint, palette.faint);

            for choice in &card.choices {
                let under = named().under(input, choice.which.slug());
                let picked = choice.which == chosen;
                // The chosen one is a card of its own with the signal on its
                // edge, the way a chosen square on the rail is. Filled with
                // the signal colour it was the loudest thing on the panel, and
                // all it has to say is "this is the one in use".
                let behind = match (picked, under) {
                    (true, _) => palette.raised,
                    (false, true) => palette.hover,
                    (false, false) => palette.surface,
                };
                if picked {
                    Panel::flat(choice.rect, ROW_CORNER)
                        .edge([palette.signal[0], palette.signal[1], palette.signal[2], 255])
                        .fill(behind)
                        .draw(scene);
                } else if under {
                    scene.rounded(
                        choice.rect.x,
                        choice.rect.y,
                        choice.rect.width,
                        choice.rect.height,
                        behind,
                        ROW_CORNER,
                    );
                }
                dot(
                    scene,
                    Rect::new(
                        choice.rect.x + ROW_PAD_X,
                        choice.title.y + (choice.title.height - DOT) / 2.0,
                        DOT,
                        DOT,
                    ),
                    picked,
                    behind,
                    palette,
                );
                let glyphs = painter.run(
                    fonts,
                    choice.which.title(),
                    choice.title.x,
                    choice.title.y,
                    Run::label(choice.title.width).sized(LABEL_SIZE),
                );
                scene.glyphs(glyphs, palette.ink, palette.faint);
                let glyphs = painter.run(
                    fonts,
                    choice.which.said(),
                    choice.said.x,
                    choice.said.y,
                    Run::label(choice.said.width).sized(SAID_SIZE),
                );
                scene.glyphs(glyphs, palette.faint, palette.faint);
            }

            let glyphs = painter.run(
                fonts,
                KEPT_HEADING,
                card.kept.x,
                card.kept.y,
                Run::label(card.kept.width).sized(GROUP_SIZE),
            );
            scene.glyphs(glyphs, palette.faint, palette.faint);

            let glyphs = painter.run(
                fonts,
                "About",
                card.about.x,
                card.about.y,
                Run::label(card.about.width).sized(GROUP_SIZE),
            );
            scene.glyphs(glyphs, palette.faint, palette.faint);

            let glyphs = painter.run(
                fonts,
                &self.build(),
                card.build.x,
                card.build.y,
                Run::label(card.build.width).sized(LABEL_SIZE),
            );
            scene.glyphs(glyphs, palette.soft, palette.faint);

            if let Some(said) = card.said {
                let glyphs = painter.run(
                    fonts,
                    &self.said,
                    said.x,
                    said.y,
                    Run::label(said.width).sized(SAID_SIZE),
                );
                scene.glyphs(glyphs, palette.faint, palette.faint);
            }
        }
        card.sizes.draw(into, input);
        card.look.draw(into, input);
        card.foot.draw(into, input);
    }
}

/// The heading over how much disk opened pictures may keep.
const KEPT_HEADING: &str = "Opened pictures kept on disk";

/// The mark saying which choice is in use.
///
/// A ring with a dot in it rather than a tick: these two are one choice with
/// two answers, and a tick beside one of them says nothing about what pressing
/// the other would do. It was drawn with the bookmark mark, which said less
/// still.
fn dot(
    scene: &mut matterless_paint::Scene,
    at: Rect,
    picked: bool,
    behind: [u8; 4],
    palette: &matterless_paint::Palette,
) {
    let ring = match picked {
        true => [palette.signal[0], palette.signal[1], palette.signal[2], 255],
        false => palette.rule,
    };
    scene.rounded(at.x, at.y, at.width, at.height, ring, at.width / 2.0);
    let inset = 1.5;
    scene.rounded(
        at.x + inset,
        at.y + inset,
        at.width - inset * 2.0,
        at.height - inset * 2.0,
        behind,
        (at.width - inset * 2.0) / 2.0,
    );
    if picked {
        let inset = 4.5;
        scene.rounded(
            at.x + inset,
            at.y + inset,
            at.width - inset * 2.0,
            at.height - inset * 2.0,
            ring,
            (at.width - inset * 2.0) / 2.0,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> Rect {
        Rect::new(0.0, 0.0, 1002.0, 792.0)
    }

    fn shown() -> (Settings, Fonts) {
        let mut settings = Settings::default();
        let mut fonts = Fonts::new();
        settings.show();
        settings.measure(&mut fonts, window());
        (settings, fonts)
    }

    #[test]
    fn a_setting_survives_being_written_and_read() {
        for one in Text::both() {
            assert_eq!(Text::read(Some(one.stored())), one);
        }
        for one in Kept::all() {
            assert_eq!(Kept::read(Some(&one.stored())), one);
        }
        assert_eq!(Kept::read(Some("0")), Kept::Unlimited, "zero is no limit");
        assert_eq!(Kept::read(Some("12345")), Kept::default());
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
        let (settings, _fonts) = shown();
        let card = settings.laid().expect("measured");
        assert_eq!(card.choices.len(), 2);
        let mut floor = card.group.bottom();
        for choice in &card.choices {
            assert!(
                choice.rect.y >= floor - 0.5,
                "{:?} sits on what is above it",
                choice.which
            );
            floor = choice.rect.bottom();
        }
        // The sizes for opened pictures, under the choices and over About.
        let sizes = Kept::all().map(|one| card.sizes.rect(one.slug()).expect("a size button"));
        for size in sizes {
            assert!(size.y >= floor - 0.5, "a size button sits on the choices");
            assert!(
                size.bottom() <= card.about.y + 0.5,
                "a size button runs into About"
            );
        }
        // A box for each, plus the card, the window behind it, the sizes, and
        // the two buttons -- the one that looks for a build and the one that
        // shuts it.
        assert_eq!(
            settings.boxes(window()).len(),
            4 + card.choices.len() + sizes.len()
        );
    }

    /// What a choice says stays inside the choice.
    ///
    /// The fault this panel shipped with: a row was a constant 46 tall and the
    /// description under "Smooth" wraps to two lines, so its second line was
    /// printed across the name of the choice below it.
    #[test]
    fn what_a_choice_says_stays_inside_it() {
        let (settings, mut fonts) = shown();
        let card = settings.laid().expect("measured");
        for choice in &card.choices {
            let said = tall(
                &mut fonts,
                choice.which.said(),
                choice.said.width,
                SAID_SIZE,
            );
            assert!(
                said > SAID_SIZE * 1.4 * 1.5,
                "{:?} fits on one line here, so this proves nothing",
                choice.which
            );
            assert!(
                choice.said.y + said <= choice.rect.bottom() + 0.5,
                "{:?} says {said} of text with {} of room",
                choice.which,
                choice.rect.bottom() - choice.said.y
            );
        }
    }

    /// A press beside the card shuts it, and a press on it does not.
    #[test]
    fn it_shuts_from_outside_and_stays_from_within() {
        let (mut settings, _fonts) = shown();
        let boxes = settings.boxes(window());
        let card = settings.laid().expect("measured").rect;

        // On the card but not on a choice: the heading, which is somebody
        // reading rather than somebody answering.
        let mut input = Input::default();
        press(
            &mut input,
            &boxes,
            Rect::new(card.x + 4.0, card.y + 4.0, 2.0, 2.0),
        );
        assert_eq!(settings.react(&mut input), None, "a press on it shut it");
        assert!(settings.open());

        // Beside it: the whole-window box underneath.
        let mut input = Input::default();
        press(&mut input, &boxes, Rect::new(2.0, 2.0, 1.0, 1.0));
        assert_eq!(settings.react(&mut input), Some(Did::Close));
        assert!(!settings.open());

        // And the button, which is the obvious way.
        let (mut settings, _fonts) = shown();
        let boxes = settings.boxes(window());
        let done = settings
            .laid()
            .expect("measured")
            .foot
            .rect("done")
            .expect("placed");
        let mut input = Input::default();
        press(&mut input, &boxes, done);
        assert_eq!(settings.react(&mut input), Some(Did::Close));
        assert!(!settings.open());
    }

    /// The press that opens it does not also shut it.
    ///
    /// The gear is pressed, the panel opens, and the same frame hands that
    /// press back to the panel -- so a rule of "anything this widget cannot
    /// name shuts it" shuts it on the click that asked for it, and the panel
    /// never appears at all.
    #[test]
    fn the_press_that_opened_it_does_not_shut_it() {
        let (mut settings, _fonts) = shown();
        let gear = vec![Placed {
            name: "rail/settings".to_string(),
            rect: Rect::new(0.0, 0.0, 40.0, 40.0),
            depth: 2,
        }];
        let mut input = Input::default();
        press(&mut input, &gear, gear[0].rect);
        assert_eq!(settings.react(&mut input), None);
        assert!(settings.open(), "it shut on the press that opened it");
    }

    fn press(input: &mut Input, boxes: &[Placed], rect: Rect) {
        input.apply(
            matterless_ui::input::Event::PointerMoved {
                x: rect.x + rect.width / 2.0,
                y: rect.y + rect.height / 2.0,
            },
            boxes,
        );
        input.apply(matterless_ui::input::Event::PointerPressed, boxes);
        input.apply(matterless_ui::input::Event::PointerReleased, boxes);
    }

    /// Asking for a newer build says so, and answers where it was asked.
    ///
    /// The card grows by the line it answers on, which is why this measures
    /// again rather than trusting the first placement.
    #[test]
    fn looking_for_a_build_is_said_on_the_card() {
        let (mut settings, mut fonts) = shown();
        let before = settings.laid().expect("measured").rect.height;
        assert!(settings.laid().expect("measured").said.is_none());

        let boxes = settings.boxes(window());
        let look = settings
            .laid()
            .expect("measured")
            .look
            .rect("look")
            .expect("placed");
        let mut input = Input::default();
        press(&mut input, &boxes, look);
        assert_eq!(settings.react(&mut input), Some(Did::Look));
        assert!(settings.open(), "asking is not leaving");
        // Still drawable on the very frame it was asked on. Nothing measures
        // a panel except the window, and only when something is pressed -- so
        // a panel that drops its placement when its words change is a panel
        // that is not drawn at all until the next press, which is what a
        // reader sees as it blinking out under their hand.
        assert!(
            settings.laid().is_some(),
            "the card went missing on the press that asked"
        );

        settings.measure(&mut fonts, window());
        let card = settings.laid().expect("measured");
        assert!(card.said.is_some(), "nothing says a look went out");
        assert!(card.rect.height > before, "the card did not make room");
        // And the button is spent while the answer is on its way: a disabled
        // row draws dimmed and registers nothing to press.
        assert!(card.look.boxes().is_empty(), "it can be asked twice");

        settings.looked("This is the newest build.");
        settings.measure(&mut fonts, window());
        let card = settings.laid().expect("measured");
        assert!(!card.look.boxes().is_empty(), "the button never came back");
        assert!(card.said.is_some());
    }

    /// And the card holds everything measured into it.
    #[test]
    fn nothing_runs_out_of_the_card() {
        let (settings, _fonts) = shown();
        let card = settings.laid().expect("measured");
        let done = card.foot.rect("done").expect("placed");
        let last = card.choices.last().expect("a choice").rect;
        assert!(card.group.y > card.rect.y + PAD);
        assert!(
            last.bottom() <= card.about.y,
            "the choices reach the heading"
        );
        assert!(
            card.about.bottom() <= card.build.y,
            "the heading reaches the build"
        );
        assert!(
            card.build.bottom() <= done.y,
            "the build reaches the button"
        );
        assert!(
            done.bottom() <= card.rect.bottom() - PAD + 0.5,
            "the button reaches past the foot of the card"
        );
        assert!(card.rect.right() <= window().right() && card.rect.bottom() <= window().bottom());
    }
}
