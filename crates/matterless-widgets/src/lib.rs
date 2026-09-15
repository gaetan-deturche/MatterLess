//! Controls, described once.
//!
//! `matterless-ui` solves boxes and draws nothing; `matterless-paint` draws and
//! knows nothing about a pointer. A button needs both, and before this crate
//! existed every widget did that join itself -- sizing a button in one method,
//! registering its hit box in a second, matching its name in a third and
//! painting it in a fourth, with a formatted string as the only thing holding
//! the four together and nothing to notice when they disagreed.
//!
//! Counted across the window's widgets at the time this was written: 49 hit
//! boxes written out by hand, 47 rounded rectangles that were nearly all
//! buttons, and 33 spellings of the same `NAME/slug`. Adding one button to the
//! update strip meant editing four of those places, and the compiler had
//! nothing to say about it.
//!
//! So a control is declared once and everything else is read off that
//! declaration: where it sits, what it is called, whether the pointer is on it,
//! and what it looks like when it is.

use matterless_layout::{Fonts, Style};
use matterless_paint::{Painter, Palette, Run, Scene};
use matterless_ui::input::Input;
use matterless_ui::{Placed, Rect};

/// What a widget draws with.
///
/// Lived in the sidebar until this crate existed, which is where every other
/// widget imported it from -- a fair description of how the drawing side of
/// this app grew.
pub struct Canvas<'a> {
    pub scene: &'a mut Scene,
    pub painter: &'a mut Painter,
    pub fonts: &'a mut Fonts,
    pub palette: &'a Palette,
}

/// What a widget's hit boxes are called.
///
/// A hit box's name is the widget's name and the control's slug, joined. That
/// join is spelled once here, because a widget otherwise writes it out again
/// in its hit test, again in its hover check, again where it answers a press
/// and again where it draws -- and a spelling that differs by one character is
/// a box nobody can press, with nothing to say so.
///
/// Not just a `format!` in a nicer coat: `slug` is the inverse, and it is the
/// half that was being written out longhand everywhere.
#[derive(Debug, Clone)]
pub struct Named {
    name: String,
}

impl Named {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    /// The widget's own name, for the box that catches everything else.
    pub fn whole(&self) -> String {
        self.name.clone()
    }

    /// What the box for `slug` is called.
    pub fn of(&self, slug: &str) -> String {
        format!("{}/{}", self.name, slug)
    }

    /// One placed box, named.
    pub fn at(&self, slug: &str, rect: Rect, depth: usize) -> Placed {
        Placed {
            name: self.of(slug),
            rect,
            depth,
        }
    }

    /// The slug inside one of this widget's names, if it is one of this
    /// widget's names at all.
    pub fn slug<'a>(&self, name: &'a str) -> Option<&'a str> {
        name.strip_prefix(&self.name)?.strip_prefix('/')
    }

    /// Which of this widget's controls the pointer is on.
    pub fn hovered<'a>(&self, input: &'a Input) -> Option<&'a str> {
        self.slug(input.hovered()?)
    }

    /// Which of this widget's controls was pressed.
    pub fn clicked<'a>(&self, input: &'a Input) -> Option<&'a str> {
        self.slug(input.clicked()?)
    }

    /// Whether the pointer is on this one in particular, which is what a draw
    /// pass asks once per row.
    pub fn under(&self, input: &Input, slug: &str) -> bool {
        self.hovered(input) == Some(slug)
    }
}

/// How a control is drawn, and so what it claims about itself.
///
/// One `Primary` to a row, or the row is saying two things are the one thing
/// to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    /// The thing to do: the signal colour, and white on it.
    Primary,
    /// Everything else: the panel's own colours and a hairline round it.
    Plain,
}

/// One control, before it knows where it is.
#[derive(Debug, Clone)]
pub struct Button {
    /// What it is called, inside its widget. The hit box's name is the
    /// widget's name and this, joined -- spelled once, here.
    pub slug: &'static str,
    pub label: String,
    pub look: Look,
}

impl Button {
    pub fn plain(slug: &'static str, label: impl Into<String>) -> Self {
        Self {
            slug,
            label: label.into(),
            look: Look::Plain,
        }
    }

    pub fn primary(slug: &'static str, label: impl Into<String>) -> Self {
        Self {
            slug,
            label: label.into(),
            look: Look::Primary,
        }
    }
}

/// How big a control is, which is the same everywhere in this window.
///
/// `font-size: 12.5px` padded `10px`, `height: 25px`, `border-radius: 5px`.
#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    pub size: f32,
    pub pad_x: f32,
    pub height: f32,
    pub corner: f32,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            size: 12.5,
            pad_x: 10.0,
            height: 25.0,
            corner: 5.0,
        }
    }
}

/// A row of controls against one edge of a rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Against {
    /// Laid right to left from the right edge, which is where an answer goes.
    Right,
    /// Left to right from the left edge.
    Left,
}

/// A row of controls waiting to be measured.
pub struct Row {
    name: Named,
    within: Rect,
    against: Against,
    buttons: Vec<Button>,
    metrics: Metrics,
    pad: f32,
    gap: f32,
    depth: usize,
    enabled: bool,
}

impl Row {
    /// A row inside `within`, belonging to the widget called `name`.
    pub fn new(name: impl Into<String>, within: Rect) -> Self {
        Self {
            name: Named::new(name),
            within,
            against: Against::Right,
            buttons: Vec::new(),
            metrics: Metrics::default(),
            pad: 12.0,
            gap: 8.0,
            depth: 30,
            enabled: true,
        }
    }

    pub fn against(mut self, against: Against) -> Self {
        self.against = against;
        self
    }

    pub fn pad(mut self, pad: f32) -> Self {
        self.pad = pad;
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    pub fn metrics(mut self, metrics: Metrics) -> Self {
        self.metrics = metrics;
        self
    }

    /// Where the row's hit boxes sit against everything else's. The row itself
    /// is not a box; only its controls are.
    pub fn depth(mut self, depth: usize) -> Self {
        self.depth = depth;
        self
    }

    /// Whether anything in the row can be pressed.
    ///
    /// A disabled row is drawn dimmed and registers no hit boxes at all, which
    /// is the only honest way to say "not now": a button that looks pressable
    /// and answers nothing is worse than one that looks spent.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn button(mut self, button: Button) -> Self {
        self.buttons.push(button);
        self
    }

    /// Adds one only if it is there to add, which every widget with an
    /// optional control was writing out itself.
    pub fn maybe(self, button: Option<Button>) -> Self {
        match button {
            Some(button) => self.button(button),
            None => self,
        }
    }

    /// Measures the labels and places the row. The one step that needs fonts.
    pub fn measure(self, fonts: &mut Fonts) -> Laid {
        let middle = self.within.y + (self.within.height - self.metrics.height) / 2.0;
        let mut rects = Vec::with_capacity(self.buttons.len());
        // Declared left to right, whichever edge the row is against: reading
        // order is what somebody writing the widget has in their head, and a
        // list that came out mirrored because it happened to be right-aligned
        // would be a trap rather than an API.
        let mut edge = match self.against {
            Against::Right => self.within.right() - self.pad,
            Against::Left => self.within.x + self.pad,
        };
        let order: Vec<&Button> = match self.against {
            Against::Right => self.buttons.iter().rev().collect(),
            Against::Left => self.buttons.iter().collect(),
        };
        for button in order {
            let wide = width_of(fonts, &button.label, self.metrics.size) + self.metrics.pad_x * 2.0;
            let x = match self.against {
                Against::Right => edge - wide,
                Against::Left => edge,
            };
            rects.push(Rect::new(x, middle, wide, self.metrics.height));
            edge = match self.against {
                Against::Right => x - self.gap,
                Against::Left => x + wide + self.gap,
            };
        }
        // Back into the order they were declared in, so a slug and a rectangle
        // still line up.
        if self.against == Against::Right {
            rects.reverse();
        }
        // What the row did not take, for whatever the widget puts beside it.
        let spare = match self.against {
            Against::Right => (edge - (self.within.x + self.pad)).max(0.0),
            Against::Left => (self.within.right() - self.pad - edge).max(0.0),
        };
        Laid {
            name: self.name,
            buttons: self.buttons,
            rects,
            metrics: self.metrics,
            depth: self.depth,
            enabled: self.enabled,
            spare,
        }
    }
}

/// A row that knows where it is.
#[derive(Debug, Clone)]
pub struct Laid {
    name: Named,
    buttons: Vec<Button>,
    rects: Vec<Rect>,
    metrics: Metrics,
    depth: usize,
    enabled: bool,
    spare: f32,
}

impl Laid {
    /// The room the controls left, for the words that go beside them.
    pub fn spare(&self) -> f32 {
        self.spare
    }

    /// Where one control ended up, by the slug it was declared with.
    pub fn rect(&self, slug: &str) -> Option<Rect> {
        let at = self.buttons.iter().position(|button| button.slug == slug)?;
        self.rects.get(at).copied()
    }

    /// The hit boxes, named from the slugs. Nothing when the row is disabled.
    pub fn boxes(&self) -> Vec<Placed> {
        if !self.enabled {
            return Vec::new();
        }
        self.buttons
            .iter()
            .zip(&self.rects)
            .map(|(button, rect)| self.name.at(button.slug, *rect, self.depth))
            .collect()
    }

    /// Which control was pressed, as the slug it was declared with -- so the
    /// widget answering it never spells the name a second time.
    pub fn clicked(&self, input: &Input) -> Option<&'static str> {
        if !self.enabled {
            return None;
        }
        let clicked = self.name.clicked(input)?;
        self.buttons
            .iter()
            .find(|button| button.slug == clicked)
            .map(|button| button.slug)
    }

    pub fn draw(&self, into: &mut Canvas<'_>, input: &Input) {
        for (button, rect) in self.buttons.iter().zip(&self.rects) {
            let under = self.enabled && self.name.under(input, button.slug);
            self.one(into, button, *rect, under);
        }
    }

    fn one(&self, into: &mut Canvas<'_>, button: &Button, rect: Rect, under: bool) {
        let Canvas {
            scene,
            painter,
            fonts,
            palette,
        } = into;
        let primary = button.look == Look::Primary;
        let signal = [palette.signal[0], palette.signal[1], palette.signal[2], 255];
        let edge = match primary {
            true => signal,
            false => palette.rule,
        };
        let ground = match (primary, under) {
            (true, _) => signal,
            (false, true) => palette.raised,
            (false, false) => palette.surface,
        };
        // The hairline is the ground, drawn one pixel bigger than the fill in
        // front of it -- the same trick every panel in this window uses.
        scene.rounded(
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            edge,
            self.metrics.corner,
        );
        scene.rounded(
            rect.x + 1.0,
            rect.y + 1.0,
            rect.width - 2.0,
            rect.height - 2.0,
            ground,
            self.metrics.corner - 1.0,
        );
        let ink = match primary {
            true => [255, 255, 255],
            false => palette.ink,
        };
        // Dimmed when there is nothing to press, which is what `:disabled`
        // does and the only thing saying a press was heard.
        let ink = match self.enabled {
            true => ink,
            false => palette.dimmed(ink, 0.55),
        };
        let glyphs = painter.run(
            fonts,
            &button.label,
            rect.x + self.metrics.pad_x,
            rect.y + (rect.height - self.metrics.size * 1.4) / 2.0,
            Run::label(f32::MAX).sized(self.metrics.size),
        );
        scene.glyphs(glyphs, ink, palette.faint);
    }
}

/// How wide a run of text is at `size`, which is how a button is sized to its
/// own label rather than to a guess.
pub fn width_of(fonts: &mut Fonts, text: &str, size: f32) -> f32 {
    matterless_layout::extent_of(fonts, text, f32::MAX, style(size)).width
}

/// The plain style every control's label is set in.
pub fn style(size: f32) -> Style {
    Style {
        size,
        line_height: size * 1.4,
        bold: false,
        italic: false,
        mono: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fonts() -> Fonts {
        Fonts::new()
    }

    fn strip() -> Rect {
        Rect::new(0.0, 0.0, 900.0, 38.0)
    }

    fn row() -> Row {
        Row::new("update", strip())
            .button(Button::plain("news", "What's new"))
            .button(Button::primary("install", "Install and restart"))
            .button(Button::plain("later", "Not now"))
    }

    /// The join, and its inverse. The inverse is the half that was being
    /// written out longhand in every widget that answers a press.
    #[test]
    fn a_name_joins_and_comes_apart_again() {
        let named = Named::new("menu");
        assert_eq!(named.of("save"), "menu/save");
        assert_eq!(named.slug("menu/save"), Some("save"));
        // An id with a slash of its own survives, which the message menu needs
        // for its nested rows.
        assert_eq!(named.slug("menu/remind/nest"), Some("remind/nest"));
        // Somebody else's box is not ours, and neither is the bare name.
        assert_eq!(named.slug("sidebar/save"), None);
        assert_eq!(named.slug("menu"), None);
        // And a widget whose name is a prefix of ours is not ours either.
        assert_eq!(Named::new("menu").slug("menubar/save"), None);
    }

    /// The point of the crate: one declaration, and the hit box, the name and
    /// the drawing all come off it.
    #[test]
    fn a_control_is_declared_once_and_found_by_its_slug() {
        let laid = row().measure(&mut fonts());
        let boxes = laid.boxes();
        assert_eq!(boxes.len(), 3);
        for button in ["news", "install", "later"] {
            let rect = laid.rect(button).expect("it was placed");
            let placed = boxes
                .iter()
                .find(|placed| placed.name == format!("update/{button}"))
                .expect("it has a hit box");
            // The hit box is the rectangle it is drawn in. Written out twice,
            // this is exactly what came adrift.
            assert_eq!(placed.rect, rect);
        }
    }

    /// Against the right edge, in the order they were named, none of them
    /// overlapping and all of them inside.
    #[test]
    fn a_row_lands_against_its_edge_in_order() {
        let laid = row().pad(14.0).gap(8.0).measure(&mut fonts());
        let news = laid.rect("news").expect("placed");
        let install = laid.rect("install").expect("placed");
        let later = laid.rect("later").expect("placed");
        assert!((later.right() - (strip().right() - 14.0)).abs() < 0.5);
        assert!(install.right() <= later.x);
        assert!(news.right() <= install.x);
        assert!(news.x >= strip().x);
        // Centred in the row it was given.
        assert!((news.y - (strip().height - news.height) / 2.0).abs() < 0.5);
    }

    /// The other way round, for a row that reads left to right.
    #[test]
    fn a_row_can_go_the_other_way() {
        let laid = row().against(Against::Left).measure(&mut fonts());
        let news = laid.rect("news").expect("placed");
        let install = laid.rect("install").expect("placed");
        assert!(news.x < install.x);
        assert!(news.right() <= install.x);
    }

    /// What is left over is what the widget has for its words. Measured rather
    /// than guessed at, because a label's width is the font's business.
    #[test]
    fn the_room_left_over_is_reported() {
        let laid = row().pad(14.0).measure(&mut fonts());
        let news = laid.rect("news").expect("placed");
        assert!(laid.spare() > 0.0);
        assert!(laid.spare() <= news.x);
    }

    /// A disabled row offers nothing to press. Registering the boxes anyway
    /// and ignoring the press is how a button comes to look alive and do
    /// nothing.
    #[test]
    fn a_disabled_row_has_nothing_to_press() {
        let laid = row().enabled(false).measure(&mut fonts());
        assert!(laid.boxes().is_empty());
        // And it is still laid out, because it still has to be drawn.
        assert!(laid.rect("install").is_some());
    }

    /// A press answers with the slug, not with a string the widget has to
    /// spell again.
    #[test]
    fn a_press_answers_with_the_slug() {
        let laid = row().measure(&mut fonts());
        let boxes = laid.boxes();
        let mut input = Input::default();
        let rect = laid.rect("install").expect("placed");
        input.apply(
            matterless_ui::input::Event::PointerMoved {
                x: rect.x + rect.width / 2.0,
                y: rect.y + rect.height / 2.0,
            },
            &boxes,
        );
        input.apply(matterless_ui::input::Event::PointerPressed, &boxes);
        input.apply(matterless_ui::input::Event::PointerReleased, &boxes);
        assert_eq!(laid.clicked(&input), Some("install"));
    }

    /// A button is as wide as its own label plus its padding, which is what
    /// stops a long word running out of its own box.
    #[test]
    fn a_button_is_as_wide_as_what_it_says() {
        let mut fonts = fonts();
        let metrics = Metrics::default();
        let laid = row().metrics(metrics).measure(&mut fonts);
        for slug in ["news", "install", "later"] {
            let rect = laid.rect(slug).expect("placed");
            let label = &laid
                .buttons
                .iter()
                .find(|button| button.slug == slug)
                .expect("declared")
                .label;
            let words = width_of(&mut fonts, label, metrics.size);
            assert!(
                (rect.width - (words + metrics.pad_x * 2.0)).abs() < 0.5,
                "{slug}"
            );
        }
    }

    /// An optional control is not there when there is nothing to offer, and
    /// the row closes up behind it rather than leaving a gap.
    #[test]
    fn an_optional_control_can_be_left_out() {
        let with = Row::new("update", strip())
            .maybe(Some(Button::plain("news", "What's new")))
            .button(Button::primary("install", "Install"))
            .measure(&mut fonts());
        let without = Row::new("update", strip())
            .maybe(None)
            .button(Button::primary("install", "Install"))
            .measure(&mut fonts());
        assert!(with.rect("news").is_some());
        assert!(without.rect("news").is_none());
        assert_eq!(without.boxes().len(), 1);
        // The one that stayed has not moved: it is against the same edge.
        assert_eq!(with.rect("install"), without.rect("install"));
        assert!(without.spare() > with.spare());
    }
}
