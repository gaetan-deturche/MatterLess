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

/// How firmly letters' edges are drawn: the `k` of DirectWrite's enhanced
/// contrast, `c(k+1)/(ck+1)`, from none to its strongest. Text is blended as
/// if in linear light, which is right but softens edges; this gives them back,
/// to taste.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Contrast(u8);

impl Default for Contrast {
    fn default() -> Self {
        Contrast(10)
    }
}

impl Contrast {
    /// What this is called in the settings table.
    pub const SETTING: &'static str = "text-contrast";

    /// Between nought and one, in twentieths: finer than anybody can tell.
    pub fn new(amount: f32) -> Self {
        Contrast((amount.clamp(0.0, 1.0) * 20.0).round() as u8)
    }

    pub fn amount(&self) -> f32 {
        f32::from(self.0) / 20.0
    }

    pub fn stored(&self) -> String {
        format!("{:.2}", self.amount())
    }

    /// Reads one back; anything that is not a number is the default.
    pub fn read(said: Option<&str>) -> Self {
        said.and_then(|said| said.trim().parse::<f32>().ok())
            .filter(|amount| amount.is_finite())
            .map(Contrast::new)
            .unwrap_or_default()
    }

    /// The amount in words, beside the heading.
    fn said(&self) -> &'static str {
        match self.0 {
            0 => "None",
            1..=7 => "Low",
            8..=12 => "Medium",
            13..=19 => "High",
            _ => "Max",
        }
    }
}

/// Every team's channels in one list, or one team's at a time with the rail
/// switching between them, as the official client does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Teams {
    #[default]
    All,
    One,
}

impl Teams {
    /// What this is called in the settings table.
    pub const SETTING: &'static str = "teams-shown";

    pub fn stored(&self) -> &'static str {
        self.slug()
    }

    /// Reads one back; anything else is the default.
    pub fn read(said: Option<&str>) -> Self {
        Self::all()
            .into_iter()
            .find(|one| Some(one.slug()) == said)
            .unwrap_or_default()
    }

    fn title(&self) -> &'static str {
        match self {
            Teams::All => "All together",
            Teams::One => "One at a time",
        }
    }

    fn slug(&self) -> &'static str {
        match self {
            Teams::All => "teams-all",
            Teams::One => "teams-one",
        }
    }

    fn all() -> [Teams; 2] {
        [Teams::All, Teams::One]
    }
}

/// Which unread channels carry a count in the sidebar: every one, or only
/// those that mention the reader -- the official client's way, where an
/// unread channel is only bold, and a list of pills reads as notifications
/// everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Counts {
    #[default]
    All,
    Mentions,
}

impl Counts {
    /// What this is called in the settings table.
    pub const SETTING: &'static str = "unread-counts";

    pub fn stored(&self) -> &'static str {
        self.slug()
    }

    /// Reads one back; anything else is the default.
    pub fn read(said: Option<&str>) -> Self {
        Self::all()
            .into_iter()
            .find(|one| Some(one.slug()) == said)
            .unwrap_or_default()
    }

    fn title(&self) -> &'static str {
        match self {
            Counts::All => "All",
            Counts::Mentions => "Mentions",
        }
    }

    fn slug(&self) -> &'static str {
        match self {
            Counts::All => "counts-all",
            Counts::Mentions => "counts-mentions",
        }
    }

    fn all() -> [Counts; 2] {
        [Counts::All, Counts::Mentions]
    }
}

/// Light text on a dark ground, or dark on light.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

impl Theme {
    /// What this is called in the settings table.
    pub const SETTING: &'static str = "theme";

    pub fn stored(&self) -> &'static str {
        self.slug()
    }

    /// Reads one back; anything else is the default.
    pub fn read(said: Option<&str>) -> Self {
        Self::all()
            .into_iter()
            .find(|one| Some(one.slug()) == said)
            .unwrap_or_default()
    }

    /// What Windows is set to for apps, for a first start with nothing chosen
    /// here yet. Dark wherever there is no such setting to read.
    pub fn system() -> Self {
        #[cfg(windows)]
        {
            use windows::Win32::System::Registry::{
                HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW,
            };
            use windows::core::w;
            let mut light = 0u32;
            let mut size = std::mem::size_of::<u32>() as u32;
            let read = unsafe {
                RegGetValueW(
                    HKEY_CURRENT_USER,
                    w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
                    w!("AppsUseLightTheme"),
                    RRF_RT_REG_DWORD,
                    None,
                    Some((&raw mut light).cast()),
                    Some(&raw mut size),
                )
            };
            if read.is_ok() && light != 0 {
                return Theme::Light;
            }
        }
        Theme::Dark
    }

    /// The colours this theme draws in.
    pub fn palette(&self) -> matterless_paint::Palette {
        match self {
            Theme::Dark => matterless_paint::Palette::default(),
            Theme::Light => matterless_paint::Palette::light(),
        }
    }

    fn title(&self) -> &'static str {
        match self {
            Theme::Dark => "Dark",
            Theme::Light => "Light",
        }
    }

    fn slug(&self) -> &'static str {
        match self {
            Theme::Dark => "theme-dark",
            Theme::Light => "theme-light",
        }
    }

    fn all() -> [Theme; 2] {
        [Theme::Dark, Theme::Light]
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

/// Whether a mention this client reads in a post counts when the server's own
/// list of who it mentions leaves the reader off: a bot signing a commit with
/// the reader's bare name, or naming them in a card's attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Words(pub bool);

impl Default for Words {
    fn default() -> Self {
        Words(true)
    }
}

impl Words {
    /// What this is called in the settings table.
    pub const SETTING: &'static str = "mention-words";

    pub fn stored(&self) -> &'static str {
        match self.0 {
            true => "on",
            false => "off",
        }
    }

    /// Reads one back; anything but "off" is on.
    pub fn read(said: Option<&str>) -> Self {
        Words(said != Some("off"))
    }
}

/// Where a setting on another tab is: nowhere, so nothing is drawn there and
/// nothing can be pressed.
const NONE: Rect = Rect {
    x: 0.0,
    y: 0.0,
    width: 0.0,
    height: 0.0,
};

/// The panel's tabs, one subject each: everything on one card had become a
/// list too long to find anything in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Appearance,
    Sidebar,
    Notifications,
    General,
}

impl Tab {
    fn all() -> [Tab; 4] {
        [
            Tab::Appearance,
            Tab::Sidebar,
            Tab::Notifications,
            Tab::General,
        ]
    }

    fn title(self) -> &'static str {
        match self {
            Tab::Appearance => "Appearance",
            Tab::Sidebar => "Sidebar",
            Tab::Notifications => "Notifications",
            Tab::General => "General",
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Tab::Appearance => "tab-appearance",
            Tab::Sidebar => "tab-sidebar",
            Tab::Notifications => "tab-notifications",
            Tab::General => "tab-general",
        }
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
    /// The tab on show, and the row of them under the title. Everything that
    /// belongs to another tab is laid out as nothing.
    tab: Tab,
    tabs: Vec<(Tab, Rect)>,
    /// The heading over the group of choices.
    group: Rect,
    choices: Vec<Choice>,
    /// The heading over the text contrast, and its steps.
    contrast: Rect,
    /// The slider's track: the whole row it can be grabbed on.
    slider: Rect,
    /// The heading over the theme, and its two buttons.
    theme: Rect,
    themes: Laid,
    /// The heading over gathering unread channels, and its two buttons.
    unreads: Rect,
    groupings: Laid,
    /// Beside it, the heading over which unread channels carry a count.
    counts: Rect,
    countings: Laid,
    /// The heading over how teams are listed, and its two buttons.
    teams: Rect,
    showings: Laid,
    /// The heading over how much opened pictures keep, and its buttons.
    kept: Rect,
    sizes: Laid,
    /// The heading over starting with Windows, and its two buttons.
    starting: Rect,
    starts: Laid,
    /// The heading over counting the mentions the words show, its two
    /// buttons, and the line under them saying what it means.
    words: Rect,
    wordings: Laid,
    words_said: Rect,
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

impl Card {
    /// Where a tab's name is, which is what is pressed to show it.
    #[cfg(test)]
    fn tab_rect(&self, tab: Tab) -> Option<Rect> {
        self.tabs
            .iter()
            .find(|(one, _)| *one == tab)
            .map(|(_, at)| *at)
    }

    /// Every row of buttons, in one place: what is drawn and what answers a
    /// press come from the same list, so a row cannot be one without the other.
    fn rows(&self) -> [&Laid; 9] {
        [
            &self.wordings,
            &self.themes,
            &self.groupings,
            &self.countings,
            &self.showings,
            &self.sizes,
            &self.starts,
            &self.look,
            &self.foot,
        ]
    }
}

/// The panel, when it is up.
#[derive(Debug, Default)]
pub struct Settings {
    shown: bool,
    pub text: Text,
    pub contrast: Contrast,
    pub theme: Theme,
    pub kept: Kept,
    /// Whether this client starts when the reader signs in to Windows. Kept in
    /// the registry, not here: the window reads it in as the panel opens.
    pub startup: bool,
    /// Whether unread channels are gathered at the top of the sidebar.
    /// Mattermost's own setting, kept on the server and shared with the
    /// reader's other clients: the window reads it in as the panel opens.
    pub unreads: bool,
    pub counts: Counts,
    pub teams: Teams,
    pub words: Words,
    tab: Tab,
    /// Whether a look for a newer build is out, so it is not asked for twice
    /// and the button can say what it is doing.
    looking: bool,
    /// What that look came back with, in the reader's words rather than the
    /// program's.
    said: String,
    /// A newer build there is to install, found by that look or already on
    /// offer, and whether it is being installed. The button installs it then,
    /// rather than sending the reader to the strip along the top.
    ready: Option<String>,
    installing: bool,
    placed: Option<Card>,
}

/// What the window should do about what just happened in here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Did {
    /// Text is to be drawn the other way, and the caller has to say so to
    /// whatever holds the glyphs.
    Text(Text),
    /// Letters' edges are to be firmed up this much.
    Contrast(Contrast),
    /// Everything is to be drawn in this theme.
    Theme(Theme),
    /// Opened pictures may keep this much on disk from now on.
    Kept(Kept),
    /// Gather unread channels at the top of the sidebar, or leave them where
    /// they are.
    Unreads(bool),
    /// Count these unread channels in the sidebar from now on.
    Counts(Counts),
    /// Count a mention the words show and the server does not, or stop.
    Words(Words),
    /// Another tab is on show: nothing to do but measure the card again.
    Tab,
    /// List the teams this way from now on.
    Teams(Teams),
    /// Start with Windows, or stop.
    Startup(bool),
    /// Go and see whether there is a newer build.
    ///
    /// Asked for rather than done here: this widget knows where its buttons
    /// are and nothing about the network, which is the window's business.
    Look,
    /// Install the newer build that was found, and restart into it.
    Install,
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

    /// The card, grown to whatever its own words come out as, showing one tab.
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

        let contrast_tall = tall(fonts, CONTRAST_HEADING, inner, GROUP_SIZE);
        let theme_tall = tall(fonts, THEME_HEADING, inner, GROUP_SIZE);
        let unreads_tall = tall(fonts, UNREADS_HEADING, inner, GROUP_SIZE);
        let teams_tall = tall(fonts, TEAMS_HEADING, inner, GROUP_SIZE);
        let kept_tall = tall(fonts, KEPT_HEADING, inner, GROUP_SIZE);
        let starting_tall = tall(fonts, STARTUP_HEADING, inner, GROUP_SIZE);
        let words_tall = tall(fonts, WORDS_HEADING, inner, GROUP_SIZE);
        let words_said_tall = tall(fonts, WORDS_SAID, inner, SAID_SIZE);

        // The build this is, and what the last look for a newer one said. The
        // line is only measured into the card when there is something on it:
        // an empty row of air under a button reads as something missing.
        let about_tall = tall(fonts, "About", inner, GROUP_SIZE);
        let build_tall = tall(fonts, &self.build(), inner, LABEL_SIZE);
        let band_tall = BUTTON_HEIGHT.max(build_tall);
        let said_tall = match self.said.is_empty() {
            true => 0.0,
            false => ROW_GAP + tall(fonts, &self.said, inner, SAID_SIZE),
        };

        // A heading and the buttons under it.
        let section = |heading: f32| heading + ROW_GAP + BUTTON_HEIGHT;
        // Every tab measured and the card the height of the tallest: a card
        // that changed height with its tab moved the tabs out from under the
        // pointer that was choosing between them.
        let content = |tab: Tab| match tab {
            Tab::Appearance => {
                heading + ROW_GAP + rows + GAP + section(contrast_tall) + GAP + section(theme_tall)
            }
            Tab::Sidebar => section(unreads_tall) + GAP + section(teams_tall),
            Tab::Notifications => section(words_tall) + ROW_GAP + words_said_tall,
            Tab::General => {
                section(kept_tall)
                    + GAP
                    + section(starting_tall)
                    + GAP
                    + about_tall
                    + ROW_GAP
                    + band_tall
                    + said_tall
            }
        };
        let tallest = Tab::all().into_iter().map(content).fold(0.0, f32::max);
        let height = PAD + head + GAP + TAB_HEIGHT + GAP + tallest + GAP + BUTTON_HEIGHT + PAD;
        let rect = Rect::new(
            window.x + (window.width - width) / 2.0,
            window.y + ((window.height - height) / 2.0).max(PAD),
            width,
            height,
        );

        let buttons =
            |fonts: &mut Fonts, at: Rect, offered: &[(&'static str, &'static str, bool)]| {
                offered
                    .iter()
                    .fold(
                        Row::new(NAME, at)
                            .against(matterless_widgets::Against::Left)
                            .pad(PAD)
                            .depth(63),
                        |row, (slug, title, on)| {
                            row.button(match on {
                                true => Button::primary(slug, *title),
                                false => Button::plain(slug, *title),
                            })
                        },
                    )
                    .measure(fonts)
            };
        let nothing = |fonts: &mut Fonts| Row::new(NAME, NONE).measure(fonts);
        let heading_at = |y: f32, tall: f32| Rect::new(rect.x + PAD, y, inner, tall);
        let under = |at: Rect| Rect::new(rect.x, at.bottom() + ROW_GAP, rect.width, BUTTON_HEIGHT);
        let after = |at: Rect| at.bottom() + ROW_GAP + BUTTON_HEIGHT + GAP;

        // Tabs drawn as tabs, their names along a hairline: as a row of
        // buttons they read as one more setting.
        let tab = self.tab;
        let strip = rect.y + PAD + head + GAP;
        let mut x = rect.x + PAD;
        let mut tabs = Vec::with_capacity(Tab::all().len());
        for one in Tab::all() {
            let wide =
                matterless_layout::extent_of(fonts, one.title(), f32::MAX, style(LABEL_SIZE)).width;
            tabs.push((one, Rect::new(x, strip, wide, TAB_HEIGHT)));
            x += wide + TAB_GAP;
        }
        let top = strip + TAB_HEIGHT + GAP;

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

        let mut card = Card {
            rect,
            tab,
            tabs,
            group: NONE,
            choices: Vec::new(),
            contrast: NONE,
            slider: NONE,
            theme: NONE,
            themes: nothing(fonts),
            unreads: NONE,
            groupings: nothing(fonts),
            counts: NONE,
            countings: nothing(fonts),
            teams: NONE,
            showings: nothing(fonts),
            kept: NONE,
            sizes: nothing(fonts),
            starting: NONE,
            starts: nothing(fonts),
            words: NONE,
            wordings: nothing(fonts),
            words_said: NONE,
            about: NONE,
            build: NONE,
            said: None,
            look: nothing(fonts),
            foot,
        };

        match tab {
            Tab::Appearance => {
                card.group = heading_at(top, heading);
                let mut y = card.group.bottom() + ROW_GAP;
                for (which, title, said) in sizes {
                    let row = Rect::new(rect.x + PAD, y, inner, ROW_PAD_Y * 2.0 + title + said);
                    let text_x = row.x + ROW_PAD_X + DOT_COLUMN;
                    card.choices.push(Choice {
                        which,
                        rect: row,
                        title: Rect::new(text_x, row.y + ROW_PAD_Y, wrap, title),
                        said: Rect::new(text_x, row.y + ROW_PAD_Y + title, wrap, said),
                    });
                    y = row.bottom() + ROW_GAP;
                }
                card.contrast = heading_at(y - ROW_GAP + GAP, contrast_tall);
                card.slider = Rect::new(
                    rect.x + PAD,
                    card.contrast.bottom() + ROW_GAP,
                    inner,
                    BUTTON_HEIGHT,
                );
                card.theme = heading_at(after(card.contrast), theme_tall);
                let offered: Vec<_> = Theme::all()
                    .into_iter()
                    .map(|one| (one.slug(), one.title(), one == self.theme))
                    .collect();
                card.themes = buttons(fonts, under(card.theme), &offered);
            }
            // Two questions about unread channels side by side, each in half
            // the card, and the teams under them.
            Tab::Sidebar => {
                let half = inner / 2.0;
                card.unreads = Rect::new(rect.x + PAD, top, half - GAP, unreads_tall);
                card.counts = Rect::new(rect.x + PAD + half, top, half, unreads_tall);
                card.groupings = buttons(
                    fonts,
                    Rect::new(
                        rect.x,
                        card.unreads.bottom() + ROW_GAP,
                        half + PAD,
                        BUTTON_HEIGHT,
                    ),
                    &[
                        ("unreads-on", "On", self.unreads),
                        ("unreads-off", "Off", !self.unreads),
                    ],
                );
                let offered: Vec<_> = Counts::all()
                    .into_iter()
                    .map(|one| (one.slug(), one.title(), one == self.counts))
                    .collect();
                card.countings = buttons(
                    fonts,
                    Rect::new(
                        card.counts.x - PAD,
                        card.unreads.bottom() + ROW_GAP,
                        half + PAD,
                        BUTTON_HEIGHT,
                    ),
                    &offered,
                );
                card.teams = heading_at(after(card.unreads), teams_tall);
                let offered: Vec<_> = Teams::all()
                    .into_iter()
                    .map(|one| (one.slug(), one.title(), one == self.teams))
                    .collect();
                card.showings = buttons(fonts, under(card.teams), &offered);
            }
            Tab::Notifications => {
                card.words = heading_at(top, words_tall);
                card.wordings = buttons(
                    fonts,
                    under(card.words),
                    &[
                        ("words-on", "On", self.words.0),
                        ("words-off", "Off", !self.words.0),
                    ],
                );
                card.words_said = Rect::new(
                    rect.x + PAD,
                    card.words.bottom() + ROW_GAP + BUTTON_HEIGHT + ROW_GAP,
                    inner,
                    words_said_tall,
                );
            }
            Tab::General => {
                card.kept = heading_at(top, kept_tall);
                let offered: Vec<_> = Kept::all()
                    .into_iter()
                    .map(|one| (one.slug(), one.title(), one == self.kept))
                    .collect();
                card.sizes = buttons(fonts, under(card.kept), &offered);
                card.starting = heading_at(after(card.kept), starting_tall);
                card.starts = buttons(
                    fonts,
                    under(card.starting),
                    &[
                        ("startup-on", "On", self.startup),
                        ("startup-off", "Off", !self.startup),
                    ],
                );
                card.about = heading_at(after(card.starting), about_tall);
                let band = Rect::new(rect.x, card.about.bottom() + ROW_GAP, rect.width, band_tall);
                // The name of the build on the left, the button that goes
                // looking on the right, on one line: they are the same
                // subject, and a button under the sentence it belongs to reads
                // as a second thing.
                card.look = Row::new(NAME, band)
                    .pad(PAD)
                    .depth(63)
                    .enabled(!self.looking && !self.installing)
                    .button(match self.ready.is_some() {
                        true => Button::primary("install", self.installing_label()),
                        false => Button::plain("look", self.looking_label()),
                    })
                    .measure(fonts);
                card.build = Rect::new(
                    band.x + PAD,
                    band.y + (band.height - build_tall) / 2.0,
                    (card.look.spare() - PAD).max(0.0),
                    build_tall,
                );
                card.said = (!self.said.is_empty()).then(|| {
                    Rect::new(
                        rect.x + PAD,
                        band.bottom() + ROW_GAP,
                        inner,
                        tall(fonts, &self.said, inner, SAID_SIZE),
                    )
                });
            }
        }
        card
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

    fn installing_label(&self) -> &'static str {
        match self.installing {
            true => "Installing\u{2026}",
            false => "Install and restart",
        }
    }

    /// There is a newer build to install, or no longer one. Set by the window
    /// from what a look found and from the offer on the strip.
    pub fn ready(&mut self, version: Option<&str>) {
        self.ready = version.map(str::to_string);
        self.installing = false;
    }

    /// The install did not work: said here, where it was asked for, and the
    /// button can be pressed again.
    pub fn install_failed(&mut self, why: &str) {
        self.installing = false;
        self.said = format!("It could not be installed: {why}");
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
        if card.tab == Tab::Appearance {
            placed.push(named().at("contrast", card.slider, 63));
        }
        for (one, at) in &card.tabs {
            placed.push(named().at(one.slug(), *at, 63));
        }
        for row in card.rows() {
            placed.extend(row.boxes());
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
        // The slider follows the pointer for as long as it is held, from the
        // press on: a click anywhere on the track goes there.
        if input.pressed().and_then(|name| named().slug(name)) == Some("contrast")
            && let (Some(card), Some((x, _))) = (self.laid(), input.pointer_at())
        {
            let (from, to) = slider_ends(card.slider);
            let asked = Contrast::new((x - from) / (to - from));
            if asked != self.contrast {
                self.contrast = asked;
                return Some(Did::Contrast(asked));
            }
            return None;
        }
        if self
            .laid()
            .and_then(|card| card.foot.clicked(input))
            .is_some()
        {
            self.hide();
            return Some(Did::Close);
        }
        let pressed = self
            .laid()
            .and_then(|card| card.look.clicked(input))
            .map(str::to_string);
        match pressed.as_deref() {
            // The panel says it is looking before anything has been asked:
            // the answer is a round trip away, and a button that sits there
            // unchanged reads as a button that missed.
            Some("look") => {
                self.looking();
                return Some(Did::Look);
            }
            // The same for installing, which then restarts the window.
            Some("install") => {
                self.installing = true;
                self.said = "Downloading the new build...".to_string();
                return Some(Did::Install);
            }
            _ => {}
        }

        if let Some(one) = input
            .clicked()
            .and_then(|name| named().slug(name))
            .and_then(|slug| Tab::all().into_iter().find(|one| one.slug() == slug))
        {
            self.tab = one;
            self.placed = None;
            return Some(Did::Tab);
        }
        if let Some(slug) = self.laid().and_then(|card| card.wordings.clicked(input)) {
            self.words = Words(slug == "words-on");
            self.placed = None;
            return Some(Did::Words(self.words));
        }
        if let Some(slug) = self.laid().and_then(|card| card.themes.clicked(input))
            && let Some(one) = Theme::all().into_iter().find(|one| one.slug() == slug)
        {
            self.theme = one;
            self.placed = None;
            return Some(Did::Theme(one));
        }
        if let Some(slug) = self.laid().and_then(|card| card.sizes.clicked(input))
            && let Some(one) = Kept::all().into_iter().find(|one| one.slug() == slug)
        {
            self.kept = one;
            // Measured again, so the chosen button is drawn as chosen.
            self.placed = None;
            return Some(Did::Kept(one));
        }
        if let Some(slug) = self.laid().and_then(|card| card.groupings.clicked(input)) {
            let on = slug == "unreads-on";
            self.unreads = on;
            self.placed = None;
            return Some(Did::Unreads(on));
        }
        if let Some(slug) = self.laid().and_then(|card| card.countings.clicked(input))
            && let Some(one) = Counts::all().into_iter().find(|one| one.slug() == slug)
        {
            self.counts = one;
            self.placed = None;
            return Some(Did::Counts(one));
        }
        if let Some(slug) = self.laid().and_then(|card| card.showings.clicked(input))
            && let Some(one) = Teams::all().into_iter().find(|one| one.slug() == slug)
        {
            self.teams = one;
            self.placed = None;
            return Some(Did::Teams(one));
        }
        if let Some(slug) = self.laid().and_then(|card| card.starts.clicked(input)) {
            let on = slug == "startup-on";
            // The window says what it ended up as; this is what was asked.
            self.startup = on;
            self.placed = None;
            return Some(Did::Startup(on));
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
                palette.backdrop(160),
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

            // The tabs: a hairline across the card, each name on it, and the
            // one on show in full ink with the signal under it.
            if let Some((_, first)) = card.tabs.first() {
                scene.fill(
                    card.rect.x + PAD,
                    first.bottom() - 1.0,
                    card.rect.width - PAD * 2.0,
                    1.0,
                    palette.rule,
                );
            }
            for (one, at) in &card.tabs {
                let shown = *one == card.tab;
                let ink = match shown || named().under(input, one.slug()) {
                    true => palette.ink,
                    false => palette.soft,
                };
                let glyphs = painter.run(
                    fonts,
                    one.title(),
                    at.x,
                    at.y + (at.height - TAB_LINE - LABEL_SIZE * 1.4) / 2.0,
                    Run::label(at.width + 1.0).sized(LABEL_SIZE),
                );
                scene.glyphs(glyphs, ink, palette.faint);
                if shown {
                    let [red, green, blue] = palette.signal;
                    scene.rounded(
                        at.x,
                        at.bottom() - TAB_LINE,
                        at.width,
                        TAB_LINE,
                        [red, green, blue, 255],
                        TAB_LINE / 2.0,
                    );
                }
            }

            // The faint headings over each setting, for the tab on show.
            let mut label = |text: &str, at: Rect, size: f32, ink: [u8; 3]| {
                let glyphs = painter.run(fonts, text, at.x, at.y, Run::label(at.width).sized(size));
                scene.glyphs(glyphs, ink, palette.faint);
            };
            match card.tab {
                Tab::Appearance => {
                    label("Text", card.group, GROUP_SIZE, palette.faint);
                    let heading = format!("{CONTRAST_HEADING} \u{b7} {}", self.contrast.said());
                    label(&heading, card.contrast, GROUP_SIZE, palette.faint);
                    label(THEME_HEADING, card.theme, GROUP_SIZE, palette.faint);
                }
                Tab::Sidebar => {
                    label(UNREADS_HEADING, card.unreads, GROUP_SIZE, palette.faint);
                    label(COUNTS_HEADING, card.counts, GROUP_SIZE, palette.faint);
                    label(TEAMS_HEADING, card.teams, GROUP_SIZE, palette.faint);
                }
                Tab::Notifications => {
                    label(WORDS_HEADING, card.words, GROUP_SIZE, palette.faint);
                    label(WORDS_SAID, card.words_said, SAID_SIZE, palette.faint);
                }
                Tab::General => {
                    label(KEPT_HEADING, card.kept, GROUP_SIZE, palette.faint);
                    label(STARTUP_HEADING, card.starting, GROUP_SIZE, palette.faint);
                    label("About", card.about, GROUP_SIZE, palette.faint);
                    label(&self.build(), card.build, LABEL_SIZE, palette.soft);
                    if let Some(said) = card.said {
                        label(&self.said, said, SAID_SIZE, palette.faint);
                    }
                }
            }

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

            // The slider: a groove, the part of it below the amount lit, a
            // tick at each quarter, and a knob where it stands.
            if card.tab == Tab::Appearance {
                let (from, to) = slider_ends(card.slider);
                let middle = card.slider.y + card.slider.height / 2.0;
                let at = from + (to - from) * self.contrast.amount();
                let held = input.pressed() == Some(named().of("contrast").as_str());
                let lit = held || input.hovered() == Some(named().of("contrast").as_str());
                let groove = palette.raised;
                let [red, green, blue] = palette.signal;
                scene.rounded(from, middle - 2.0, to - from, 4.0, groove, 2.0);
                scene.rounded(
                    from,
                    middle - 2.0,
                    at - from,
                    4.0,
                    [red, green, blue, 255],
                    2.0,
                );
                for quarter in 0..5 {
                    let x = from + (to - from) * quarter as f32 / 4.0;
                    scene.rounded(x - 1.0, middle + 6.0, 2.0, 4.0, palette.rule, 1.0);
                }
                let knob = if lit { SLIDER_KNOB + 2.0 } else { SLIDER_KNOB };
                scene.rounded(
                    at - knob / 2.0 - 2.0,
                    middle - knob / 2.0 - 2.0,
                    knob + 4.0,
                    knob + 4.0,
                    palette.surface,
                    (knob + 4.0) / 2.0,
                );
                scene.rounded(
                    at - knob / 2.0,
                    middle - knob / 2.0,
                    knob,
                    knob,
                    [red, green, blue, 255],
                    knob / 2.0,
                );
            }
        }
        for row in card.rows() {
            row.draw(into, input);
        }
    }
}

/// The strip of tabs: how tall, the room between two names, and the line
/// under the one on show.
const TAB_HEIGHT: f32 = 30.0;
const TAB_GAP: f32 = 24.0;
const TAB_LINE: f32 = 2.0;

/// The heading over how firmly letters are drawn.
const CONTRAST_HEADING: &str = "Text contrast";

/// The slider's knob, and how far its ends sit in from the track's.
const SLIDER_KNOB: f32 = 14.0;

/// Where the slider's knob can go: the track less half a knob each end, so it
/// never hangs off either.
fn slider_ends(track: Rect) -> (f32, f32) {
    let inset = SLIDER_KNOB / 2.0 + 2.0;
    (
        track.x + inset,
        (track.right() - inset).max(track.x + inset + 1.0),
    )
}

/// The heading over the theme.
const THEME_HEADING: &str = "Theme";

/// The heading over how much disk opened pictures may keep.
const KEPT_HEADING: &str = "Opened pictures kept on disk";

/// The heading over gathering unread channels, in Mattermost's own words.
const UNREADS_HEADING: &str = "Group unread channels";
const COUNTS_HEADING: &str = "Unread counts";

/// The heading over how teams are listed.
const TEAMS_HEADING: &str = "Teams in the sidebar";

/// The heading over starting when the reader signs in to Windows.
const STARTUP_HEADING: &str = "Start with Windows";

/// The heading over counting what the words say, and what it means.
const WORDS_HEADING: &str = "Mentions the server does not count";
const WORDS_SAID: &str = "Your mention keywords, or an @ in a bot's card, counted as mentions \
even when the server leaves you out of the post's mentions.";

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

    /// The panel open on one tab.
    fn opened_on(tab: Tab) -> (Settings, Fonts) {
        let (mut settings, mut fonts) = shown();
        settings.tab = tab;
        settings.measure(&mut fonts, window());
        (settings, fonts)
    }

    #[test]
    fn contrast_and_theme_read_back_as_stored() {
        for step in 0..=20u8 {
            let one = Contrast::new(f32::from(step) / 20.0);
            assert_eq!(Contrast::read(Some(&one.stored())), one);
        }
        assert_eq!(Contrast::read(Some("0.33")).amount(), 0.35, "in twentieths");
        assert_eq!(Contrast::read(Some("7")).amount(), 1.0, "clamped");
        assert_eq!(Contrast::read(Some("nope")), Contrast::default());
        assert_eq!(Contrast::default().amount(), 0.5);
        for one in Theme::all() {
            assert_eq!(Theme::read(Some(one.stored())), one);
        }
        assert_eq!(Theme::read(None), Theme::Dark);
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
        assert_eq!(card.tab, Tab::Appearance, "it opens on the first tab");
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
        assert!(card.contrast.y >= floor, "the contrast sits on the choices");
        // The window behind, the card, a box for each choice, the slider, the
        // two themes, the tabs and the button that shuts it: nothing from
        // another tab can be pressed.
        let boxes = settings.boxes(window());
        assert_eq!(
            boxes.len(),
            2 + card.choices.len() + 1 + 2 + Tab::all().len() + 1
        );
        for slug in ["theme-dark", "theme-light"] {
            assert!(
                boxes.iter().any(|placed| placed.name == named().of(slug)),
                "{slug} is drawn but cannot be pressed"
            );
        }
    }

    /// A tab is chosen by its button, and the card is the same height on
    /// every one, so the tabs do not move under the pointer choosing them.
    #[test]
    fn the_tabs_change_the_page_and_not_the_card() {
        let (mut settings, mut fonts) = shown();
        let card = settings.laid().expect("measured").clone();
        for tab in Tab::all() {
            let at = card.tab_rect(tab).expect("a tab");
            let boxes = settings.boxes(window());
            let mut input = Input::default();
            press(&mut input, &boxes, at);
            assert_eq!(settings.react(&mut input), Some(Did::Tab));
            settings.measure(&mut fonts, window());
            let now = settings.laid().expect("measured");
            assert_eq!(now.tab, tab);
            assert_eq!(now.rect, card.rect, "{tab:?} changed the card");
            assert_eq!(now.tab_rect(tab), Some(at), "the tabs moved");
        }
    }

    /// Whatever a tab holds sits between the tabs and the foot.
    #[test]
    fn nothing_runs_out_of_the_card() {
        for tab in Tab::all() {
            let (settings, _fonts) = opened_on(tab);
            let card = settings.laid().expect("measured");
            let done = card.foot.rect("done").expect("placed");
            let top = card.tab_rect(Tab::Appearance).expect("a tab").bottom();
            let mut held: Vec<Rect> = card.choices.iter().map(|choice| choice.rect).collect();
            held.extend(
                [
                    card.group,
                    card.contrast,
                    card.slider,
                    card.theme,
                    card.unreads,
                    card.counts,
                    card.teams,
                    card.kept,
                    card.starting,
                    card.words,
                    card.words_said,
                    card.about,
                    card.build,
                ]
                .into_iter()
                .filter(|rect| rect.height > 0.0),
            );
            held.extend(card.said);
            for row in card.rows() {
                held.extend(row.boxes().into_iter().map(|placed| placed.rect));
            }
            held.retain(|rect| *rect != done);
            assert!(!held.is_empty(), "{tab:?} is empty");
            for rect in held {
                assert!(rect.y >= top, "{tab:?}: {rect:?} is over the tabs");
                assert!(
                    rect.bottom() <= done.y + 0.5,
                    "{tab:?}: {rect:?} reaches the foot"
                );
            }
            assert!(
                done.bottom() <= card.rect.bottom() - PAD + 0.5,
                "the button reaches past the foot of the card"
            );
            assert!(
                card.rect.right() <= window().right() && card.rect.bottom() <= window().bottom()
            );
        }
    }

    /// Counting the mentions the words show is asked for by its buttons, and
    /// reads back as it was kept; on unless turned off.
    #[test]
    fn counting_mention_words_is_asked_for_by_its_buttons() {
        let (mut settings, _fonts) = opened_on(Tab::Notifications);
        let off = settings
            .laid()
            .expect("measured")
            .wordings
            .rect("words-off")
            .expect("placed");
        let boxes = settings.boxes(window());
        let mut input = Input::default();
        press(&mut input, &boxes, off);
        assert_eq!(settings.react(&mut input), Some(Did::Words(Words(false))));
        assert_eq!(Words::read(Some(Words(false).stored())), Words(false));
        assert_eq!(Words::read(None), Words(true));
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
        let (mut settings, mut fonts) = opened_on(Tab::General);
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
        let said = card.said.expect("nothing says a look went out");
        // The card is the tallest tab's height, so the line has room inside
        // it rather than the card growing round it.
        assert!(card.rect.height >= before, "the card shrank");
        assert!(
            said.bottom() <= card.foot.rect("done").expect("placed").y,
            "the answer runs into the foot"
        );
        // And the button is spent while the answer is on its way: a disabled
        // row draws dimmed and registers nothing to press.
        assert!(card.look.boxes().is_empty(), "it can be asked twice");

        settings.looked("This is the newest build.");
        settings.measure(&mut fonts, window());
        let card = settings.laid().expect("measured");
        assert!(!card.look.boxes().is_empty(), "the button never came back");
        assert!(card.said.is_some());
    }

    /// A newer build that is ready is installed from the panel itself, and a
    /// failed install can be tried again from the same button.
    #[test]
    fn a_ready_build_is_installed_from_here() {
        let (mut settings, mut fonts) = opened_on(Tab::General);
        settings.ready(Some("9.9.9"));
        settings.measure(&mut fonts, window());
        let card = settings.laid().expect("measured");
        assert!(
            card.look.rect("look").is_none(),
            "no second check once one is found"
        );
        let install = card.look.rect("install").expect("the install button");

        let boxes = settings.boxes(window());
        let mut input = Input::default();
        press(&mut input, &boxes, install);
        assert_eq!(settings.react(&mut input), Some(Did::Install));
        settings.measure(&mut fonts, window());
        assert!(
            settings.laid().expect("measured").look.boxes().is_empty(),
            "it can be installed twice"
        );

        settings.install_failed("the download broke");
        settings.measure(&mut fonts, window());
        let card = settings.laid().expect("measured");
        assert!(card.look.rect("install").is_some(), "it can be tried again");
        assert!(card.said.is_some(), "and it says why");
    }

    /// Starting with Windows sits between the disk budget and About, and a
    /// press on either button asks for exactly that.
    #[test]
    fn starting_with_windows_is_asked_for_by_its_buttons() {
        let (mut settings, _fonts) = opened_on(Tab::General);
        let card = settings.laid().expect("measured");
        let on = card.starts.rect("startup-on").expect("placed");
        assert!(
            card.kept.bottom() <= card.starting.y,
            "under the disk budget"
        );
        assert!(on.bottom() <= card.about.y, "above About");

        let boxes = settings.boxes(window());
        let mut input = Input::default();
        press(&mut input, &boxes, on);
        assert_eq!(settings.react(&mut input), Some(Did::Startup(true)));
        assert!(settings.startup);

        let (mut settings, _fonts) = opened_on(Tab::General);
        let off = settings
            .laid()
            .expect("measured")
            .starts
            .rect("startup-off")
            .expect("placed");
        let boxes = settings.boxes(window());
        let mut input = Input::default();
        press(&mut input, &boxes, off);
        assert_eq!(settings.react(&mut input), Some(Did::Startup(false)));
    }

    /// How teams are listed sits under grouping unread channels, and its
    /// buttons ask for exactly what they say.
    #[test]
    fn listing_teams_is_asked_for_by_its_buttons() {
        let (mut settings, _fonts) = opened_on(Tab::Sidebar);
        let card = settings.laid().expect("measured");
        let one = card.showings.rect("teams-one").expect("placed");
        assert!(
            card.unreads.bottom() <= card.teams.y,
            "under grouping unreads"
        );

        let boxes = settings.boxes(window());
        let mut input = Input::default();
        press(&mut input, &boxes, one);
        assert_eq!(settings.react(&mut input), Some(Did::Teams(Teams::One)));
        assert_eq!(settings.teams, Teams::One);
        assert_eq!(Teams::read(Some(Teams::One.stored())), Teams::One);
        assert_eq!(Teams::read(None), Teams::All);
    }

    /// Grouping unread channels heads the sidebar's tab, and its buttons ask
    /// for exactly what they say.
    #[test]
    fn grouping_unreads_is_asked_for_by_its_buttons() {
        let (mut settings, _fonts) = opened_on(Tab::Sidebar);
        let card = settings.laid().expect("measured");
        let on = card.groupings.rect("unreads-on").expect("placed");
        assert!(on.bottom() <= card.teams.y, "above the teams");

        let boxes = settings.boxes(window());
        let mut input = Input::default();
        press(&mut input, &boxes, on);
        assert_eq!(settings.react(&mut input), Some(Did::Unreads(true)));
        assert!(settings.unreads);

        let (mut settings, _fonts) = opened_on(Tab::Sidebar);
        let off = settings
            .laid()
            .expect("measured")
            .groupings
            .rect("unreads-off")
            .expect("placed");
        let boxes = settings.boxes(window());
        let mut input = Input::default();
        press(&mut input, &boxes, off);
        assert_eq!(settings.react(&mut input), Some(Did::Unreads(false)));
    }
}
