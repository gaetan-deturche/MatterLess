// Mirrors of what the Rust side serialises. The row-plan shape is pinned by a
// test in matterless-render, so these types are a contract, not a guess.
import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type Node =
  | { t: "text"; value: string }
  | { t: "emphasis"; children: Node[] }
  | { t: "strong"; children: Node[] }
  | { t: "strike"; children: Node[] }
  | { t: "inline_code"; value: string }
  | { t: "code_block"; language: string | null; value: string }
  | { t: "link"; href: string; children: Node[] }
  /** `![alt](url)` — how Mattermost's own GIF picker posts. */
  | { t: "image"; url: string; alt: string }
  | { t: "user_mention"; username: string; everyone: boolean }
  | { t: "channel_link"; name: string }
  | { t: "emoji"; name: string; unicode: string | null }
  | { t: "inline_math"; value: string }
  | { t: "paragraph"; children: Node[] }
  | { t: "heading"; level: number; children: Node[] }
  | { t: "blockquote"; children: Node[] }
  | { t: "list"; ordered: boolean; items: Node[][] }
  | { t: "table"; head: Node[][]; rows: Node[][][] }
  | { t: "rule" }
  | { t: "soft_break" }
  | { t: "hard_break" };

export interface ReactionSummary {
  /** The character for a standard emoji, from the same table bodies use. */
  unicode: string | null;
  emoji: string;
  count: number;
  mine: boolean;
  /** Who reacted, in order, the viewer as "You". Capped at eight -- `count` is
   *  all of them, so the difference is "and N others". */
  names: string[];
}
/** Which of the server's three renditions of a file to draw.
 *
 *  Measured sizes: `thumb` is capped at 120x100, `preview` at 1920 wide, and
 *  `original` is whatever was uploaded. */
export type ImageVariant = "thumb" | "preview" | "original";

export interface FileRef {
  id: string;
  name: string;
  extension: string;
  size: number;
  mime_type: string;
  width: number;
  height: number;
  /** Drawn inline as a picture rather than as a card. */
  image: boolean;
  variant: ImageVariant;
  /** A ~1 KB base64 JPEG from the post's metadata: a placeholder that costs no
   *  request. */
  mini_preview: string | null;
  /** The box Rust reserved for it: never larger than the chosen variant's own
   *  pixels, so nothing is upscaled, and known before the bytes arrive so the
   *  virtualiser's height estimates stay honest. */
  box_width: number;
  box_height: number;
  archived: boolean;
}
export interface AttachmentField { title: string; value: Node[]; short: boolean }
export interface Attachment {
  color: string | null;
  pretext: Node[];
  title: string | null;
  title_link: string | null;
  text: Node[];
  fields: AttachmentField[];
}

/** A preview under a message: a fetched page, or another message quoted. */
export type Preview =
  | {
      kind: "page";
      url: string;
      title: string;
      description: string;
      site_name: string;
      /** Already fitted to its box in Rust, so the row's height is known before
       *  the image loads. */
      image: { url: string; width: number; height: number } | null;
    }
  | {
      kind: "permalink";
      post_id: string;
      channel_id: string;
      channel_label: string;
      author_name: string;
      create_at: number;
      nodes: Node[];
    };

export interface PostRow {
  post_id: string;
  /** The thread this message belongs to, empty when it is the root itself. */
  root_id: string;
  author_id: string;
  /** Resolved in Rust against the server's name-display preference. */
  author_name: string;
  create_at: number;
  update_at: number;
  edited: boolean;
  nodes: Node[];
  reactions: ReactionSummary[];
  files: FileRef[];
  attachments: Attachment[];
  /** Posted by an integration rather than a person. */
  bot: boolean;
  /** The author's `last_picture_update`: the avatar's version. */
  avatar_at: number;
  body_is_attachment_only: boolean;
  /** An optimistic send the server has not confirmed. */
  pending: boolean;
  /** The send failed; the row stays so it can be retried. */
  failed: boolean;
  /** Pinned to the channel, for everyone. */
  pinned: boolean;
  /** Saved by this reader (a `flagged_post` preference on the server). */
  saved: boolean;
  /** This reader follows the thread this message belongs to. */
  following: boolean;
  /** Link and permalink previews the server attached. */
  previews: Preview[];
}

export interface ThreadFace {
  user_id: string;
  name: string;
  /** `last_picture_update`: the avatar's version. */
  avatar_at: number;
}

export type Row =
  | { kind: "date_separator"; epoch_day: number }
  | { kind: "unread_divider" }
  | { kind: "post"; post: PostRow }
  | { kind: "continuation"; post: PostRow }
  | {
      kind: "system";
      post_id: string;
      post_type: string;
      nodes: Node[];
      /** What happened, in words: composed in Rust because it needs `props`. */
      text: string;
    }
  | { kind: "deleted_root"; post_id: string }
  | {
      kind: "thread_footer";
      root_id: string;
      reply_count: number;
      last_reply_at: number;
      participants: ThreadFace[];
      /** Replies not yet read, from the server's per-thread count. */
      unread_replies: number;
      /** Of those, the ones that named you. */
      unread_mentions: number;
      following: boolean;
    };

export interface ChannelSummary {
  id: string;
  /** The other person in a DM, whose face labels it. Null for channels and for
   *  group messages, which have several. */
  counterpart_id: string | null;
  team_id: string;
  display_name: string;
  channel_type: string;
  last_post_at: number;
  unread: number;
  mentions: number;
  muted: boolean;
}

export interface Bootstrap {
  me_id: string;
  username: string;
  thread_mode: string;
  display_mode: string;
  channels: ChannelSummary[];
  from_cache: boolean;
  /** The server's `MaxFileSize` in bytes: 150 MB on this one. */
  max_file_size: number;
  /** Whether the websocket is up now -- the level, since the deltas are edges. */
  connected: boolean;
  /** This reader's own presence. */
  own_status: string;
}

export interface Timings {
  page_ms: number;
  threads_ms: number;
  names_ms: number;
  cache_ms: number;
  parse_ms: number;
  plan_ms: number;
  cache_hits: number;
  cache_misses: number;
}
export interface RowsPayload {
  channel_id: string;
  rows: Row[];
  /** Oldest create_at in this window; the cursor for paging further back. */
  oldest_in_window: number | null;
  /** The page came back full, so more posts exist locally below it. */
  window_full: boolean;
  /** Newest `create_at` in the page: where the page above it starts. */
  newest_in_window: number | null;
  /** Custom emoji this page uses: name to id, for the image route. A standard
   *  emoji carries its character on the node itself. */
  emoji: Record<string, string>;
  build_ms: number;
  timings: Timings;
}

export type UiDelta =
  | {
      kind: "channel";
      channel_id: string;
      /** Frame-read to delta-emit, measured in Rust. Null for a REST refresh,
       *  which has no frame to measure from. */
      ws_ms: number | null;
      /** Wall clock at emit, for pricing the IPC hop. */
      emitted_at_ms: number | null;
    }
  | { kind: "unread"; channel_id: string; messages: number; mentions: number; muted: boolean }
  | { kind: "status"; user_id: string; status: string }
  | {
      kind: "typing";
      channel_id: string;
      user_id: string;
      /** The thread being typed in, empty for the channel itself. */
      root_id: string;
    }
  | { kind: "sidebar"; team_id: string }
  | {
      kind: "notify";
      channel_id: string;
      post_id: string;
      author: string;
      /** The conversation's name, or what kind of conversation it is. */
      channel: string;
      preview: string;
      /** A direct or group message, where the person is the conversation. */
      direct: boolean;
    }
  | { kind: "connection"; connected: boolean; resync: boolean };

export const session = () => invoke<{ signed_in: boolean; server: string }>("session");
export const signIn = (login_id: string, password: string, mfa_token?: string) =>
  invoke<string>("sign_in", { loginId: login_id, password, mfaToken: mfa_token ?? null });
export const bootstrap = () => invoke<Bootstrap>("bootstrap");

export interface SidebarGroup {
  id: string;
  display_name: string;
  /** The server's order within a team; breaks ties between custom categories. */
  sort_order: number;
  /** `favorites`, `custom`, `channels` or `direct_messages`. */
  category_type: string;
  team_id: string;
  /** The team's display name, shown when more than one team contributes. */
  team_name: string;
  collapsed: boolean;
  channels: ChannelSummary[];
}
export interface SidebarPayload {
  groups: SidebarGroup[];
  /** The reader groups unread channels separately; the split is applied here,
   *  where which channels are unread is live state. */
  separate_unreads: boolean;
}

/** The sidebar as the reader arranged it server-side: teams, then categories,
 *  with direct messages merged into one group. */
export const sidebar = (meId: string, displayMode: string) =>
  invoke<SidebarPayload>("sidebar", { meId, displayMode });

export interface ThreadPayload {
  root_id: string;
  channel_id: string;
  rows: Row[];
  reply_count: number;
  unread_replies: number;
  following: boolean;
  build_ms: number;
}
/** One thread's rows, from SQLite. */
/** The viewer's device pixel ratio, so an image is never asked to cover more
 *  device pixels than it has. */
export const pixelRatio = () => window.devicePixelRatio || 1;

export const threadRows = (
  rootId: string,
  meId: string,
  displayMode: string,
  fullRes = false,
) =>
  invoke<ThreadPayload>("thread_rows", {
    request: {
      rootId,
      meId,
      displayMode,
      utcOffsetMinutes: utcOffsetMinutes(),
      fullRes,
      pixelRatio: pixelRatio(),
    },
  });
/** Fetches the thread from the server and stores it. Resolves to the count. */
export const refreshThread = (rootId: string) =>
  invoke<number>("refresh_thread", { rootId });
export const markThreadRead = (rootId: string) =>
  invoke<void>("mark_thread_read", { rootId });
export const setThreadFollowing = (rootId: string, following: boolean) =>
  invoke<void>("set_thread_following", { rootId, following });

export interface ThreadTotals {
  followed: number;
  unread_threads: number;
  unread_mentions: number;
}
/** Catalogues every custom emoji the server has, resolving to name -> id.
 *
 *  The whole map, because the shell otherwise only learns the names a page
 *  payload happened to carry. */
export const refreshEmoji = () => invoke<Record<string, string>>("refresh_emoji");

/** Re-reads followed threads across every team, deduped by thread id. */
export const refreshThreads = (meId: string) =>
  invoke<ThreadTotals>("refresh_threads", { meId });
/** The second half of "paint from cache, then refresh": re-reads every
 *  membership, so unread and mention counts stop being a session old. */
export const refreshMembership = (meId: string, displayMode: string) =>
  invoke<ChannelSummary[]>("refresh_membership", { meId, displayMode });
export interface ChannelOpened {
  /** Where this visit's "New messages" divider belongs. */
  divider_at: number;
  /** Ask for a refresh *after* rendering, not before. */
  needs_refresh: boolean;
}

export const openChannel = (channelId: string) =>
  invoke<ChannelOpened>("open_channel", { channelId });

/** Writes to the server and clears the badge in every Mattermost client, so it
 *  is called only after a few seconds of *focused* dwell. */
export const markRead = (channelId: string) => invoke<void>("mark_read", { channelId });
export const refresh = (channelId: string) => invoke<void>("refresh", { channelId });
/** Resolves to whether anything older still exists. */
export const loadOlder = (channelId: string) => invoke<boolean>("load_older", { channelId });
export const setFocus = (focused: boolean) => invoke<void>("set_focus", { focused });

export interface BadgeReport {
  /** Something unread somewhere, in a channel that is not muted. */
  any_unread: boolean;
  /** The number on the badge: mentions plus unread in followed channels. */
  attention: number;
  /** The parts, so a wrong total is diagnosable from the log. */
  mentions: number;
  /** Mentions inside followed threads, which the server counts separately. */
  thread_mentions: number;
  followed_unread: number;
}

/** Redraws the taskbar overlay badge. The rule lives in Rust because it depends
 *  on each channel's notify_props, which the sidebar payload does not carry. */
export const updateBadge = (meId: string) => invoke<BadgeReport>("update_badge", { meId });
/** Raises a Windows toast that focuses the app when clicked. */
export const raiseToast = (channelId: string, title: string, body: string) =>
  invoke<void>("raise_toast", { channelId, title, body });

/** Flashes the taskbar button. Only worth calling while unfocused. */
export const flashTaskbar = (urgent: boolean) => invoke<void>("flash_taskbar", { urgent });
export const clearAttention = () => invoke<void>("clear_attention");
export const sendPost = (
  channelId: string,
  message: string,
  rootId?: string,
  fileIds: string[] = [],
) =>
  invoke<string>("send_post", {
    channelId,
    message,
    rootId: rootId ?? null,
    fileIds,
  });

/** How an upload ended, emitted on the `attachment` event.
 *
 *  An event rather than a reply because the command hands back the IPC thread
 *  as soon as it has the bytes -- a 20 MB drop takes long enough that the tray
 *  has to be able to say "uploading". */
export interface AttachmentEvent {
  attach_id: string;
  file: FileRef | null;
  error: string | null;
}

/** Uploads one file, ahead of the post that will carry it.
 *
 *  The bytes go as a raw IPC body: as JSON they would be a number array, which
 *  is roughly four characters per byte to serialise and parse. Everything else
 *  travels in headers, and the filename is percent-encoded because a header
 *  value is ASCII while a filename is not.
 *
 *  Resolves when the upload has *started*; watch for `attachment` with the same
 *  `attachId` for the outcome. */
export const attachFile = (
  attachId: string,
  channelId: string,
  name: string,
  bytes: ArrayBuffer,
) =>
  invoke<void>("attach_file", bytes, {
    headers: {
      "x-attach-id": attachId,
      "x-channel-id": channelId,
      "x-file-name": encodeURIComponent(name),
    },
  });

/** Subscribes to upload outcomes. Resolves to the unsubscribe function, which
 *  is the shape `listen` already has. */
export const onAttachment = (handle: (payload: AttachmentEvent) => void) =>
  listen<AttachmentEvent>("attachment", (event) => handle(event.payload));

/** One completion offered for `@`, `~` or `:`. */
export interface Suggestion {
  /** What gets inserted. */
  value: string;
  label: string;
  /** Second line: a real name, or a standard emoji's own glyph. */
  detail: string;
  /** A user id, channel id or custom emoji id, for its image. */
  id: string;
  kind: "user" | "channel" | "emoji";
  avatar_at: number;
  /** `O`/`P`/`D`/`G` for a channel; empty otherwise. */
  channel_type: string;
}

/** Completions for what is being typed, answered from the local store first.
 *
 *  Matching is a subsequence, not a prefix: `al` finds `ada.lovelace`. */
export const suggest = (kind: string, query: string, channelId?: string, limit?: number) =>
  invoke<Suggestion[]>("suggest", {
    kind,
    query,
    channelId: channelId ?? null,
    displayMode: "username",
    // Omitted for a completion list under a caret; a browsing grid asks for
    // more than the eight that suits one.
    limit: limit ?? null,
  });

/** A group of standard emoji, as a picker shows them. */
export interface EmojiCategory {
  label: string;
  /** `[name, character]`, in the order a reader expects to find them. */
  emoji: [string, string][];
}

/** Every standard emoji, grouped. Sent whole and once: it never changes without
 *  a server upgrade, and a request per category would put a round trip between
 *  the reader and a grid of tiles. */
export const emojiCategories = () => invoke<EmojiCategory[]>("emoji_categories");

/** One search result, ready to draw. */
export interface SearchHit {
  post_id: string;
  channel_id: string;
  /** The channel as the sidebar labels it: a DM says who it is with. */
  channel_label: string;
  author_name: string;
  create_at: number;
  nodes: Node[];
  local_only: boolean;
}

/** Searches messages: the local index first, then the server, merged and
 *  deduped by post id across teams. */
export const searchMessages = (query: string, meId: string, displayMode: string) =>
  invoke<SearchHit[]>("search_messages", {
    query,
    meId,
    displayMode,
    // A date modifier means the reader's own day, not a day in UTC.
    utcOffsetMinutes: utcOffsetMinutes(),
  });

/** Channels and people in one ranked list, for the quick switcher. */
export const switcher = (query: string) =>
  invoke<Suggestion[]>("switcher", { query, displayMode: "full_name" });

/** Finds or creates the direct message channel with someone, returning its id. */
export const openDirectMessage = (meId: string, userId: string) =>
  invoke<string>("open_direct_message", { meId, userId });

/** Edits a message. The store is written from the server's reply, which is
 *  what carries the real `update_at`. */
export const editPost = (postId: string, message: string) =>
  invoke<void>("edit_post", { postId, message });

/** Deletes a message. */
export const deletePostNow = (postId: string) =>
  invoke<void>("delete_post_now", { postId });

/** Everything a profile card shows. */
export interface Profile {
  user_id: string;
  username: string;
  display_name: string;
  full_name: string;
  nickname: string;
  email: string;
  avatar_at: number;
  status: string;
  /** The reader themselves, so the card offers notes to self. */
  is_me: boolean;
}

/** A profile by username *or* user id: the local users table first, then the
 *  server. Both, because a mention knows a name and a message row knows an id
 *  -- and a displayed name is not always a username. */
export const profile = (who: string, meId: string, displayMode: string) =>
  invoke<Profile>("profile", { who, meId, displayMode });

/** The channel a `~link` names, or null when the reader is not in it. */
export const channelByName = (name: string) =>
  invoke<string | null>("channel_by_name", { name });

/** Presence for the people on screen, in one request. */
export const statuses = (userIds: string[]) =>
  invoke<Record<string, string>>("statuses", { userIds });

/** Sets this reader's own presence, returning what the server settled on. */
export const setMyStatus = (meId: string, status: string) =>
  invoke<string>("set_my_status", { meId, status });

/** Tells the server this reader is typing.
 *
 *  Called on every keystroke: the throttle is in Rust, so there is one answer
 *  to how often it actually goes out (the server asks for one every 5000 ms). */
export const sendTyping = (channelId: string, rootId?: string) =>
  invoke<void>("send_typing", { channelId, rootId: rootId ?? null });

/** The message as it was typed, for "Copy Text". Fetched on demand rather than
 *  carried on every row. */
export const postText = (postId: string) => invoke<string>("post_text", { postId });

/** A link to the message: `<server>/<team>/pl/<post id>`. */
/** How far an upload has got, while it runs. */
export interface AttachmentProgress {
  attach_id: string;
  sent: number;
  total: number;
}

export const onAttachmentProgress = (handle: (payload: AttachmentProgress) => void) =>
  listen<AttachmentProgress>("attachment.progress", (event) => handle(event.payload));

/** Stops an upload still on the wire. There is no resumable upload and no
 *  cancel endpoint: this drops the connection, which is the only way. */
export const cancelAttachment = (attachId: string) =>
  invoke<boolean>("cancel_attachment", { attachId });

/** Points this install at a Mattermost server, checking it answers first.
 *
 *  Returns the tidied URL actually stored. */
export const setServer = (url: string) => invoke<string>("set_server", { url });

/** A public channel the reader could join. */
export interface Joinable {
  id: string;
  name: string;
  display_name: string;
  purpose: string;
  channel_type: string;
  /** Already a member: shown as "open" rather than "join". */
  joined: boolean;
}

/** The public channels of a team, from the server: the point is the ones the
 *  reader is *not* in, which have never been synced. */
export const browseChannels = (teamId: string, query: string) =>
  invoke<Joinable[]>("browse_channels", { teamId, query });

export const joinChannel = (channelId: string, meId: string) =>
  invoke<void>("join_channel", { channelId, meId });

/** Leaves a channel and forgets it locally. Refused by the server for a direct
 *  or group message: those are left by hiding them, not by membership. */
export const leaveChannel = (channelId: string, meId: string) =>
  invoke<void>("leave_channel", { channelId, meId });

/** Finds or creates the group conversation with these people. */
export const openGroupMessage = (meId: string, userIds: string[]) =>
  invoke<string>("open_group_message", { meId, userIds });

/** Where a jumped-to post sits, once its surroundings are local. */
export interface Island {
  create_at: number;
  oldest: number;
  newest: number;
  /** True when the window reached the newest post, so there is no gap. */
  reaches_present: boolean;
}

/** Brings down the posts either side of one, so a jump has context.
 *
 *  Replaces walking backwards a page at a time, which cost a request per page
 *  and simply failed for anything older than the reader's patience. */
export const fetchAround = (channelId: string, postId: string, span = 60) =>
  invoke<Island>("fetch_around", { channelId, postId, span });

/** The reader's saved messages, newest first. */
export const savedMessages = (meId: string, displayMode: string) =>
  invoke<SearchHit[]>("saved_messages", { meId, displayMode });

/** The messages pinned in a channel, newest first. */
export const pinnedMessages = (channelId: string, meId: string, displayMode: string) =>
  invoke<SearchHit[]>("pinned_messages", { channelId, meId, displayMode });

/** Asks the server to remind the reader about a post.
 *
 *  `targetTime` is epoch *seconds*, computed here: which instant "tomorrow
 *  morning" is depends on this machine's clock and timezone. The server posts
 *  the reminder itself, so nothing is scheduled locally. */
export const setReminder = (postId: string, targetTime: number) =>
  invoke<void>("set_reminder", { postId, targetTime });

export const postPermalink = (postId: string) =>
  invoke<string>("post_permalink", { postId });

/** Marks the channel unread from this message down. */
export const markPostUnread = (meId: string, postId: string) =>
  invoke<void>("mark_post_unread", { meId, postId });

/** Saves or unsaves a message for this reader. */
export const setPostSaved = (meId: string, postId: string, saved: boolean) =>
  invoke<void>("set_post_saved", { meId, postId, saved });

/** Pins or unpins a message. Channel-wide: everyone sees it. */
export const setPostPinned = (postId: string, pinned: boolean) =>
  invoke<void>("set_post_pinned", { postId, pinned });

/** Forgets an upload the composer no longer wants to send. */
export const releaseAttachment = (fileId: string) =>
  invoke<boolean>("release_attachment", { fileId });

/** Writes an attachment into the downloads folder, resolving to its path. */
export const saveAttachment = (fileId: string, name: string) =>
  invoke<string>("save_attachment", { fileId, name });
/** Adds or removes the viewer's reaction. Resolves to whether it was added. */
export const toggleReaction = (postId: string, emoji: string) =>
  invoke<boolean>("toggle_reaction", { postId, emoji });

export const retryPost = (pendingPostId: string) =>
  invoke<void>("retry_post", { pendingPostId });
export const discardPost = (pendingPostId: string) =>
  invoke<void>("discard_post", { pendingPostId });

/** Minutes east of UTC, so Rust can put day boundaries where the viewer sees them. */
export const utcOffsetMinutes = () => -new Date().getTimezoneOffset();

export const channelRows = (
  channelId: string,
  meId: string,
  threadMode: string,
  displayMode: string,
  anchor: number | null,
  since: number | null,
  unreadSince: number,
  limit: number,
  fullRes = false,
) =>
  invoke<RowsPayload>("channel_rows", {
    request: {
      channelId,
      meId,
      threadMode,
      displayMode,
      utcOffsetMinutes: utcOffsetMinutes(),
      anchor,
      since,
      unreadSince,
      limit,
      fullRes,
      pixelRatio: pixelRatio(),
    },
  });

/** One stream for every delta rather than an event type per mutation. */
export async function subscribe(onDelta: (delta: UiDelta) => void): Promise<void> {
  const channel = new Channel<UiDelta>();
  channel.onmessage = onDelta;
  await invoke("subscribe", { channel });
}
