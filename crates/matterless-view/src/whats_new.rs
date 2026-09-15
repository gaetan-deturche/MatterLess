//! What a release said about itself, over the window.
//!
//! The strip along the top asks for a restart; this is the answer to "why
//! would I". It is opened from there and belongs to nothing else, but it is
//! its own panel rather than part of the strip: a bar that grew to hold a
//! change list would push the conversation down by however much the release
//! happened to say, which is a lot of the window moving for something the
//! reader is about to shut again.
//!
//! Modal, in the one sense that matters here: a press anywhere outside it
//! closes it, so there is no way to leave it open and forget about it.

use matterless_paint::Run;
use matterless_ui::input::Input;
use matterless_ui::{Placed, Rect};
use matterless_widgets::{Button, Canvas, Laid, Named, Row};

pub const NAME: &str = "whats-new";
/// What this widget's hit boxes are called. The join and its inverse in one
/// place, so the box it registers and the press it answers cannot disagree.
fn named() -> Named {
    Named::new(NAME)
}

/// How big the panel is allowed to get, against the window it is over.
///
/// A column narrow enough to read and short enough to still show where you
/// were. Past that the list is cut rather than the panel grown: this is a
/// summary somebody glances at before deciding, not a document.
const WIDEST: f32 = 560.0;
const TALLEST: f32 = 0.66;
const PAD: f32 = 18.0;
const CORNER: f32 = 10.0;
const DROP: f32 = 16.0;
const TITLE_SIZE: f32 = 15.0;
const NOTES_SIZE: f32 = 12.5;
const GAP: f32 = 12.0;
/// How tall the row at the bottom is. What the button in it is set in comes
/// from `matterless_widgets::Metrics`, the same as every other control.
const BUTTON_HEIGHT: f32 = 26.0;

/// The panel, when it is up.
#[derive(Debug, Default)]
pub struct WhatsNew {
    version: String,
    notes: String,
    /// The panel, the words in it and the row at its foot, measured together.
    placed: Option<(Rect, Rect, Laid)>,
}

impl WhatsNew {
    pub fn open(&self) -> bool {
        !self.version.is_empty()
    }

    pub fn show(&mut self, version: &str, notes: &str) {
        self.version = version.to_string();
        self.notes = notes.trim().to_string();
        self.placed = None;
    }

    pub fn hide(&mut self) {
        self.version.clear();
        self.notes.clear();
        self.placed = None;
    }

    fn title(&self) -> String {
        format!("What's new in MatterLess {}", self.version)
    }

    fn style(size: f32) -> matterless_layout::Style {
        matterless_layout::Style {
            size,
            line_height: size * 1.5,
            bold: false,
            italic: false,
            mono: false,
        }
    }

    /// The change list, cut to what the panel will show.
    ///
    /// By source line and then measured, because one line of a change list
    /// wraps to several and it is the wrapped height that has to fit. Cut
    /// rather than clipped, so the ellipsis can say it was cut -- a list that
    /// simply stopped would read as the whole of it.
    fn shown(&self, fonts: &mut matterless_layout::Fonts, wrap: f32, tallest: f32) -> String {
        if self.notes.is_empty() {
            return String::new();
        }
        let lines: Vec<&str> = self.notes.lines().collect();
        let mut taken = lines.len();
        loop {
            let cut = taken < lines.len();
            let mut text = lines[..taken].join("\n");
            if cut {
                text.push('\n');
                text.push(matterless_layout::ELLIPSIS);
            }
            let tall =
                matterless_layout::extent_of(fonts, &text, wrap, Self::style(NOTES_SIZE)).height;
            if taken <= 1 || tall <= tallest {
                return text;
            }
            taken -= 1;
        }
    }

    pub fn measure(&mut self, fonts: &mut matterless_layout::Fonts, window: Rect) {
        self.placed = self.open().then(|| self.place(fonts, window));
    }

    fn laid(&self) -> Option<(Rect, Rect, &Laid)> {
        let (panel, notes, row) = self.placed.as_ref()?;
        Some((*panel, *notes, row))
    }

    /// The panel, the change list inside it, and the row that shuts it.
    fn place(&self, fonts: &mut matterless_layout::Fonts, window: Rect) -> (Rect, Rect, Laid) {
        let width = WIDEST.min(window.width - PAD * 2.0).max(0.0);
        let wrap = (width - PAD * 2.0).max(0.0);
        let title =
            matterless_layout::extent_of(fonts, &self.title(), wrap, Self::style(TITLE_SIZE))
                .height;
        // What the list may take: the panel's cap, less everything that is not
        // the list.
        let furniture = PAD * 2.0 + title + GAP + GAP + BUTTON_HEIGHT;
        let tallest = (window.height * TALLEST - furniture).max(NOTES_SIZE * 1.5);
        let shown = self.shown(fonts, wrap, tallest);
        let list =
            matterless_layout::extent_of(fonts, &shown, wrap, Self::style(NOTES_SIZE)).height;
        let height = furniture + list;

        let panel = Rect::new(
            window.x + (window.width - width) / 2.0,
            window.y + (window.height - height) / 2.0,
            width,
            height,
        );
        let notes = Rect::new(panel.x + PAD, panel.y + PAD + title + GAP, wrap, list);
        let row = Row::new(
            NAME,
            Rect::new(
                panel.x,
                panel.bottom() - PAD - BUTTON_HEIGHT,
                panel.width,
                BUTTON_HEIGHT,
            ),
        )
        .pad(PAD)
        .depth(42)
        .button(Button::plain("close", "Close"))
        .measure(fonts);
        (panel, notes, row)
    }

    pub fn boxes(&self, window: Rect) -> Vec<Placed> {
        let Some((panel, _, row)) = self.laid() else {
            return Vec::new();
        };
        // The whole window first, so a press beside the panel shuts it rather
        // than reaching the conversation it is covering; then the panel, which
        // catches its own presses so reading does not shut it.
        let mut placed = vec![
            Placed {
                name: NAME.to_string(),
                rect: window,
                depth: 40,
            },
            named().at("panel", panel, 41),
        ];
        placed.extend(row.boxes());
        placed
    }

    /// Whether it was shut. A press on the panel itself is not: reading is
    /// what it is for.
    pub fn react(&mut self, input: &Input) -> bool {
        if !self.open() {
            return false;
        }
        let closed = self
            .laid()
            .and_then(|(_, _, row)| row.clicked(input))
            .is_some();
        let beside = input.clicked() == Some(NAME);
        if closed || beside {
            self.hide();
        }
        closed || beside
    }

    pub fn draw(&self, into: &mut Canvas<'_>, input: &Input, window: Rect) {
        let Some((panel, notes, row)) = self.laid() else {
            return;
        };
        let row = row.clone();
        let title = self.title();
        let shown = {
            let Canvas { fonts, .. } = into;
            self.shown(fonts, notes.width, notes.height.max(1.0))
        };
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        // The window behind it, dimmed rather than hidden: the reader is about
        // to decide whether to restart, and covering where they were entirely
        // takes away half of what that decision is about.
        scene.fill(
            window.x,
            window.y,
            window.width,
            window.height,
            [0, 0, 0, 160],
        );
        scene.floating(
            panel.x,
            panel.y,
            panel.width,
            panel.height,
            palette.rule,
            CORNER,
            DROP,
        );
        scene.rounded(
            panel.x + 1.0,
            panel.y + 1.0,
            panel.width - 2.0,
            panel.height - 2.0,
            palette.surface,
            CORNER - 1.0,
        );

        let glyphs = painter.run(
            fonts,
            &title,
            panel.x + PAD,
            panel.y + PAD,
            Run::label(panel.width - PAD * 2.0).sized(TITLE_SIZE).bold(),
        );
        scene.glyphs(glyphs, palette.ink, palette.faint);

        let glyphs = painter.run(
            fonts,
            &shown,
            notes.x,
            notes.y,
            Run::label(notes.width).sized(NOTES_SIZE),
        );
        scene.glyphs(glyphs, palette.soft, palette.faint);

        row.draw(into, input);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> Rect {
        Rect::new(0.0, 0.0, 1000.0, 700.0)
    }

    fn shown(notes: &str) -> (WhatsNew, matterless_layout::Fonts) {
        let mut panel = WhatsNew::default();
        let mut fonts = matterless_layout::Fonts::new();
        panel.show("0.1.6", notes);
        panel.measure(&mut fonts, window());
        (panel, fonts)
    }

    #[test]
    fn it_is_not_there_until_it_is_asked_for() {
        let mut fonts = matterless_layout::Fonts::new();
        let mut panel = WhatsNew::default();
        panel.measure(&mut fonts, window());
        assert!(!panel.open());
        assert!(panel.boxes(window()).is_empty());
    }

    /// Centred on the window and inside it, with the change list and the
    /// button inside the panel.
    #[test]
    fn it_sits_in_the_middle_of_the_window() {
        let (panel, _fonts) = shown("- fixed the thing\n- fixed the other thing");
        let (at, notes, row) = panel.laid().expect("measured");
        let close = row.rect("close").expect("placed");
        assert!(at.x >= window().x && at.right() <= window().right());
        assert!(at.y >= window().y && at.bottom() <= window().bottom());
        // Centred, to the pixel the arithmetic gives.
        assert!((at.x - (window().width - at.width) / 2.0).abs() < 0.5);
        assert!(notes.y > at.y && notes.bottom() <= close.y);
        assert!(close.right() <= at.right() && close.bottom() <= at.bottom());
    }

    /// The panel is capped and the list is cut to it. Grown to fit instead, a
    /// release with a hundred lines would cover the window it is explaining.
    #[test]
    fn a_long_change_list_is_cut_to_the_panel() {
        let many = (0..300)
            .map(|at| format!("- something that changed, the {at}th of them"))
            .collect::<Vec<_>>()
            .join("\n");
        let (panel, mut fonts) = shown(&many);
        let (at, notes, _) = panel.laid().expect("measured");
        assert!(
            at.height <= window().height * TALLEST + 1.0,
            "{} of {}",
            at.height,
            window().height
        );
        assert!(at.bottom() <= window().bottom());
        let text = panel.shown(&mut fonts, notes.width, notes.height);
        assert!(text.ends_with(matterless_layout::ELLIPSIS), "{text:?}");
        assert!(text.lines().count() < many.lines().count());
    }

    /// A press beside it shuts it; a press on it does not. Reading is what it
    /// is for, and a panel that vanished when clicked would be unusable.
    #[test]
    fn it_shuts_from_outside_and_stays_from_within() {
        let (mut panel, _fonts) = shown("- fixed the thing");
        let boxes = panel.boxes(window());
        let (at, _, _) = panel.laid().expect("measured");

        let mut input = Input::default();
        press(&mut input, &boxes, at);
        assert!(!panel.react(&input), "a press on it shut it");
        assert!(panel.open());

        // Beside it: the whole-window box underneath.
        let mut input = Input::default();
        press(&mut input, &boxes, Rect::new(2.0, 2.0, 1.0, 1.0));
        assert!(panel.react(&input));
        assert!(!panel.open());

        // And the button, which is the obvious way.
        let (mut panel, _fonts) = shown("- fixed the thing");
        let boxes = panel.boxes(window());
        let close = panel
            .laid()
            .expect("measured")
            .2
            .rect("close")
            .expect("placed");
        let mut input = Input::default();
        press(&mut input, &boxes, close);
        assert!(panel.react(&input));
        assert!(!panel.open());
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
}
