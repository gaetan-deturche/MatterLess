//! A list of things to do, opened at a point.
//!
//! Two of them in the app -- the right-click on a channel row, and the one
//! behind the `...` on a message -- and one shape between them: a floating
//! panel of rows, a rule where the items change subject, and a click anywhere
//! else to shut it. Only the numbers differ, so they are one widget and a
//! `Style`.
//!
//! This is also where most of what the port had put on the message toolbar
//! belongs. The app offers three controls there and a menu behind the third;
//! eight flat buttons across the top of a message was that menu spilled into
//! the row.

use matterless_paint::Run;
use matterless_ui::input::{Input, Key};
use matterless_ui::{Placed, Rect};
use matterless_widgets::{Canvas, Named};

pub const NAME: &str = "menu";

/// What this widget's hit boxes are called.
///
/// The join and its inverse in one place. Written out longhand, this menu
/// spelled its own name nine times across four methods that all had to agree
/// about it -- and the rows carry ids from the caller, so a disagreement would
/// be a menu entry that draws, highlights, and does nothing.
fn named() -> Named {
    Named::new(NAME)
}

/// The colour a row is written in.
///
/// Leaving a channel is `--flag` and a delete is the danger colour: the two
/// nobody wants to press by accident say so before they are pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tint {
    #[default]
    Ink,
    Flag,
    Danger,
}

/// One row: a thing to do, a rule, or a thing that asks first.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Item {
    /// What a hit test carries back, which is what the caller acts on.
    pub id: String,
    /// The 16px column before the label. Empty in the message menu, which has
    /// no such column.
    pub glyph: String,
    pub label: String,
    pub tint: Tint,
    /// A separator rather than a button.
    pub rule: bool,
    /// What this row leads to: the choices of a "Delete..." or a "Remind
    /// me...", or the categories behind a "Move to...".
    pub children: Vec<Item>,
    /// Whether the children take the row's own place or open beside it.
    ///
    /// In place for the two that ask a question -- "a submenu that opens
    /// sideways has nowhere to go at the edge of the window" -- and beside it
    /// for the one that offers a list.
    pub inline: bool,
}

impl Item {
    pub fn new(id: &str, label: &str) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            ..Self::default()
        }
    }

    /// A separator. Carries no id because nothing can land on it.
    pub fn rule() -> Self {
        Self {
            rule: true,
            ..Self::default()
        }
    }

    pub fn marked(mut self, glyph: &str) -> Self {
        self.glyph = glyph.to_string();
        self
    }

    pub fn tinted(mut self, tint: Tint) -> Self {
        self.tint = tint;
        self
    }

    /// Choices that replace this row once it is pressed.
    pub fn asks(mut self, children: Vec<Item>) -> Self {
        self.children = children;
        self.inline = true;
        self
    }

    /// A list that opens to the side on hover.
    pub fn nests(mut self, children: Vec<Item>) -> Self {
        self.children = children;
        self.inline = false;
        self
    }

    fn pressable(&self) -> bool {
        !self.rule
    }
}

/// The numbers one menu is drawn with.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    pub width: f32,
    pub corner: f32,
    pub padding: f32,
    pub item: f32,
    pub item_corner: f32,
    pub size: f32,
    pub pad_x: f32,
    /// Whether rows reserve the 16px glyph column.
    pub glyphs: bool,
    pub drop: f32,
}

impl Style {
    /// `min-width: 190px; padding: 4px; border-radius: 6px`, rows of
    /// `font-size: 12.5px` padded `5px 8px` with `border-radius: 4px`, under a
    /// `0 6px 18px` shadow.
    pub fn message() -> Self {
        Self {
            width: 190.0,
            corner: 6.0,
            padding: 4.0,
            item: 25.0,
            item_corner: 4.0,
            size: 12.5,
            pad_x: 8.0,
            glyphs: false,
            drop: 6.0,
        }
    }

    /// `min-width: 200px; padding: 4px; border-radius: 7px`, rows of
    /// `font-size: 13px` padded `6px 9px` with `border-radius: 5px`, a 16px
    /// glyph column, under a `0 8px 24px` shadow.
    pub fn channel() -> Self {
        Self {
            width: 200.0,
            corner: 7.0,
            padding: 4.0,
            item: 28.0,
            item_corner: 5.0,
            size: 13.0,
            pad_x: 9.0,
            glyphs: true,
            drop: 8.0,
        }
    }

    /// The nested list: narrower, and without the parent's glyph column.
    fn nested(self) -> Self {
        Self {
            width: 160.0,
            glyphs: false,
            ..self
        }
    }

    fn height_of(&self, item: &Item) -> f32 {
        // A rule is a hairline with 4px of air either side.
        if item.rule { 9.0 } else { self.item }
    }

    /// Where the label starts, past the glyph column when there is one.
    fn text_x(&self) -> f32 {
        if self.glyphs {
            self.pad_x + 16.0 + 9.0
        } else {
            self.pad_x
        }
    }
}

/// Where a menu hangs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Anchor {
    /// At the pointer, which is where a right-click puts it.
    At(f32, f32),
    /// Under a button and right-aligned with it: `top: 100%; right: 0`.
    Under(Rect),
}

impl Default for Anchor {
    fn default() -> Self {
        Anchor::At(0.0, 0.0)
    }
}

/// An open menu, and what it is about.
#[derive(Debug)]
pub struct Menu {
    pub items: Vec<Item>,
    /// What it was opened on -- a post id, a channel id -- handed back with
    /// whatever is chosen, so the caller does not have to remember it.
    pub about: String,
    pub style: Style,
    anchor: Anchor,
    /// The inline row currently asking its question.
    armed: Option<String>,
    /// The nested list currently open, which hover decides.
    nested: Option<String>,
}

impl Default for Menu {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            about: String::new(),
            style: Style::message(),
            anchor: Anchor::default(),
            armed: None,
            nested: None,
        }
    }
}

impl Menu {
    pub fn open(&self) -> bool {
        !self.items.is_empty()
    }

    pub fn show(&mut self, about: &str, anchor: Anchor, style: Style, items: Vec<Item>) {
        self.about = about.to_string();
        self.anchor = anchor;
        self.style = style;
        self.items = items;
        self.armed = None;
        self.nested = None;
    }

    pub fn hide(&mut self) {
        self.items.clear();
        self.about.clear();
        self.armed = None;
        self.nested = None;
    }

    fn height(&self) -> f32 {
        self.style.padding * 2.0
            + self
                .items
                .iter()
                .map(|item| self.style.height_of(item))
                .sum::<f32>()
    }

    /// Where it sits: where it was asked for, and never off the window.
    ///
    /// Pushed back on rather than flipped, which is what the app does -- a
    /// menu that jumped to the other side of the pointer would move out from
    /// under the hand that opened it.
    pub fn rect(&self, within: Rect) -> Rect {
        let height = self.height();
        let (x, y) = match self.anchor {
            Anchor::At(x, y) => (x, y),
            Anchor::Under(under) => (under.right() - self.style.width, under.bottom() + 2.0),
        };
        Rect::new(
            x.min(within.right() - self.style.width - 8.0)
                .max(within.x + 4.0),
            y.min(within.bottom() - height - 8.0).max(within.y + 4.0),
            self.style.width,
            height,
        )
    }

    /// Every row, with the rect it occupies.
    fn rows(&self, within: Rect) -> Vec<(&Item, Rect)> {
        let panel = self.rect(within);
        let mut top = panel.y + self.style.padding;
        let mut rows = Vec::new();
        for item in &self.items {
            let height = self.style.height_of(item);
            rows.push((
                item,
                Rect::new(
                    panel.x + self.style.padding,
                    top,
                    panel.width - self.style.padding * 2.0,
                    height,
                ),
            ));
            top += height;
        }
        rows
    }

    /// Where a nested list hangs off its parent: `left: calc(100% - 4px);
    /// top: -4px`, and clamped the same way the menu itself is.
    fn nest_rect(&self, parent: Rect, children: usize, within: Rect) -> Rect {
        let style = self.style.nested();
        let height = style.padding * 2.0 + children as f32 * style.item;
        Rect::new(
            (parent.right() - 4.0)
                .min(within.right() - style.width - 8.0)
                .max(within.x + 4.0),
            (parent.y - 4.0)
                .min(within.bottom() - height - 8.0)
                .max(within.y + 4.0),
            style.width,
            height,
        )
    }

    /// The choices an armed row shows in its own place, laid out left to right.
    fn inline_rects(&self, row: Rect, children: &[Item]) -> Vec<Rect> {
        // The question keeps the left ("Delete?", "Remind me in") and the
        // answers take what is left, which is the shape of `.confirm`.
        let asked = self.style.pad_x + label_width(&self.style, "Remind me in");
        let room = (row.width - asked - self.style.pad_x).max(10.0);
        let each = room / children.len().max(1) as f32;
        (0..children.len())
            .map(|at| {
                Rect::new(
                    row.x + asked + at as f32 * each,
                    row.y + 3.0,
                    each - 3.0,
                    row.height - 6.0,
                )
            })
            .collect()
    }

    pub fn boxes(&self, within: Rect) -> Vec<Placed> {
        if !self.open() {
            return Vec::new();
        }
        let named = named();
        let panel = self.rect(within);
        // The whole window under it, so a click anywhere else shuts the menu
        // rather than reaching what it is covering. `.catcher { inset: 0 }`.
        let mut placed = vec![
            named.at("elsewhere", within, 40),
            Placed {
                name: named.whole(),
                rect: panel,
                depth: 41,
            },
        ];
        for (item, rect) in self.rows(within) {
            if !item.pressable() {
                continue;
            }
            placed.push(named.at(&item.id, rect, 42));
            if item.inline && self.armed.as_deref() == Some(item.id.as_str()) {
                for (child, at) in item
                    .children
                    .iter()
                    .zip(self.inline_rects(rect, &item.children))
                {
                    placed.push(named.at(&child.id, at, 43));
                }
            }
            if !item.inline
                && !item.children.is_empty()
                && self.nested.as_deref() == Some(item.id.as_str())
            {
                let nest = self.nest_rect(rect, item.children.len(), within);
                let style = self.style.nested();
                placed.push(named.at(&format!("{}/nest", item.id), nest, 43));
                for (at, child) in item.children.iter().enumerate() {
                    placed.push(named.at(
                        &child.id,
                        Rect::new(
                            nest.x + style.padding,
                            nest.y + style.padding + at as f32 * style.item,
                            nest.width - style.padding * 2.0,
                            style.item,
                        ),
                        44,
                    ));
                }
            }
        }
        placed
    }

    /// Applies a frame's input. Answers the item chosen, with what it is about.
    pub fn react(&mut self, input: &Input) -> Option<(String, String)> {
        if !self.open() {
            return None;
        }
        if input.struck(Key::Escape) {
            self.hide();
            return None;
        }
        // Hover opens a nested list and hover takes it away again, which is
        // what `onmouseenter`/`onmouseleave` on `.nest` do.
        self.nested = self.hovered_nest(input);

        let clicked = input.clicked()?.to_string();
        let Some(rest) = named().slug(&clicked) else {
            // A click on the panel itself but not on a row: it stays open,
            // because the reader is still choosing.
            if clicked != NAME {
                self.hide();
            }
            return None;
        };
        if rest == "elsewhere" {
            self.hide();
            return None;
        }
        if rest.ends_with("/nest") {
            return None;
        }
        // A row that asks a question arms instead of firing.
        if let Some(item) = self.items.iter().find(|item| item.id == rest)
            && item.inline
            && !item.children.is_empty()
        {
            self.armed = Some(item.id.clone());
            return None;
        }
        let known = self
            .items
            .iter()
            .flat_map(|item| std::iter::once(item).chain(item.children.iter()))
            .any(|item| item.pressable() && item.id == rest);
        if !known {
            return None;
        }
        let chosen = (self.about.clone(), rest.to_string());
        self.hide();
        Some(chosen)
    }

    /// Which nested list the pointer is keeping open: the parent row, the list
    /// itself, or one of its children.
    fn hovered_nest(&self, input: &Input) -> Option<String> {
        let over = named().hovered(input)?;
        self.items
            .iter()
            .filter(|item| !item.inline && !item.children.is_empty())
            .find(|item| {
                over == item.id
                    || over == format!("{}/nest", item.id)
                    || item.children.iter().any(|child| child.id == over)
            })
            .map(|item| item.id.clone())
    }

    pub fn draw(&self, into: &mut Canvas<'_>, within: Rect, input: &Input) {
        if !self.open() {
            return;
        }
        let panel = self.rect(within);
        let rows: Vec<(Item, Rect)> = self
            .rows(within)
            .into_iter()
            .map(|(item, rect)| (item.clone(), rect))
            .collect();
        self.panel(into, panel, self.style);
        for (item, rect) in &rows {
            if item.rule {
                // `height: 1px; margin: 4px 6px`.
                into.scene.fill(
                    rect.x + 6.0,
                    rect.y + 4.0,
                    rect.width - 12.0,
                    1.0,
                    into.palette.rule,
                );
                continue;
            }
            if item.inline && self.armed.as_deref() == Some(item.id.as_str()) {
                self.armed_row(into, item, *rect, input);
                continue;
            }
            self.row(into, item, *rect, self.style, input);
            if !item.inline && !item.children.is_empty() {
                // The chevron that says there is more this way.
                let glyphs = into.painter.run(
                    into.fonts,
                    matterless_layout::marks::NEXT,
                    rect.right() - self.style.pad_x - 6.0,
                    rect.y + (rect.height - self.style.size * 1.4) / 2.0,
                    Run::mark(self.style.size - 1.0),
                );
                into.scene
                    .glyphs(glyphs, into.palette.faint, into.palette.faint);
            }
        }
        // The nested list over everything, since it hangs outside the panel.
        for (item, rect) in &rows {
            if item.inline || item.children.is_empty() || self.nested.as_deref() != Some(&item.id) {
                continue;
            }
            let style = self.style.nested();
            let nest = self.nest_rect(*rect, item.children.len(), within);
            self.panel(into, nest, style);
            for (at, child) in item.children.iter().enumerate() {
                self.row(
                    into,
                    child,
                    Rect::new(
                        nest.x + style.padding,
                        nest.y + style.padding + at as f32 * style.item,
                        nest.width - style.padding * 2.0,
                        style.item,
                    ),
                    style,
                    input,
                );
            }
        }
    }

    /// The floating ground and the hairline round it.
    fn panel(&self, into: &mut Canvas<'_>, rect: Rect, style: Style) {
        into.scene.floating(
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            into.palette.rule,
            style.corner,
            style.drop,
        );
        // `border: 1px solid var(--rule)` drawn as the ground inset by one,
        // which is the only way a single quad has an edge.
        into.scene.rounded(
            rect.x + 1.0,
            rect.y + 1.0,
            rect.width - 2.0,
            rect.height - 2.0,
            into.palette.surface,
            style.corner - 1.0,
        );
    }

    fn row(&self, into: &mut Canvas<'_>, item: &Item, rect: Rect, style: Style, input: &Input) {
        if named().under(input, &item.id) {
            into.scene.rounded(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                into.palette.ground,
                style.item_corner,
            );
        }
        let middle = rect.y + (rect.height - style.size * 1.4) / 2.0;
        if style.glyphs && !item.glyph.is_empty() {
            // 16px wide and centred in it, in the faint ink the app gives it.
            let glyphs = into.painter.run(
                into.fonts,
                &item.glyph,
                rect.x + style.pad_x,
                middle - 3.0,
                // Bigger than the words beside it, and drawn down from twice
                // its size: a mark is a picture with detail inside it, and at
                // the size of a word the font hints that detail into stems
                // too hard to read.
                Run::mark(14.0),
            );
            into.scene
                .glyphs(glyphs, into.palette.soft, into.palette.faint);
        }
        let ink = match item.tint {
            Tint::Ink => into.palette.ink,
            Tint::Flag => into.palette.flag,
            Tint::Danger => into.palette.danger,
        };
        let glyphs = into.painter.run(
            into.fonts,
            &item.label,
            rect.x + style.text_x(),
            middle,
            Run::label(rect.width - style.text_x()).sized(style.size),
        );
        into.scene.glyphs(glyphs, ink, into.palette.faint);
    }

    /// A row that has been pressed and is now asking: the question on the left
    /// and the answers beside it.
    fn armed_row(&self, into: &mut Canvas<'_>, item: &Item, rect: Rect, input: &Input) {
        let style = self.style;
        let middle = rect.y + (rect.height - style.size * 1.4) / 2.0;
        let asked = into.painter.run(
            into.fonts,
            &asking(&item.id),
            rect.x + style.pad_x,
            middle,
            Run::label(f32::MAX).sized(style.size),
        );
        into.scene
            .glyphs(asked, into.palette.soft, into.palette.faint);
        for (child, at) in item
            .children
            .iter()
            .zip(self.inline_rects(rect, &item.children))
        {
            let under = named().under(input, &child.id);
            into.scene.rounded(
                at.x,
                at.y,
                at.width,
                at.height,
                if under {
                    into.palette.rule
                } else {
                    into.palette.raised
                },
                4.0,
            );
            let ink = match child.tint {
                Tint::Danger => into.palette.danger,
                Tint::Flag => into.palette.flag,
                Tint::Ink => into.palette.ink,
            };
            let glyphs = into.painter.run(
                into.fonts,
                &child.label,
                at.x + 5.0,
                at.y + (at.height - style.size * 1.3) / 2.0,
                Run::label(at.width - 6.0).sized(style.size - 1.0),
            );
            into.scene.glyphs(glyphs, ink, into.palette.faint);
        }
    }
}

/// What an armed row asks. Both of the app's say what the answers are for.
fn asking(id: &str) -> String {
    match id {
        "delete" => "Delete?".to_string(),
        "remind" => "Remind me in".to_string(),
        _ => String::new(),
    }
}

/// Roughly how wide a label is, for splitting an armed row.
///
/// An estimate rather than a shaping pass: this decides where a handful of
/// small buttons start, and being a few pixels out costs nothing that shaping
/// the string every frame would buy back.
fn label_width(style: &Style, text: &str) -> f32 {
    text.chars().count() as f32 * style.size * 0.52
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> Rect {
        Rect::new(0.0, 0.0, 900.0, 600.0)
    }

    fn items() -> Vec<Item> {
        vec![
            Item::new("unread", "Mark as Unread").marked(matterless_layout::marks::UNREAD),
            Item::new("mute", "Mute Channel").marked(matterless_layout::marks::BELL_OFF),
            Item::rule(),
            Item::new("leave", "Leave Channel").tinted(Tint::Flag),
        ]
    }

    fn menu() -> Menu {
        let mut menu = Menu::default();
        menu.show("c1", Anchor::At(100.0, 100.0), Style::channel(), items());
        menu
    }

    /// A menu near an edge is pushed back on, not flipped: the app clamps, and
    /// a menu that jumped to the other side of the pointer would move out from
    /// under the hand that opened it.
    #[test]
    fn a_menu_at_an_edge_stays_on_the_window() {
        let mut menu = menu();
        for at in [(880.0, 580.0), (-40.0, -40.0), (450.0, 595.0)] {
            menu.show("c1", Anchor::At(at.0, at.1), Style::channel(), items());
            let rect = menu.rect(window());
            assert!(rect.x >= window().x, "{at:?} ran off the left");
            assert!(rect.right() <= window().right(), "{at:?} ran off the right");
            assert!(rect.y >= window().y, "{at:?} ran off the top");
            assert!(
                rect.bottom() <= window().bottom(),
                "{at:?} ran off the bottom"
            );
        }
    }

    /// Under a button means right-aligned with it, which is `right: 0`.
    #[test]
    fn a_menu_under_a_button_lines_up_with_its_right_edge() {
        let mut menu = Menu::default();
        let button = Rect::new(600.0, 200.0, 24.0, 24.0);
        menu.show("p1", Anchor::Under(button), Style::message(), items());
        let rect = menu.rect(window());
        assert_eq!(rect.right(), button.right());
        assert!(rect.y >= button.bottom());
    }

    /// A rule is not something to land on: a click has to fall through it to
    /// the panel rather than choosing a row with no id.
    #[test]
    fn a_rule_is_not_a_target() {
        let menu = menu();
        let names: Vec<String> = menu
            .boxes(window())
            .into_iter()
            .map(|placed| placed.name)
            .collect();
        assert_eq!(
            names
                .iter()
                .filter(|name| name.starts_with("menu/"))
                .count(),
            4,
            "three rows and the catcher, and no rule"
        );
        assert!(names.contains(&"menu/leave".to_string()));
    }

    /// The catcher covers the whole window, or a click in a corner would reach
    /// whatever is under it instead of shutting the menu.
    #[test]
    fn a_click_anywhere_else_shuts_it() {
        let menu = menu();
        let catcher = &menu.boxes(window())[0];
        assert_eq!(catcher.name, "menu/elsewhere");
        assert_eq!(catcher.rect, window());
    }

    /// A shut menu offers nothing and places nothing.
    #[test]
    fn a_shut_menu_is_not_there() {
        let mut menu = menu();
        menu.hide();
        assert!(!menu.open());
        assert!(menu.boxes(window()).is_empty());
    }

    /// Pressing a row that asks a question does not fire it: that is the whole
    /// point of asking, and a delete that went through on the first press
    /// would be the one thing the confirmation exists to prevent.
    #[test]
    fn a_row_that_asks_first_does_not_fire_on_the_first_press() {
        let mut menu = Menu::default();
        menu.show(
            "p1",
            Anchor::At(100.0, 100.0),
            Style::message(),
            vec![Item::new("delete", "Delete\u{2026}").asks(vec![
                Item::new("delete.now", "Delete").tinted(Tint::Danger),
                Item::new("delete.keep", "Keep"),
            ])],
        );
        let mut input = Input::default();
        let boxes = menu.boxes(window());
        let row = boxes
            .iter()
            .find(|placed| placed.name == "menu/delete")
            .expect("the row");
        press(&mut input, &boxes, row.rect);
        assert_eq!(menu.react(&input), None, "it asked instead of firing");
        assert!(menu.open(), "and stayed open to be answered");

        // The answer is there now, and it is the answer that fires.
        let boxes = menu.boxes(window());
        let confirm = boxes
            .iter()
            .find(|placed| placed.name == "menu/delete.now")
            .expect("the confirmation");
        let mut input = Input::default();
        press(&mut input, &boxes, confirm.rect);
        assert_eq!(
            menu.react(&input),
            Some(("p1".to_string(), "delete.now".to_string()))
        );
        assert!(!menu.open());
    }

    /// A nested list is only there while the pointer is on its way to it.
    #[test]
    fn a_nested_list_follows_the_pointer() {
        let mut menu = Menu::default();
        menu.show(
            "c1",
            Anchor::At(100.0, 100.0),
            Style::channel(),
            vec![
                Item::new("move", "Move to\u{2026}").nests(vec![
                    Item::new("move.a", "Work"),
                    Item::new("move.b", "Play"),
                ]),
                Item::new("mute", "Mute Channel"),
            ],
        );
        assert!(
            !menu
                .boxes(window())
                .iter()
                .any(|placed| placed.name == "menu/move.a"),
            "shut until hovered"
        );

        let mut input = Input::default();
        let boxes = menu.boxes(window());
        let row = boxes
            .iter()
            .find(|placed| placed.name == "menu/move")
            .expect("the row");
        hover(&mut input, &boxes, row.rect);
        menu.react(&input);
        let open = menu.boxes(window());
        assert!(open.iter().any(|placed| placed.name == "menu/move.a"));

        // Away from it entirely, and it closes again.
        let mut input = Input::default();
        let other = open
            .iter()
            .find(|placed| placed.name == "menu/mute")
            .expect("the other row");
        hover(&mut input, &open, other.rect);
        menu.react(&input);
        assert!(
            !menu
                .boxes(window())
                .iter()
                .any(|placed| placed.name == "menu/move.a")
        );
    }

    fn hover(input: &mut Input, boxes: &[Placed], rect: Rect) {
        input.apply(
            matterless_ui::input::Event::PointerMoved {
                x: rect.x + rect.width / 2.0,
                y: rect.y + rect.height / 2.0,
            },
            boxes,
        );
    }

    fn press(input: &mut Input, boxes: &[Placed], rect: Rect) {
        hover(input, boxes, rect);
        input.apply(matterless_ui::input::Event::PointerPressed, boxes);
        input.apply(matterless_ui::input::Event::PointerReleased, boxes);
    }
}
