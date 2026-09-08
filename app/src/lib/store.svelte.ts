// The single source every view reads.
//
// Auger's rule, inherited: the UI only visualises the store, and a command's
// only job is to populate it. There must not be two render paths -- if a
// command writes the same data, the view must not change. The only accepted
// second path is a loading placeholder when the store has no entry for a key
// yet, which is why `read` returning `undefined` means "never fetched" and an
// empty array means "fetched, genuinely empty".
import type { SidebarGroup, ChannelSummary, Row } from "./api";

type Scope =
  | "rows"
  | "pages"
  | "emoji"
  | "unread"
  | "typing"
  | "channels"
  | "nav"
  | "meta"
  | "draft"
  | "settings"
  | "status"
  | "unread_since";

const memory: Record<string, unknown> = $state({});

const at = (scope: Scope, key: string) => `${scope}/${key}`;

export function read<T>(scope: Scope, key: string): T | undefined {
  return memory[at(scope, key)] as T | undefined;
}

export function write<T>(scope: Scope, key: string, value: T): void {
  memory[at(scope, key)] = value;
}

// ---- typed helpers, so views never stringify a scope by hand ----

export const rowsOf = (channelId: string) => read<Row[]>("rows", channelId);
export const setRows = (channelId: string, rows: Row[]) => write("rows", channelId, rows);

export interface UnreadState { messages: number; mentions: number; muted: boolean }
export const unreadOf = (channelId: string) => read<UnreadState>("unread", channelId);
export const setUnread = (channelId: string, unread: UnreadState) =>
  write("unread", channelId, unread);

/** Custom emoji ids by name, from whatever pages have been built.
 *
 *  Reference data like author names: accumulated rather than replaced, so
 *  switching channels does not lose what the last one resolved. */
export const emojiIds = () => read<Record<string, string>>("emoji", "ids") ?? {};
export function learnEmoji(found: Record<string, string>): void {
  const known = emojiIds();
  let changed = false;
  for (const [name, id] of Object.entries(found)) {
    if (known[name] !== id) {
      known[name] = id;
      changed = true;
    }
  }
  // Written only on a change: this runs on every page build.
  if (changed) write("emoji", "ids", { ...known });
}

export const channelList = () => read<ChannelSummary[]>("channels", "all");
/** Load preview images from the sites they come from.
 *
 *  This server runs no image proxy (`HasImageProxy: false`), so every preview
 *  image is a request from this machine to that host -- which tells it someone
 *  here read the message. On by default, as the official client is, but the
 *  card is still useful without one. */
export const previewImages = () => read<boolean>("settings", "previewImages") ?? true;
export const setPreviewImages = (on: boolean) => write("settings", "previewImages", on);

/** Draw a lone image from the file as uploaded rather than the server's
 *  preview rendition: sharper on a large screen, and several megabytes.
 *
 *  Part of the row request rather than a CSS choice, because which rendition is
 *  fetched decides the box the plan reserves. */
export const fullRes = () => read<boolean>("settings", "fullRes") ?? false;
export const setFullRes = (on: boolean) => write("settings", "fullRes", on);

export const sidebarGroups = () => read<SidebarGroup[]>("channels", "groups");
export const setSidebarGroups = (groups: SidebarGroup[]) =>
  write("channels", "groups", groups);
/** Whether unread channels are shown in their own leading group. */
export const separateUnreads = () => read<boolean>("channels", "separate") ?? false;
export const setSeparateUnreads = (separate: boolean) =>
  write("channels", "separate", separate);
export const setChannelList = (channels: ChannelSummary[]) => write("channels", "all", channels);

/** The thread the pane is showing, if any. */
export const activeThread = () => read<string>("nav", "thread");
export const setActiveThread = (rootId: string | undefined) =>
  write("nav", "thread", rootId);

/** Clears a thread's unread locally, so the footer stops saying "new" the
 *  moment it is opened rather than after the next refresh. */
export function markThreadRead(rootId: string): void {
  const channelId = activeChannel();
  if (!channelId) return;
  const rows = rowsOf(channelId);
  if (!rows) return;
  setRows(
    channelId,
    rows.map((row) =>
      row.kind === "thread_footer" && row.root_id === rootId
        ? { ...row, unread_replies: 0, unread_mentions: 0 }
        : row,
    ),
  );
}

export const activeChannel = () => read<string>("nav", "channel");
export const setActiveChannel = (channelId: string) => write("nav", "channel", channelId);

/** Ephemeral, and deliberately its own scope: typing was 72-93% of all
 *  websocket traffic, so it must never invalidate a message list.
 *
 *  Keyed by conversation rather than by channel: a reply being typed in a
 *  thread arrives on the thread's channel, so keying by channel alone showed
 *  "someone is typing" under a stream nobody was typing in. */
const typingKey = (channelId: string, rootId?: string) =>
  rootId ? `${channelId}/${rootId}` : channelId;

export const typingIn = (channelId: string, rootId?: string) =>
  read<string[]>("typing", typingKey(channelId, rootId)) ?? [];

export function noteTyping(channelId: string, userId: string, rootId?: string): void {
  const key = typingKey(channelId, rootId);
  const current = typingIn(channelId, rootId).filter((id) => id !== userId);
  write("typing", key, [...current, userId]);
  setTimeout(() => {
    write(
      "typing",
      key,
      typingIn(channelId, rootId).filter((id) => id !== userId),
    );
  }, 6000);
}

/** Drafts are per channel and in memory only, so switching away and back does
 *  not lose what was typed. Persisting them to SQLite is a later slice. */
export const draftOf = (channelId: string) => read<string>("draft", channelId) ?? "";
export const setDraft = (channelId: string, text: string) =>
  write("draft", channelId, text);

/** Posts per page.
 *
 *  A page is built once and then only rebuilt if its newest post changes --
 *  which only ever happens to the newest page. That is what bounds the cost of
 *  a live message: it used to rebuild every row the reader had paged back
 *  through (measured 36-49 ms at 1957 rows, and rising with depth), and now it
 *  rebuilds one page however deep the scrollback goes.
 *
 *  200 posts is a few screens under collapsed threads, so paging back is rare
 *  enough not to be felt and small enough to build in a couple of milliseconds.
 */
export const PAGE_POSTS = 200;

/** One built page of rows, oldest page first.
 *
 *  `oldest` is the cursor for asking for the page below this one. Whether more
 *  history exists is not recorded: the shell finds out by asking for the next
 *  page, which is the only answer that cannot be stale.
 */
export interface Page {
  rows: Row[];
  oldest: number | null;
}

export const pagesOf = (channelId: string) => read<Page[]>("pages", channelId) ?? [];

/** The newest page's fixed floor: posts at or after this belong to it.
 *
 *  Set from the first page of a visit, and moved only when that page is sealed.
 *  Without it the newest page would mean "the newest N posts", whose window
 *  slides forward as messages arrive and drops posts out of the bottom of it --
 *  a hole at the seam with the page below.
 */
export const floorOf = (channelId: string) => read<number>("pages", `${channelId}/floor`);
export const setFloor = (channelId: string, at: number) =>
  write("pages", `${channelId}/floor`, at);
export const clearFloor = (channelId: string) =>
  write("pages", `${channelId}/floor`, undefined);

/** A limit for the newest page, whose real bound is its floor. Generous enough
 *  never to bite in practice, present so a bug cannot ask for a whole channel. */
export const NEWEST_PAGE_CEILING = 2000;
/** Empties a channel's paging state for a fresh visit, leaving the list in its
 *  *unknown* state rather than its *empty* one.
 *
 *  `setPages(id, [])` writes an empty row array, and an empty row array means
 *  "this channel has no messages" -- the list draws "Nothing here yet." for it.
 *  That is a lie while the first page is still being fetched, and it is what the
 *  reader saw on every reload. */
export function clearPages(channelId: string): void {
  write("pages", channelId, []);
  write("rows", channelId, undefined);
}

export function setPages(channelId: string, pages: Page[]): void {
  write("pages", channelId, pages);
  // The list consumes one flat array: pages are a loading strategy, not
  // something the reader should be able to see.
  write("rows", channelId, flatten(pages));
}

/** Pages joined into the single array the list draws, with each calendar day
 *  keeping exactly one separator.
 *
 *  Every page builds its own rows, so two pages that touch the same day each
 *  emit a separator for it -- and the list keys rows by identity, which for a
 *  separator is its day. Two rows then share a key, which breaks both the keyed
 *  `{#each}` and the measured-height cache, and makes restoring the reader's
 *  position after a page lands pick whichever came first. Observed: a channel
 *  with 951 rows and 950 distinct keys, where `day/20137` appeared at index 209
 *  and again at 214.
 *
 *  The first occurrence is the one kept: pages run oldest first, so it is the
 *  one that sits above that day's earliest message.
 */
function flatten(pages: Page[]): Row[] {
  const rows: Row[] = [];
  const days = new Set<number>();
  for (const page of pages) {
    for (const row of page.rows) {
      if (row.kind === "date_separator") {
        if (days.has(row.epoch_day)) continue;
        days.add(row.epoch_day);
      }
      rows.push(row);
    }
  }
  return rows;
}

/** Where this visit's "New messages" divider sits: `last_viewed_at` captured
 *  when the channel was opened, held so marking read does not erase the line
 *  while it is still being read. */
export const unreadSinceOf = (channelId: string) =>
  read<number>("unread_since", channelId) ?? 0;
export const setUnreadSince = (channelId: string, at: number) =>
  write("unread_since", channelId, at);

export interface Meta {
  meId: string;
  username: string;
  threadMode: string;
  displayMode: string;
  /** The server's `MaxFileSize`, so the composer can refuse an oversized file
   *  before spending the upload. */
  maxFileSize: number;
}
export const meta = () => read<Meta>("meta", "app");

/** Whether the websocket is up.
 *
 *  Its own entry rather than a field of `meta`, and that is the whole point:
 *  connection deltas arrive from the moment the shell subscribes, which is
 *  *before* bootstrap returns -- so a "connected" that lived in `meta` was
 *  first dropped (nothing to patch yet) and then overwritten by bootstrap
 *  writing `connected: false`. The socket was live and the badge said OFFLINE
 *  for the rest of the session. Here, an early delta lands and nothing else
 *  clobbers it. */
/** Presence per person, from the batch fetch and from `status_change` events.
 *
 *  Its own scope, like typing: a dot changing colour must never invalidate a
 *  message row. */
export const statusOf = (userId: string) => read<string>("status", userId);
export const setStatus = (userId: string, status: string) =>
  write("status", userId, status);
export function learnStatuses(found: Record<string, string>): void {
  for (const [userId, status] of Object.entries(found)) setStatus(userId, status);
}

/** What a clicked mention or `~channel` in a message body should do.
 *
 *  A registry rather than props: `Nodes` renders itself recursively, so passing
 *  two callbacks down would thread them through every branch -- a table cell, a
 *  list item, a blockquote -- to reach the one leaf that uses them. The shell
 *  registers these once at mount; nothing else may write them.
 */
let openProfileHandler: ((username: string, at: { x: number; y: number }) => void) | undefined;
let followChannelHandler: ((name: string) => void) | undefined;
let followPermalinkHandler: ((channelId: string, postId: string) => void) | undefined;

export function onMessageLinks(handlers: {
  profile: (username: string, at: { x: number; y: number }) => void;
  channel: (name: string) => void;
  permalink: (channelId: string, postId: string) => void;
}): void {
  openProfileHandler = handlers.profile;
  followChannelHandler = handlers.channel;
  followPermalinkHandler = handlers.permalink;
}

export const openProfile = (username: string, at: { x: number; y: number }) =>
  openProfileHandler?.(username, at);
export const followChannel = (name: string) => followChannelHandler?.(name);
export const followPermalink = (channelId: string, postId: string) =>
  followPermalinkHandler?.(channelId, postId);

export const connected = () => read<boolean>("meta", "connected") ?? false;
export const setConnected = (up: boolean) => write("meta", "connected", up);
export const setMeta = (value: Meta) => write("meta", "app", value);
export function patchMeta(patch: Partial<Meta>): void {
  const current = meta();
  if (current) setMeta({ ...current, ...patch });
}
