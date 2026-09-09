//! The row plan: Rust decides what the message list contains, Svelte maps over it.
//!
//! The instinct is to ship posts to the frontend and let it group them, insert
//! date separators and render markdown. That puts per-frame logic in JavaScript
//! and makes row heights unknowable until mount. Inverted, this crate emits a
//! **flat array of typed rows** -- already filtered, grouped, separated and
//! parsed -- and the shell renders one component per row kind with no decisions
//! of its own.
//!
//! Consequences worth stating:
//!
//! * Rows are measurable before mount, so the virtualiser gets a real height
//!   cache instead of guesses.
//! * Re-rendering one post touches one row rather than re-deciding the list.
//! * **With collapsed threads on, a reply is not in the channel stream at all.**
//!   Phase 0 measured this account at `collapsed_reply_threads: on` with 84% of
//!   posts being replies, so a flat list is not a simplification to refine
//!   later -- it is a different data model. The stream is roots plus footers.
//!
//! Nothing here formats a date or a time: the plan carries raw server
//! milliseconds and an epoch day, and the shell formats with the viewer's
//! locale. Keeping locale out of Rust is deliberate.

pub mod emoji;
mod emoji_categories;
mod emoji_table;
pub mod markdown;

use markdown::Node;
use matterless_core::model::{Post, ThreadMode, Timestamp};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Consecutive posts by one author inside this window collapse into a
/// continuation row. Matches the webapp's five minutes.
pub const DEFAULT_COLLAPSE_WINDOW_MS: i64 = 5 * 60 * 1000;
const MS_PER_DAY: i64 = 86_400_000;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReactionSummary {
    pub emoji: String,
    pub count: usize,
    /// Whether the viewer is among them, so the shell can show it pressed.
    pub mine: bool,
    /// The character for a standard emoji, from the same table message bodies
    /// use.
    ///
    /// Resolved here rather than in the shell because it was resolved in both:
    /// a second, shorter table over there rendered `:pray:` as its name while
    /// the same emoji in a sentence came out as a character.
    pub unicode: Option<String>,
    /// Everyone who reacted, in the order they did, resolved to names -- the
    /// viewer appears as "You".
    ///
    /// All of them, not a sample: the official client names every reactor, and
    /// on a post with 48 of them "and 40 others" answers none of the question
    /// the tooltip exists for. They cost nothing to resolve here -- the plan
    /// already hydrates every reactor's name so the pill can be labelled at
    /// all.
    pub names: Vec<String>,
}

/// A preview drawn under a message.
///
/// Two shapes, so an enum rather than one struct with half its fields empty:
/// a page preview is a card, and a permalink preview is a quoted message.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Preview {
    /// A fetched page: what the server's OpenGraph pass found.
    Page {
        url: String,
        title: String,
        description: String,
        site_name: String,
        /// Absent when the page offered no image, or offered one with no usable
        /// URL.
        image: Option<PreviewImage>,
    },
    /// Another message, quoted in place.
    Permalink {
        post_id: String,
        channel_id: String,
        /// Where it was said. A direct message is labelled by its *kind*: its
        /// `channel_display_name` comes back empty, and its name is the
        /// `id__id` pair.
        channel_label: String,
        author_name: String,
        create_at: Timestamp,
        /// The quoted message, parsed the same way any other body is.
        nodes: Vec<Node>,
    },
}

/// A preview image, with the box it is drawn in.
///
/// Sized here for the same reason attachments are: the virtualiser reserves a
/// row's height before it mounts, and an image whose size only became known
/// when it loaded would make every estimate below it wrong.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PreviewImage {
    pub url: String,
    pub width: i32,
    pub height: i32,
}

/// The largest a preview image is drawn.
const PREVIEW_BOX: (i32, i32) = (400, 220);

/// Previews for a post, in the order the server listed them.
fn previews_of(post: &Post, options: &PlanOptions) -> Vec<Preview> {
    post.metadata
        .embeds
        .iter()
        .filter_map(|embed| match embed.embed_type.as_str() {
            "opengraph" => page_preview(embed),
            "permalink" => permalink_preview(embed, options),
            // A bare `link` embed is `{type, url}` and nothing else: the server
            // fetched no metadata for it, so a card would repeat the URL that
            // is already in the message.
            _ => None,
        })
        .collect()
}

fn page_preview(embed: &matterless_core::model::Embed) -> Option<Preview> {
    let data = embed.data.as_object()?;
    let text = |key: &str| {
        data.get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let title = text("title");
    let description = text("description");
    // Nothing worth a card: no title and no description is just a URL.
    if title.is_empty() && description.is_empty() {
        return None;
    }

    let image = data
        .get("images")
        .and_then(serde_json::Value::as_array)
        .and_then(|images| images.first())
        .and_then(|image| image.as_object())
        .and_then(|image| {
            // `secure_url` first: it is the https one where a page offers both.
            let url = ["secure_url", "url"]
                .iter()
                .filter_map(|key| image.get(*key).and_then(serde_json::Value::as_str))
                .find(|found| !found.is_empty())?
                .to_string();
            // The dimensions arrive as strings on this server ("1200"), which
            // is why they are parsed rather than read as numbers.
            let number = |key: &str| -> i32 {
                image
                    .get(key)
                    .map(|value| match value {
                        serde_json::Value::String(text) => text.parse().unwrap_or(0),
                        other => other.as_i64().unwrap_or(0) as i32,
                    })
                    .unwrap_or(0)
            };
            let (width, height) = fit_box(number("width"), number("height"), PREVIEW_BOX);
            Some(PreviewImage { url, width, height })
        });

    Some(Preview::Page {
        url: embed.url.clone(),
        title,
        description,
        site_name: text("site_name"),
        image,
    })
}

fn permalink_preview(
    embed: &matterless_core::model::Embed,
    options: &PlanOptions,
) -> Option<Preview> {
    let data = embed.data.as_object()?;
    let quoted: Post = serde_json::from_value(data.get("post")?.clone()).ok()?;
    if quoted.delete_at != 0 {
        return None;
    }

    let channel_type = data
        .get("channel_type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let display_name = data
        .get("channel_display_name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let channel_label = match (channel_type, display_name) {
        // Same rule as a toast title: a direct message is labelled by its kind,
        // because its display name is empty and its name is a pair of ids.
        ("D", _) => "Direct message".to_string(),
        ("G", "") => "Group message".to_string(),
        (_, "") => String::new(),
        (_, named) => named.to_string(),
    };

    Some(Preview::Permalink {
        author_name: options
            .author_names
            .get(&quoted.user_id)
            .cloned()
            .unwrap_or_else(|| quoted.user_id.clone()),
        create_at: quoted.create_at,
        channel_id: quoted.channel_id.clone(),
        channel_label,
        // Parsed here rather than looked up in `options.parsed`: a quoted post
        // is not one of the page's own, so it was never in that map.
        nodes: markdown::parse(&quoted.message),
        post_id: quoted.id,
    })
}

/// One attachment on a post, resolved for display.
///
/// The layout box is computed here rather than in CSS because the virtualiser
/// learns a height per row kind and reserves it before the row exists: an image
/// whose size only becomes known when its bytes arrive would make every
/// estimate wrong and the scroll position jump when it loaded.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileRef {
    pub id: String,
    pub name: String,
    pub extension: String,
    pub size: i64,
    pub mime_type: String,
    /// The file's own pixels; zero for anything that is not an image.
    pub width: i32,
    pub height: i32,
    /// Renders inline as a picture rather than as a card.
    pub image: bool,
    /// Plays inline rather than being offered as a download.
    pub video: bool,
    /// Which of the server's three renditions to draw.
    pub variant: ImageVariant,
    /// The ~1 KB base64 JPEG from the post's metadata, shown blurred until the
    /// real bytes arrive. Costs no request, so there is no reason not to.
    pub mini_preview: Option<String>,
    /// The box the image is drawn in: never larger than the chosen variant's
    /// own pixels, so nothing is ever upscaled.
    pub box_width: i32,
    pub box_height: i32,
    /// The bytes are gone from the server's storage.
    pub archived: bool,
}

/// Which rendition of a file to ask the server for.
///
/// Sizes measured against a live server, not assumed:
/// a thumbnail is capped at 120x100 and a preview at 1920 wide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageVariant {
    /// `/files/{id}/thumbnail` -- at most 120x100, a few kilobytes.
    Thumb,
    /// `/files/{id}/preview` -- at most 1920 wide, re-encoded as JPEG.
    Preview,
    /// `/files/{id}` -- the file as uploaded. The only honest choice for an
    /// animation or a vector, whose "preview" would be a still frame or
    /// nothing at all.
    Original,
}

/// The largest a lone inline image is drawn.
const IMAGE_BOX: (i32, i32) = (420, 350);

/// The server's thumbnail ceiling. Measured: 1899x1052 comes back 120x66,
/// 3072x4096 as 75x100, 52x107 as 49x100 -- so 120 wide by 100 tall, and never
/// upscaled.
const THUMB_BOX: (i32, i32) = (120, 100);

/// How the post as a whole wants its attachments drawn.
///
/// Passed in rather than inferred per file because the decision is about the
/// post: one image gets room and resolution, several get a compact row of
/// thumbnails at their true size.
#[derive(Debug, Clone, Copy)]
pub struct FileLayout {
    /// This post carries more than one image, so they are drawn as a row of
    /// thumbnails rather than one large picture.
    pub gallery: bool,
    /// Draw a lone image from the file as uploaded instead of the server's
    /// preview rendition. The reader's choice: it is sharper on a large screen
    /// and it is also several megabytes.
    pub full_res: bool,
    /// The server's `EnableSVGs`. False here, so an SVG attachment is drawn as
    /// a file card rather than inline.
    pub allow_svg: bool,
    /// The viewer's `devicePixelRatio`.
    ///
    /// A 120x100 thumbnail drawn in a 120x100 CSS box on a 1.25 ratio display
    /// is a 25% upscale, which is exactly the blur this is here to avoid: the
    /// box is divided by the ratio so one image pixel covers at least one
    /// device pixel.
    pub pixel_ratio: f32,
}

impl Default for FileLayout {
    fn default() -> Self {
        Self {
            gallery: false,
            full_res: false,
            allow_svg: false,
            pixel_ratio: 1.0,
        }
    }
}

impl FileRef {
    /// Resolves one for display. Public because the composer's tray shows an
    /// uploaded file the same way the message list shows an attached one, and
    /// there should not be two answers to "how big is this drawn".
    pub fn from_info(file: &matterless_core::model::FileInfo, layout: FileLayout) -> Self {
        // An image either has server-side previews or simply says so in its
        // mime type -- an SVG has no thumbnail but is still a picture.
        let vector = file.mime_type == "image/svg+xml";
        let image = (file.has_preview_image || file.mime_type.starts_with("image/"))
            && !file.archived
            // An SVG is a picture here only if the server allows it: `EnableSVGs`
            // is false on this one, and it is false because an SVG is a document
            // that can carry script and external references. The official client
            // shows a file card instead.
            && (!vector || layout.allow_svg);
        // A GIF's thumbnail is a still frame and an SVG has neither thumbnail
        // nor preview, so both want the file itself however big it is.
        let rendered = !matches!(file.mime_type.as_str(), "image/gif" | "image/svg+xml");

        // What a webview will actually play. The container is only half of it --
        // an AVI or a MKV is refused whatever its extension says -- so the list
        // is the formats WebView2 decodes rather than everything video-shaped.
        let video = !file.archived
            && matches!(
                file.mime_type.as_str(),
                "video/mp4" | "video/webm" | "video/ogg" | "video/quicktime"
            );

        let variant = match (rendered, layout.gallery, layout.full_res) {
            (false, _, _) => ImageVariant::Original,
            (true, true, _) => ImageVariant::Thumb,
            (true, false, true) => ImageVariant::Original,
            (true, false, false) => ImageVariant::Preview,
        };

        // Two decisions, and they used to be one: *which* rendition to fetch,
        // and how big to draw it. A GIF must fetch the original -- its
        // thumbnail is a still frame -- and a video plays the file itself, but
        // in a gallery both should still be drawn thumbnail-sized like
        // everything beside them. So the box comes from the layout, and only
        // the pixel-ratio trim comes from the variant.
        let (box_width, box_height) = if layout.gallery {
            let (natural_width, natural_height) = fit_box(file.width, file.height, THUMB_BOX);
            if matches!(variant, ImageVariant::Thumb) {
                // Only a fetched thumbnail has a pixel budget to divide the box
                // down to; anything drawn from the original does not.
                for_ratio(natural_width, natural_height, layout.pixel_ratio)
            } else {
                (natural_width, natural_height)
            }
        } else {
            // A preview is at least 1920 wide and an original is whatever was
            // uploaded, so both are larger than the box for anything but a small
            // image -- and `fit_box` never upscales a small one.
            fit_box(file.width, file.height, IMAGE_BOX)
        };

        Self {
            id: file.id.clone(),
            name: file.name.clone(),
            extension: file.extension.clone(),
            size: file.size,
            mime_type: file.mime_type.clone(),
            width: file.width,
            height: file.height,
            image,
            video,
            variant,
            mini_preview: file.mini_preview.clone(),
            box_width,
            box_height,
            archived: file.archived,
        }
    }
}

/// Every attachment on a post, laid out as the post as a whole calls for.
fn files_of(post: &Post, options: &PlanOptions) -> Vec<FileRef> {
    // Everything that draws at its own size rather than as a card, which is what
    // decides whether one of them gets the room or they share it. A player
    // counts: two videos stacked full width push the next post off the screen
    // exactly as two pictures would.
    let inline = post
        .metadata
        .files
        .iter()
        .filter(|file| {
            !file.archived
                && (file.has_preview_image
                    || file.mime_type.starts_with("image/")
                    || file.mime_type.starts_with("video/"))
        })
        .count();
    let layout = FileLayout {
        // One gets the room; several get a row of thumbnails, which is both more
        // compact and sharper than several stretched ones.
        gallery: inline > 1,
        full_res: options.full_res,
        allow_svg: options.allow_svg,
        pixel_ratio: options.pixel_ratio,
    };
    post.metadata
        .files
        .iter()
        .map(|file| FileRef::from_info(file, layout))
        .collect()
}

/// Scales `width` x `height` down into `limit`, preserving the aspect ratio. A
/// picture smaller than the limit is left alone rather than blown up.
fn fit_box(width: i32, height: i32, limit: (i32, i32)) -> (i32, i32) {
    let (limit_width, limit_height) = limit;
    if width <= 0 || height <= 0 {
        return (0, 0);
    }
    if width <= limit_width && height <= limit_height {
        return (width, height);
    }
    // Integer arithmetic throughout, rounded to nearest rather than truncated:
    // the server rounds, and matching it is what makes the reserved box the
    // size of the image that arrives. Checked against all six pairs measured
    // by measurement -- truncation disagrees with three of them
    // (330x377 comes back 88x100, not 87, and 52x107 comes back 49x100).
    let by_width = scaled(limit_width, height, width);
    if by_width <= limit_height as i64 {
        (limit_width, by_width.max(1) as i32)
    } else {
        (
            scaled(limit_height, width, height).max(1) as i32,
            limit_height,
        )
    }
}

/// `limit * other / own`, rounded to nearest.
fn scaled(limit: i32, other: i32, own: i32) -> i64 {
    let numerator = limit as i64 * other as i64 + own as i64 / 2;
    numerator / own as i64
}

/// The same pixels expressed in CSS pixels for a display of this ratio, so an
/// image is never asked to cover more device pixels than it has.
fn for_ratio(width: i32, height: i32, pixel_ratio: f32) -> (i32, i32) {
    if pixel_ratio <= 1.0 || width <= 0 || height <= 0 {
        return (width, height);
    }
    let scaled_width = (width as f32 / pixel_ratio).floor().max(1.0) as i32;
    let scaled_height = (height as f32 / pixel_ratio).floor().max(1.0) as i32;
    (scaled_width, scaled_height)
}

/// The Slack-style attachment integrations use. A CI or bot post often leaves
/// `message` empty and puts everything here, so ignoring it shows blank posts
/// from exactly the integrations you care about.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Attachment {
    pub color: Option<String>,
    pub pretext: Vec<Node>,
    pub title: Option<String>,
    pub title_link: Option<String>,
    pub text: Vec<Node>,
    pub fields: Vec<AttachmentField>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AttachmentField {
    pub title: String,
    pub value: Vec<Node>,
    pub short: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PostRow {
    pub post_id: String,
    /// The thread this message belongs to, empty when it *is* the root. Carried
    /// because the shell's thread actions -- reply, follow -- act on the thread
    /// rather than on the message.
    pub root_id: String,
    pub author_id: String,
    /// Resolved here rather than in the shell, because which name to show is a
    /// server-side preference (`TeammateNameDisplay`), not a display detail.
    /// Falls back to the id so a missing user is visible instead of blank.
    pub author_name: String,
    pub create_at: Timestamp,
    /// The markdown cache key: an edit bumps it, so a stale row simply misses.
    pub update_at: Timestamp,
    pub edited: bool,
    /// Shared rather than owned, so handing a cached body to a row is a refcount
    /// bump instead of a recursive copy of the whole tree. Serialises to the
    /// same JSON as a plain `Vec<Node>` would.
    pub nodes: Arc<Vec<Node>>,
    pub reactions: Vec<ReactionSummary>,
    pub files: Vec<FileRef>,
    pub attachments: Vec<Attachment>,
    /// The author's `last_picture_update`, which is the avatar's version.
    ///
    /// Part of the image URL rather than a header: a new picture arrives under
    /// the same user id, so without it the old face would stay cached until the
    /// process restarted.
    pub avatar_at: Timestamp,
    /// Posted by an integration rather than a person.
    ///
    /// Measured on this server: 243 of 5101 posts came from a bot and 14 from a
    /// webhook, and a webhook post's name is its own -- so the author's user
    /// record is the wrong thing to show for it.
    pub bot: bool,
    /// True when the post is only a webhook payload, so the shell knows not to
    /// leave an empty message body.
    pub body_is_attachment_only: bool,
    /// An optimistic send that the server has not confirmed. Held in memory
    /// only and never written to SQLite, so a wrong guess is never persisted.
    pub pending: bool,
    /// The send failed. The row stays, with a retry, rather than vanishing --
    /// a message that silently disappears is worse than one marked broken.
    pub failed: bool,
    /// Pinned to the channel, for everyone.
    pub pinned: bool,
    /// Saved by this reader. A `flagged_post` preference on the server, so it
    /// comes from the preferences rather than from the post.
    pub saved: bool,
    /// This reader follows the thread this message belongs to -- its own, if it
    /// is a root. Decides whether the menu offers to follow or to unfollow.
    pub following: bool,
    /// Link and permalink previews the server attached.
    pub previews: Vec<Preview>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Row {
    DateSeparator {
        /// Days since the epoch in the viewer's offset. The shell formats it.
        epoch_day: i64,
    },
    /// Positioned from `last_viewed_at`. A row, not an overlay, so it survives
    /// virtualisation and takes part in height.
    UnreadDivider,
    Post {
        post: PostRow,
    },
    /// Same author, inside the collapse window: no avatar, no header.
    Continuation {
        post: PostRow,
    },
    System {
        post_id: String,
        post_type: String,
        nodes: Vec<Node>,
        /// What actually happened, in words.
        ///
        /// Composed here because it needs `props`, which the shell does not
        /// see: it was rendering the raw type, so a channel join read
        /// "join channel" rather than naming who joined.
        text: String,
    },
    /// A deleted root still holding replies. It has to render as a placeholder
    /// or its thread loses its anchor.
    DeletedRoot {
        post_id: String,
    },
    ThreadFooter {
        root_id: String,
        reply_count: i64,
        last_reply_at: Timestamp,
        /// Who has replied, resolved here: a name for the tooltip and the
        /// avatar's version, because the shell can look up neither.
        participants: Vec<ThreadFace>,
        /// Replies you have not read, from the server's own per-thread count.
        ///
        /// This row is the whole answer to the gap Phase 3 measured: with
        /// collapsed threads a reply is never a row, so a channel could show a
        /// truthful unread badge with nothing on screen to explain it.
        unread_replies: i64,
        /// Of those, the ones that named you.
        unread_mentions: i64,
        /// Whether this thread is being followed, which is what decides if its
        /// unread counts are asking for anything.
        following: bool,
    },
}

/// One person in a thread, as its footer shows them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ThreadFace {
    pub user_id: String,
    pub name: String,
    /// `last_picture_update`, so a changed avatar is a different URL.
    pub avatar_at: Timestamp,
}

/// What a thread footer shows.
///
/// The reply count and participants are derived from the posts held locally;
/// the unread counts are the server's, because a reply that arrived while the
/// app was closed still has to count and only the server knows what was read on
/// another device.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ThreadSummary {
    pub reply_count: i64,
    pub last_reply_at: Timestamp,
    pub participants: Vec<String>,
    pub unread_replies: i64,
    pub unread_mentions: i64,
    pub following: bool,
}

/// The row immediately older than this page, when a page is not the oldest.
///
/// A plan built for a page in isolation cannot know whether its first post
/// continues an author's run, or whether its day has already been announced --
/// so it would grow a duplicate author header and a stray date separator at
/// every page seam. This carries just enough of the previous page's last post
/// to answer both.
#[derive(Debug, Clone)]
pub struct Preceding {
    pub user_id: String,
    pub create_at: Timestamp,
}

pub struct PlanOptions {
    /// The viewer's offset, so day boundaries land where they see them.
    pub utc_offset_minutes: i32,
    pub collapse_window_ms: i64,
    pub thread_mode: ThreadMode,
    /// From the channel member record. Zero means "no divider".
    pub last_viewed_at: Timestamp,
    pub me_id: String,
    /// user id -> the name to show, already resolved against the display
    /// preference by the caller.
    pub author_names: HashMap<String, String>,
    /// `last_picture_update` per author, for avatar URLs.
    pub author_avatars: HashMap<String, Timestamp>,
    /// Already-parsed message bodies, keyed by post id.
    ///
    /// Parsing measured at 78% of a plan build once the two SQL costs were
    /// fixed, so the caller supplies whatever it has cached by
    /// `post.id + update_at` and anything absent is parsed here.
    pub parsed: HashMap<String, Arc<Vec<Node>>>,
    /// Post ids that exist only in memory as optimistic sends.
    pub pending: HashSet<String>,
    /// Of those, the ones whose send failed.
    pub failed: HashSet<String>,
    /// What sits directly above this page, when it is not the oldest one.
    pub preceding: Option<Preceding>,
    /// Posts this reader has saved, by id.
    pub saved: HashSet<String>,
    /// Threads this reader follows, by root post id. Keyed by root so a reply
    /// answers the same as the root it belongs to.
    pub followed: HashSet<String>,
    /// Draw a lone image from the file as uploaded rather than the server's
    /// preview rendition. The reader's setting.
    pub full_res: bool,
    /// The server's `EnableSVGs`; false means an SVG is a file, not a picture.
    pub allow_svg: bool,
    /// The viewer's `devicePixelRatio`, so a thumbnail is not asked to cover
    /// more device pixels than it has.
    pub pixel_ratio: f32,
}

impl PlanOptions {
    pub fn new(thread_mode: ThreadMode, me_id: impl Into<String>) -> Self {
        Self {
            utc_offset_minutes: 0,
            collapse_window_ms: DEFAULT_COLLAPSE_WINDOW_MS,
            thread_mode,
            last_viewed_at: 0,
            me_id: me_id.into(),
            author_names: HashMap::new(),
            author_avatars: HashMap::new(),
            parsed: HashMap::new(),
            pending: HashSet::new(),
            failed: HashSet::new(),
            preceding: None,
            saved: HashSet::new(),
            followed: HashSet::new(),
            full_res: false,
            allow_svg: false,
            pixel_ratio: 1.0,
        }
    }
}

/// Which name to show for a user, per the server's `TeammateNameDisplay`.
/// Phase 0 measured this deployment on `username`.
pub fn display_name(user: &matterless_core::User, mode: &str) -> String {
    let full = format!("{} {}", user.first_name, user.last_name)
        .trim()
        .to_string();
    match mode {
        "full_name" if !full.is_empty() => full,
        "nickname_full_name" => {
            if !user.nickname.is_empty() {
                user.nickname.clone()
            } else if !full.is_empty() {
                full
            } else {
                user.username.clone()
            }
        }
        _ => user.username.clone(),
    }
}

fn epoch_day(create_at: Timestamp, utc_offset_minutes: i32) -> i64 {
    let shifted = create_at + i64::from(utc_offset_minutes) * 60_000;
    shifted.div_euclid(MS_PER_DAY)
}

fn summarise_reactions(post: &Post, options: &PlanOptions) -> Vec<ReactionSummary> {
    let mut order: Vec<String> = Vec::new();
    let mut counts: HashMap<&str, (usize, bool)> = HashMap::new();
    let mut reactors: HashMap<&str, Vec<String>> = HashMap::new();
    for reaction in &post.metadata.reactions {
        let entry = counts
            .entry(reaction.emoji_name.as_str())
            .or_insert_with(|| {
                order.push(reaction.emoji_name.clone());
                (0, false)
            });
        entry.0 += 1;
        let mine = reaction.user_id == options.me_id;
        entry.1 |= mine;

        reactors
            .entry(reaction.emoji_name.as_str())
            .or_default()
            .push(if mine {
                "You".to_string()
            } else {
                // Falls back to the id rather than blank: a name that failed to
                // hydrate should be visible as one, not silently missing.
                options
                    .author_names
                    .get(&reaction.user_id)
                    .cloned()
                    .unwrap_or_else(|| reaction.user_id.clone())
            });
    }
    order
        .into_iter()
        .map(|emoji| {
            let (count, mine) = counts.get(emoji.as_str()).copied().unwrap_or((0, false));
            let unicode = emoji::character_for(&emoji);
            let names = reactors.remove(emoji.as_str()).unwrap_or_default();
            ReactionSummary {
                emoji,
                count,
                mine,
                unicode,
                names,
            }
        })
        .collect()
}

fn attachment_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|found| found.as_str())
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn parse_attachments(props: &serde_json::Value) -> Vec<Attachment> {
    props
        .get("attachments")
        .and_then(|value| value.as_array())
        .map(|array| {
            array
                .iter()
                .map(|raw| Attachment {
                    color: attachment_string(raw, "color"),
                    pretext: attachment_string(raw, "pretext")
                        .map(|text| markdown::parse(&text))
                        .unwrap_or_default(),
                    title: attachment_string(raw, "title"),
                    title_link: attachment_string(raw, "title_link"),
                    // `fallback` is the plain-text version an integration
                    // supplies for notifications, and on this server 76
                    // attachments populate it while leaving `text` empty -- so
                    // without it those posts render as nothing at all. Used
                    // only when the attachment would otherwise be blank, since
                    // it usually duplicates `pretext` word for word.
                    text: attachment_string(raw, "text")
                        .or_else(|| {
                            let has_other_content = attachment_string(raw, "pretext").is_some()
                                || attachment_string(raw, "title").is_some()
                                || raw
                                    .get("fields")
                                    .and_then(|value| value.as_array())
                                    .is_some_and(|fields| !fields.is_empty());
                            if has_other_content {
                                None
                            } else {
                                attachment_string(raw, "fallback")
                            }
                        })
                        .map(|text| markdown::parse(&text))
                        .unwrap_or_default(),
                    fields: raw
                        .get("fields")
                        .and_then(|value| value.as_array())
                        .map(|fields| {
                            fields
                                .iter()
                                .map(|field| AttachmentField {
                                    title: attachment_string(field, "title").unwrap_or_default(),
                                    value: attachment_string(field, "value")
                                        .map(|text| markdown::parse(&text))
                                        .unwrap_or_default(),
                                    short: field
                                        .get("short")
                                        .and_then(|value| value.as_bool())
                                        .unwrap_or(false),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A string from a post's props, empty treated as absent.
fn prop(props: &serde_json::Value, key: &str) -> Option<String> {
    props
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// What to call the author, and whether it is an integration.
///
/// A webhook carries its own display name in props -- `webhook_display_name` on
/// this server, which is what it actually sends; `override_username` is the
/// other name the API allows and is honoured too, but was not seen once in 5101
/// posts. Showing the owning user's name instead would credit a colleague with
/// everything CI says.
fn author_of(post: &Post, options: &PlanOptions) -> (String, bool) {
    let from_webhook = prop(&post.props, "from_webhook").as_deref() == Some("true");
    let from_bot = prop(&post.props, "from_bot").as_deref() == Some("true");
    let overridden = prop(&post.props, "webhook_display_name")
        .or_else(|| prop(&post.props, "override_username"));

    let name = overridden.unwrap_or_else(|| {
        options
            .author_names
            .get(&post.user_id)
            .cloned()
            .unwrap_or_else(|| post.user_id.clone())
    });
    (name, from_webhook || from_bot)
}

/// A system message as a sentence.
///
/// The types and props here are the ones this server actually sends, counted
/// across 5101 posts: joins (119), adds (90), leaves (35), team joins (7),
/// header changes (5), team removals (2). Anything else falls back to a
/// readable form of its type rather than a slug, so a new type is odd-looking
/// instead of meaningless.
fn system_message(post: &Post) -> String {
    let who = prop(&post.props, "username").unwrap_or_else(|| "Someone".to_string());
    let target = prop(&post.props, "addedUsername")
        .or_else(|| prop(&post.props, "removedUsername"))
        .unwrap_or_else(|| "someone".to_string());

    match post.post_type.as_str() {
        "system_join_channel" => format!("{who} joined the channel"),
        "system_leave_channel" => format!("{who} left the channel"),
        "system_add_to_channel" => format!("{who} added {target} to the channel"),
        "system_remove_from_channel" => format!("{target} was removed from the channel"),
        "system_join_team" => format!("{who} joined the team"),
        "system_leave_team" => format!("{who} left the team"),
        "system_add_to_team" => format!("{who} added {target} to the team"),
        "system_remove_from_team" => format!("{target} was removed from the team"),
        "system_header_change" => match (
            prop(&post.props, "old_header"),
            prop(&post.props, "new_header"),
        ) {
            (_, Some(_)) => format!("{who} updated the channel header"),
            (Some(_), None) => format!("{who} removed the channel header"),
            _ => format!("{who} changed the channel header"),
        },
        "system_purpose_change" => format!("{who} updated the channel purpose"),
        "system_displayname_change" => format!("{who} renamed the channel"),
        "system_channel_deleted" => format!("{who} archived the channel"),
        "system_channel_restored" => format!("{who} unarchived the channel"),
        // The post's own message, when it has one, beats any guess.
        _ if !post.message.trim().is_empty() => post.message.trim().to_string(),
        other => {
            let words = other.trim_start_matches("system_").replace('_', " ");
            format!("{who}: {words}")
        }
    }
}

fn build_post_row(post: &Post, options: &PlanOptions) -> PostRow {
    let attachments = parse_attachments(&post.props);
    let nodes = match options.parsed.get(&post.id) {
        // Arc::clone: a counter increment, not a tree copy.
        Some(cached) => Arc::clone(cached),
        None => Arc::new(markdown::parse(&post.message)),
    };
    let (author_name, bot) = author_of(post, options);
    PostRow {
        post_id: post.id.clone(),
        root_id: post.root_id.clone(),
        author_name,
        bot,
        avatar_at: options
            .author_avatars
            .get(&post.user_id)
            .copied()
            .unwrap_or(0),
        author_id: post.user_id.clone(),
        create_at: post.create_at,
        update_at: post.update_at,
        edited: post.edit_at != 0,
        body_is_attachment_only: nodes.is_empty() && !attachments.is_empty(),
        nodes,
        reactions: summarise_reactions(post, options),
        files: files_of(post, options),
        attachments,
        pending: options.pending.contains(&post.id),
        failed: options.failed.contains(&post.id),
        previews: previews_of(post, options),
        pinned: post.is_pinned,
        saved: options.saved.contains(&post.id),
        following: options.followed.contains(thread_root_of(post)),
    }
}

/// The root of the thread a post belongs to: itself, if it is one.
fn thread_root_of(post: &Post) -> &str {
    if post.root_id.is_empty() {
        &post.id
    } else {
        &post.root_id
    }
}

/// Builds the channel stream.
///
/// `posts` arrive as `Store::channel_page` returns them -- **newest first** --
/// and the rows come back oldest first, in reading order.
pub fn plan_channel(
    posts: &[Post],
    threads: &HashMap<String, ThreadSummary>,
    options: &PlanOptions,
) -> Vec<Row> {
    let mut ordered: Vec<&Post> = posts.iter().collect();
    ordered.sort_by(|left, right| {
        left.create_at
            .cmp(&right.create_at)
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut rows: Vec<Row> = Vec::with_capacity(ordered.len() + 4);
    // Seeded from the previous page, so a run of messages and a day both carry
    // across a seam instead of restarting at it.
    let mut current_day: Option<i64> = options
        .preceding
        .as_ref()
        .map(|preceding| epoch_day(preceding.create_at, options.utc_offset_minutes));
    // The divider belongs above the first post newer than the watermark. If the
    // page above already contains such a post, it carries the divider, and
    // emitting another here would put one on every page.
    let mut divider_placed = options.last_viewed_at == 0
        || options
            .preceding
            .as_ref()
            .is_some_and(|preceding| preceding.create_at > options.last_viewed_at);
    // Author and timestamp of the last row eligible to be continued.
    let mut previous: Option<(&str, Timestamp)> = options
        .preceding
        .as_ref()
        .map(|preceding| (preceding.user_id.as_str(), preceding.create_at));

    // What a post contributes to the stream, decided before any separator is
    // emitted: a separator above a post that turns out to be invisible would
    // hang there with nothing under it.
    enum Payload {
        DeletedRoot,
        System,
        Post,
    }

    for post in ordered {
        // A reply belongs to its thread, not the stream, when threads collapse.
        if options.thread_mode == ThreadMode::Collapsed && post.is_reply() {
            continue;
        }

        let summary = threads.get(&post.id);
        let payload = if post.is_deleted() {
            // Only a root still holding replies survives as a placeholder.
            if summary.is_some_and(|thread| thread.reply_count > 0) {
                Some(Payload::DeletedRoot)
            } else {
                None
            }
        } else if post.is_system() {
            Some(Payload::System)
        } else {
            Some(Payload::Post)
        };
        let Some(payload) = payload else { continue };

        let day = epoch_day(post.create_at, options.utc_offset_minutes);
        let mut broke_run = false;
        if current_day != Some(day) {
            rows.push(Row::DateSeparator { epoch_day: day });
            current_day = Some(day);
            broke_run = true;
        }
        if !divider_placed && post.create_at > options.last_viewed_at {
            rows.push(Row::UnreadDivider);
            divider_placed = true;
            broke_run = true;
        }

        if matches!(payload, Payload::DeletedRoot) {
            rows.push(Row::DeletedRoot {
                post_id: post.id.clone(),
            });
            previous = None;
            continue;
        }
        if matches!(payload, Payload::System) {
            rows.push(Row::System {
                post_id: post.id.clone(),
                post_type: post.post_type.clone(),
                nodes: markdown::parse(&post.message),
                text: system_message(post),
            });
            previous = None;
            continue;
        }

        let continues = !broke_run
            && previous.is_some_and(|(author, at)| {
                author == post.user_id && post.create_at - at <= options.collapse_window_ms
            });
        let row = build_post_row(post, options);
        rows.push(if continues {
            Row::Continuation { post: row }
        } else {
            Row::Post { post: row }
        });
        previous = Some((post.user_id.as_str(), post.create_at));

        // A root with replies grows a footer. Only meaningful when collapsed;
        // in flat mode the replies are in the stream already.
        //
        // Unread on its own is enough: a followed thread can have unread replies
        // that are not held locally, and suppressing the footer then would
        // recreate exactly the gap this row exists to close.
        if options.thread_mode == ThreadMode::Collapsed
            && let Some(thread) = summary
            && (thread.reply_count > 0 || thread.unread_replies > 0)
        {
            rows.push(Row::ThreadFooter {
                root_id: post.id.clone(),
                reply_count: thread.reply_count,
                last_reply_at: thread.last_reply_at,
                participants: thread
                    .participants
                    .iter()
                    .map(|user_id| ThreadFace {
                        name: options
                            .author_names
                            .get(user_id)
                            .cloned()
                            .unwrap_or_else(|| user_id.clone()),
                        avatar_at: options.author_avatars.get(user_id).copied().unwrap_or(0),
                        user_id: user_id.clone(),
                    })
                    .collect(),
                unread_replies: thread.unread_replies,
                unread_mentions: thread.unread_mentions,
                following: thread.following,
            });
            // A footer ends the run: the next post gets its own header.
            previous = None;
        }
    }

    rows
}

/// The thread pane: one root and its replies, always flat and always ordered.
pub fn plan_thread(root: &Post, replies: &[Post], options: &PlanOptions) -> Vec<Row> {
    let mut flat = PlanOptions {
        utc_offset_minutes: options.utc_offset_minutes,
        collapse_window_ms: options.collapse_window_ms,
        thread_mode: ThreadMode::Flat,
        last_viewed_at: options.last_viewed_at,
        me_id: options.me_id.clone(),
        author_names: options.author_names.clone(),
        author_avatars: options.author_avatars.clone(),
        parsed: options.parsed.clone(),
        pending: options.pending.clone(),
        failed: options.failed.clone(),
        // A thread pane is never paged: it holds one root and all its replies.
        preceding: None,
        saved: options.saved.clone(),
        followed: options.followed.clone(),
        full_res: options.full_res,
        allow_svg: options.allow_svg,
        pixel_ratio: options.pixel_ratio,
    };
    // The divider belongs here now: `threads.last_viewed_at` is a real
    // per-thread watermark from the server, where in Phase 3 there was nothing
    // to place it against and it was zeroed out.
    flat.last_viewed_at = options.last_viewed_at;

    let mut posts: Vec<Post> = Vec::with_capacity(replies.len() + 1);
    posts.push(root.clone());
    posts.extend_from_slice(replies);
    plan_channel(&posts, &HashMap::new(), &flat)
}

#[cfg(test)]
mod tests;
