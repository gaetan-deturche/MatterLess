// Row heights and offsets for the message list.
//
// The list renders only the rows inside the viewport, so paint cost follows the
// *window*, not the history: measured at 0.07-0.23 ms per row mounted, a
// 2000-row channel cost 456 ms to paint while adding one row to that same list
// cost 1.8 ms. Mounting was the whole cost, so mounting a viewport's worth is
// the whole fix.
//
// That leaves the scrollbar needing a total height for rows nobody has seen.
// Each row kind gets an estimate, every row that mounts reports its real height,
// and the offsets are a prefix sum over the two -- so the estimate is only ever
// wrong for history that has not been on screen yet, and is corrected the moment
// it is.
import type { Row } from "./api";
import * as log from "./log";
import { previewImages } from "./store.svelte";

/** Starting guesses, in pixels, until this channel has measured enough of its
 *  own rows to say better.
 *
 *  A bad estimate is what the reader feels as the list twitching while they
 *  scroll through history nobody has measured yet, so these are only the
 *  opening bid -- see `learned`.
 */
const ESTIMATES: Record<Row["kind"], number> = {
  post: 68,
  continuation: 26,
  date_separator: 34,
  unread_divider: 30,
  thread_footer: 30,
  system: 24,
  deleted_root: 24,
};

/** Rows above and below the viewport that are mounted anyway.
 *
 *  Scrolling fast enough to outrun a frame would otherwise show blank space;
 *  600px is about half a screen, which is enough at wheel speed.
 */
const OVERSCAN_PX = 600;

/** Measurements needed before a channel's own average beats the constant. */
const ENOUGH_SAMPLES = 8;

/** A stable identity for a row, so a measured height survives a rebuild.
 *
 *  Never positional. Keying the odd row kinds by index looked harmless -- their
 *  heights barely vary -- but prepending a page shifts every index below it, so
 *  a row would inherit the measured height of whatever used to sit at its
 *  position. Every kind has something of its own to key on, and there is only
 *  ever one unread divider.
 */
export function rowKey(row: Row): string {
  switch (row.kind) {
    case "post":
    case "continuation":
      return row.post.post_id;
    case "thread_footer":
      return `footer/${row.root_id}`;
    case "date_separator":
      return `day/${row.epoch_day}`;
    case "system":
      return `system/${row.post_id}`;
    case "deleted_root":
      return `deleted/${row.post_id}`;
    case "unread_divider":
      return "unread";
  }
}

export interface Layout {
  /** Cumulative top of each row; one longer than `rows`, so the last entry is
   *  the total height. */
  offsets: number[];
  total: number;
}

/** Measured heights, keyed by row key, per channel.
 *
 *  Kept out of Svelte's reactive state on purpose: a measurement must not
 *  invalidate the render that produced it, or every scroll becomes a loop.
 */
const measured = new Map<string, Map<string, number>>();

/** What this channel's rows actually turned out to be, per kind.
 *
 *  A constant estimate is wrong in a consistent direction -- one channel writes
 *  one-liners, another writes paragraphs -- and every wrong estimate is a
 *  correction the reader feels when they scroll into it. Averaging what has
 *  been measured makes the unmeasured rows above much closer to the truth.
 */
const learned = new Map<string, Map<Row["kind"], { total: number; count: number }>>();

/** Roughly how many characters fit on a line of a message body.
 *
 *  Derived from the column's *measured* width, not assumed. It was a constant
 *  95, which is a 660px column -- and at a 1280px window the column is nearer
 *  990px, so every wrap estimate here was a third too pessimistic. Measured
 *  before this changed: `textExtra` claiming 80px of text on a row 67px tall,
 *  because 382 characters were called five lines where two were drawn. */
const AVERAGE_CHAR_PX = 6.95;
const DEFAULT_CONTENT_WIDTH = 660;

/** The message column's content width, as last measured. */
let contentWidth = DEFAULT_CONTENT_WIDTH;

/** Told by the list, whose element is the one that actually has a width.
 *
 *  A change invalidates the learned per-kind bases -- each was recorded as
 *  `height - knownExtra - textExtra` at the old width -- and unsettles the rows
 *  whose height was fixed, so they can learn their new one. Measured heights
 *  are kept: a mounted row re-measures immediately, and an unmounted one is
 *  better off with a stale height than with none. */
export function setContentWidth(width: number): void {
  if (width < 200 || Math.abs(width - contentWidth) < 8) return;
  contentWidth = width;
  // A break counts as a line's worth of characters, so these depend on the
  // width too.
  lengths.clear();
  learned.clear();
  settled.clear();
  seenAgain.clear();
}

function charsPerLine(): number {
  return Math.max(20, Math.round(contentWidth / AVERAGE_CHAR_PX));
}
/** A line of body text, in pixels: 14px at 1.45 line-height. */
const LINE = 20;

/** Characters in a row's body, cached by row key.
 *
 *  Walked once per row: the tree is small, but the layout is recomputed on
 *  every measurement and this must not be part of that cost. */
const lengths = new Map<string, number>();

function charactersIn(nodes: readonly unknown[]): number {
  let total = 0;
  for (const node of nodes as { t?: string; value?: string; children?: unknown[]; items?: unknown[][] }[]) {
    if (typeof node.value === "string") {
      // A code block is pre-wrapped, so its newlines are real lines.
      total +=
        node.t === "code_block"
          ? node.value.length + node.value.split("\n").length * charsPerLine()
          : node.value.length;
    }
    if (node.children) total += charactersIn(node.children);
    if (node.items) for (const item of node.items) total += charactersIn(item);
    // A break ends a line wherever it lands.
    if (node.t === "soft_break" || node.t === "hard_break") total += charsPerLine();
  }
  return total;
}

/** Height a row's *text* adds beyond its first line.
 *
 *  The learned per-kind average is one number for every message in a channel,
 *  so a three-line post and a twenty-line post get the same guess -- and the
 *  difference is exactly what the reader feels as a jump when they scroll into
 *  unmeasured history. Measured before this existed: corrections of 90 to 195
 *  pixels while paging back. Subtracted again when a measurement teaches the
 *  average, so the average stays "what a one-line row costs".
 */
function textExtra(row: Row): number {
  if (row.kind !== "post" && row.kind !== "continuation") return 0;
  const key = rowKey(row);
  let characters = lengths.get(key);
  if (characters === undefined) {
    characters = charactersIn(row.post.nodes ?? []);
    lengths.set(key, characters);
  }
  const lines = Math.max(1, Math.ceil(characters / charsPerLine()));
  return (lines - 1) * LINE;
}

/** Height a row carries that does not need measuring to be known.
 *
 *  An attachment's box comes from the plan, so an image post can be estimated
 *  within a few pixels instead of at a text row's height -- which is the
 *  difference between scrolling smoothly into unmeasured history and the list
 *  lurching 300px when the row mounts. Subtracted again before a measurement
 *  teaches the average, so one screenshot does not make every text row's
 *  estimate too tall.
 */
/** A reaction pill's own height: 11.5px text on the inherited 1.5 line-height,
 *  plus a pixel of padding and a pixel of border at each edge. */
const PILL_HEIGHT = 21;
/** `.reactions` is a wrapping flex row, so its gap separates wrapped lines as
 *  well as neighbouring pills. */
const PILL_GAP = 5;
/** The row's own margin, above and below together. */
const PILL_MARGINS = 4;
/** Roughly the width a row of pills fills before it wraps. Same basis as
 *  CHARS_PER_LINE: the stream's content box at the default window width. */
const REACTION_LINE_PX = 660;

/** The height a post's reactions add.
 *
 *  Reactions belong in the reserved height for the same reason attachments do:
 *  the row data says they are there before the row is ever mounted. Left out,
 *  they were wrong twice over -- a reacted post was under-reserved by a whole
 *  pill row, and because `learn` subtracts only what is accounted for here,
 *  reacted posts also dragged up the learned average for *every* post, so the
 *  ones without reactions were over-reserved. Content drifting both up and down
 *  on a channel switch is that pair of errors.
 */
function reactionExtra(row: Row): number {
  if (row.kind !== "post" && row.kind !== "continuation") return 0;
  const reactions = row.post.reactions ?? [];
  if (reactions.length === 0) return 0;

  let width = 0;
  for (const reaction of reactions) {
    // Border, padding, the glyph, the gap before the tally, and the tally's
    // digits. A custom emoji draws as an image about a glyph wide.
    width += 2 + 14 + 16 + 4 + String(reaction.count).length * 7 + PILL_GAP;
  }
  const lines = Math.max(1, Math.ceil(width / contentWidth));
  return lines * PILL_HEIGHT + (lines - 1) * PILL_GAP + PILL_MARGINS;
}



/** A box in a wrapping flex row. `scales` is true for a picture, whose height
 *  follows its width, and false for a card, whose height does not. */
interface WrapBox {
  width: number;
  height: number;
  scales: boolean;
}

/** The height a wrapping flex row comes to: boxes laid left to right, wrapping
 *  when the next one does not fit, each line as tall as its tallest box.
 *
 *  Worth simulating rather than approximating, because both ways of
 *  approximating it are wrong in a direction that matters. Taking the tallest
 *  box assumes one line and under-reserves the moment the pictures wrap; summing
 *  them assumes one per line and over-reserves whenever they sit side by side.
 */
function wrappedHeight(boxes: WrapBox[], gap: number): number {
  let stacked = 0;
  let lineWidth = 0;
  let lineHeight = 0;
  for (const box of boxes) {
    const width = Math.min(box.width, contentWidth);
    const height =
      box.scales && box.width > 0 ? box.height * (width / box.width) : box.height;
    const needed = lineWidth === 0 ? width : lineWidth + gap + width;
    if (needed > contentWidth && lineWidth > 0) {
      stacked += lineHeight + gap;
      lineWidth = width;
      lineHeight = height;
      continue;
    }
    lineWidth = needed;
    lineHeight = Math.max(lineHeight, height);
  }
  return stacked + lineHeight;
}

/** `.attachments` is a wrapping flex row with a 6px gap and 4px/2px margins. */
const ATTACHMENT_GAP = 6;
const ATTACHMENT_MARGINS = 6;
/** `.shot` carries a 1px border on each edge, outside the box Rust fitted. */
const SHOT_BORDER = 2;
/** A non-image attachment: a kind badge beside a name and a size, in a bordered
 *  box with 6px of padding top and bottom. */
const CARD_HEIGHT = 42;
const CARD_WIDTH = 340;

function fileExtra(row: Row): number {
  if (row.kind !== "post" && row.kind !== "continuation") return 0;
  const files = row.post.files ?? [];
  if (files.length === 0) return 0;
  const boxes: WrapBox[] = files.map((file) => {
    if (file.image) {
      return {
        width: file.box_width || 320,
        height: (file.box_height || 180) + SHOT_BORDER,
        scales: true,
      };
    }
    // A player is drawn at the same box an image would be, plus its controls,
    // which the element adds inside its own height only once it has metadata.
    if (file.video) {
      return {
        width: file.box_width || 480,
        height: (file.box_height || 270) + SHOT_BORDER,
        scales: true,
      };
    }
    return { width: CARD_WIDTH, height: CARD_HEIGHT, scales: false };
  });
  return ATTACHMENT_MARGINS + wrappedHeight(boxes, ATTACHMENT_GAP);
}

/** `.card` and `.quoted`: 8px padding top and bottom, 4px/2px margins. */
const PREVIEW_PADDING = 16;
const PREVIEW_MARGINS = 6;
/** How wide a preview card draws its image.
 *
 *  Exported because `Previews.svelte` sizes the box with it: the image Rust
 *  fitted to its 400x220 box is scaled down *again* here, so this number
 *  decides the drawn height, and an estimate that disagreed with the
 *  stylesheet would be wrong by the difference. */
export const PREVIEW_IMAGE_WIDTH = 160;
/** The card's text column, line by line, because the fields are optional: an
 *  11px site name, a 13.5px title, and a description clamped to two lines of
 *  12.5px, with a 2px gap between whichever are present. The card is a flex
 *  *row*, so it is as tall as the taller of this column and the image. */
const PREVIEW_SITE_LINE = 14;
const PREVIEW_TITLE_LINE = 17;
const PREVIEW_BLURB_LINE = 16;
const PREVIEW_BLURB_MAX_LINES = 2;
const PREVIEW_TEXT_GAP = 2;
/** A quoted message: the who/where/when line, the 3px column gap, and a body
 *  clamped to three lines. */
const PERMALINK_HEAD = 19;
const PERMALINK_GAP = 3;
const PERMALINK_LINE = 20;
const PERMALINK_MAX_LINES = 3;

function previewExtra(row: Row, withImages: boolean): number {
  if (row.kind !== "post" && row.kind !== "continuation") return 0;
  let total = 0;
  for (const preview of row.post.previews ?? []) {
    if (preview.kind === "page") {
      const image = preview.image;
      const drawn =
        withImages && image && image.width > 0
          ? image.height * Math.min(1, PREVIEW_IMAGE_WIDTH / image.width)
          : 0;
      let text = 0;
      let gaps = -1;
      if (preview.site_name) {
        text += PREVIEW_SITE_LINE;
        gaps += 1;
      }
      if (preview.title) {
        text += PREVIEW_TITLE_LINE;
        gaps += 1;
      }
      if (preview.description) {
        const blurb = Math.min(
          PREVIEW_BLURB_MAX_LINES,
          Math.max(1, Math.ceil(preview.description.length / charsPerLine())),
        );
        text += blurb * PREVIEW_BLURB_LINE;
        gaps += 1;
      }
      text += Math.max(0, gaps) * PREVIEW_TEXT_GAP;
      total += PREVIEW_PADDING + PREVIEW_MARGINS + Math.max(text, drawn);
      continue;
    }
    const characters = charactersIn(preview.nodes ?? []);
    const lines = Math.min(
      PERMALINK_MAX_LINES,
      Math.max(1, Math.ceil(characters / charsPerLine())),
    );
    total +=
      PREVIEW_PADDING +
      PREVIEW_MARGINS +
      PERMALINK_HEAD +
      PERMALINK_GAP +
      lines * PERMALINK_LINE;
  }
  return total;
}

/** Everything a row's height carries that is known before it is mounted.
 *
 *  Every term here has to match what the renderer actually draws, in both
 *  directions. Over-claiming is not the harmless side: `learn` is handed
 *  `height - knownExtra - textExtra` as the row's base, so an extra that claims
 *  more than the row has makes that base negative and drags the learned average
 *  for the whole kind down. Measured before this was corrected: bases as low as
 *  -868px, and a layout that reserved nothing for 49 unmounted rows.
 */
/** The box an inline markdown image draws in, plus its margins.
 *
 *  A fixed height on purpose: an image in a message body has no dimensions
 *  until it loads, and a row's height has to be known before it mounts. See
 *  `.inline-image` in `Nodes.svelte`; the two numbers have to agree. */
const IMAGE_LINE = 186;

/** Markdown images in a post's body.
 *
 *  Counted from the node tree the same way characters are, because that is the
 *  only place they appear -- an image posted this way is not a file and never
 *  reaches `post.files`. */
function imagesIn(nodes: readonly unknown[]): number {
  let total = 0;
  for (const node of nodes as {
    t?: string;
    children?: unknown[];
    items?: unknown[][];
  }[]) {
    if (node.t === "image") total += 1;
    if (node.children) total += imagesIn(node.children);
    if (node.items) for (const item of node.items) total += imagesIn(item);
  }
  return total;
}

function knownExtra(row: Row): number {
  if (row.kind !== "post" && row.kind !== "continuation") return 0;
  return (
    imagesIn(row.post.nodes ?? []) * IMAGE_LINE +
    reactionExtra(row) +
    fileExtra(row) +
    // The reader can turn preview images off, and then the card draws none --
    // so the height they would have taken must not be reserved either.
    previewExtra(row, previewImages())
  );
}

/** The least a row's *base* height can be, once everything known in advance is
 *  taken out of it.
 *
 *  A floor rather than a guess: `learn` is handed `height - knownExtra -
 *  textExtra`, so any extra that over-estimates makes that difference negative,
 *  and a few negative samples drag the learned average for the whole kind to
 *  zero. Rows the virtualiser has never mounted are then reserved nothing --
 *  measured, a channel put 49 rows at offset 0, drew the mounted window 2679px
 *  above the viewport, and showed the reader a blank stream over a 22000px
 *  scroll space. */
const MINIMUM_BASE = 12;

function estimate(channelId: string, kind: Row["kind"]): number {
  const sample = learned.get(channelId)?.get(kind);
  if (!sample || sample.count < ENOUGH_SAMPLES) return ESTIMATES[kind];
  return Math.max(MINIMUM_BASE, sample.total / sample.count);
}

function learn(channelId: string, kind: Row["kind"], height: number): void {
  let kinds = learned.get(channelId);
  if (!kinds) {
    kinds = new Map();
    learned.set(channelId, kinds);
  }
  const sample = kinds.get(kind) ?? { total: 0, count: 0 };
  sample.total += Math.max(MINIMUM_BASE, height);
  sample.count += 1;
  kinds.set(kind, sample);
}

/** Reports rows that mount with no height, rate-limited. */
const empties = new Map<string, number>();
let lastEmpty = 0;

function collapsed(kind: string, key: string, height: number): void {
  empties.set(kind, (empties.get(kind) ?? 0) + 1);
  const now = performance.now();
  if (now - lastEmpty < 1000) return;
  lastEmpty = now;
  log.debug("row.collapsed", {
    kind,
    height: height.toFixed(1),
    row: key.slice(0, 28),
    byKind: [...empties].map(([name, times]) => `${name}=${times}`).join(" "),
  });
}

/** Reports extras that claim more height than the row has, rate-limited.
 *
 *  Broken down by term, because a negative base only says that *something*
 *  over-claimed. `reactions`, `files` and `previews` are the parts of
 *  `knownExtra`; `text` is the wrapped-line estimate, and `chars` is what that
 *  estimate was computed from. */
const clamped = new Map<string, { times: number; worst: number }>();
let lastClamp = 0;

function overReserved(
  row: Row,
  height: number,
  extras: number,
  text: number,
  base: number,
): void {
  const kind = kindOf(row);
  const seen = clamped.get(kind) ?? { times: 0, worst: 0 };
  seen.times += 1;
  seen.worst = Math.min(seen.worst, base);
  clamped.set(kind, seen);
  const now = performance.now();
  if (now - lastClamp < 1500) return;
  lastClamp = now;
  const post = row.kind === "post" || row.kind === "continuation" ? row.post : undefined;
  log.debug("row.over_reserved", {
    kind,
    base: base.toFixed(1),
    height: height.toFixed(1),
    extras: extras.toFixed(1),
    reactions: reactionExtra(row).toFixed(1),
    files: fileExtra(row).toFixed(1),
    previews: previewExtra(row, previewImages()).toFixed(1),
    text: text.toFixed(1),
    chars: post ? charactersIn(post.nodes ?? []) : -1,
    worst: seen.worst.toFixed(1),
    times: seen.times,
  });
}

function heightsFor(channelId: string): Map<string, number> {
  let heights = measured.get(channelId);
  if (!heights) {
    heights = new Map();
    measured.set(channelId, heights);
  }
  return heights;
}

/** Re-measurements a row is allowed before its height is settled for good.
 *
 *  A row that reports two different heights is not giving new information, it
 *  is oscillating: the height changes the layout, the layout changes which rows
 *  are mounted, and the row comes back with the height it started with. Left
 *  alone the loop is self-sustaining -- measured, a channel scrolled itself
 *  +-300px a dozen times a second with the mouse untouched, which is what a
 *  reader feels as the scrollbar only half following their hand. */
const ALLOWED_REMEASURES = 1;

/** Rows whose height is no longer up for debate. */
const settled = new Map<string, Set<string>>();

function settledIn(channelId: string): Set<string> {
  let keys = settled.get(channelId);
  if (!keys) {
    keys = new Set();
    settled.set(channelId, keys);
  }
  return keys;
}

/** Records a row's real height. Returns true if it changed the layout. */
export function measure(channelId: string, key: string, row: Row, height: number): boolean {
  if (height < 4) {
    // A mounted row with no height renders nothing, and the layout goes on
    // reserving its estimate for ever: the reader gets reserved space with
    // nothing in it. Reported with the kind, because which kind draws nothing
    // is the whole question.
    collapsed(kindOf(row), key, height);
    return false;
  }
  const heights = heightsFor(channelId);
  const known = heights.get(key);
  // Sub-pixel churn from zoom or font loading is not worth a relayout.
  if (known !== undefined && Math.abs(known - height) < 0.5) return false;
  if (settledIn(channelId).has(key)) return false;
  if (known !== undefined) {
    // A row measuring *differently* after it was already measured is a
    // feedback loop, not new information. Logged with the two heights, because
    // the only way to understand such a loop is to know which row and by how
    // much.
    remeasured(key, known, height, kindOf(row));
    if (timesSeen(key) > ALLOWED_REMEASURES) {
      // The tallest it has ever been, then never again. Tallest rather than
      // latest because a row reserved too short leaves a gap that the next
      // measurement closes -- which is another turn of the same loop -- while
      // one reserved too tall is simply a little space below the last line.
      heights.set(key, Math.max(known, height));
      settledIn(channelId).add(key);
      return true;
    }
  }
  // Only the first measurement of a row teaches: re-measuring the same row on
  // every remount would weight whatever the reader happens to look at most.
  if (known === undefined) {
    const extras = knownExtra(row);
    const text = textExtra(row);
    const base = height - extras - text;
    // Attributed, not just flagged: three terms could each account for a
    // negative base, and the numbers say which one did.
    if (base < 0) overReserved(row, height, extras, text, base);
    learn(channelId, row.kind, base);
  }
  heights.set(key, height);
  return true;
}

/** Reports re-measurements, rate-limited: a loop produces them by the hundred
 *  and the log must stay readable. */
const seenAgain = new Map<string, number>();
let lastReport = 0;

function kindOf(row: Row): string {
  return row.kind;
}

function timesSeen(key: string): number {
  return seenAgain.get(key) ?? 0;
}

function remeasured(key: string, from: number, to: number, kind: string): void {
  const times = timesSeen(key) + 1;
  seenAgain.set(key, times);
  const now = performance.now();
  // At most one line a second, naming the worst offender so far.
  if (now - lastReport < 1000) return;
  lastReport = now;
  let worst = key;
  let most = times;
  for (const [candidate, count] of seenAgain) {
    if (count > most) {
      most = count;
      worst = candidate;
    }
  }
  log.debug("row.remeasured", {
    kind,
    from: from.toFixed(1),
    to: to.toFixed(1),
    times,
    worstRow: worst.slice(0, 24),
    worstTimes: most,
  });
}

/** The height the layout has reserved for a row: measured if it ever mounted,
 *  estimated otherwise.
 *
 *  Public because a placeholder has to be exactly this tall. A placeholder any
 *  other height would reintroduce the very mismatch it exists to remove. */
export function heightOf(channelId: string, row: Row): number {
  return (
    heightsFor(channelId).get(rowKey(row)) ??
    estimate(channelId, row.kind) + knownExtra(row) + textExtra(row)
  );
}

export function layoutOf(channelId: string, rows: Row[]): Layout {
  const heights = heightsFor(channelId);
  const offsets: number[] = [];
  let running = 0;
  rows.forEach((row) => {
    offsets.push(running);
    running +=
      heights.get(rowKey(row)) ??
      estimate(channelId, row.kind) + knownExtra(row) + textExtra(row);
  });
  // One past the end, so the last entry is the total: an offset for "after the
  // final row" is what the bottom spacer measures against.
  offsets.push(running);
  return { offsets, total: running };
}

/** The height the layout reserved for a row.
 *
 *  Not always the height the row now measures: a row measured *after* the
 *  layout was computed carries a newer number, and until the next relayout the
 *  two disagree. */
export function reservedIn(layout: Layout, index: number): number {
  return (layout.offsets[index + 1] ?? 0) - (layout.offsets[index] ?? 0);
}

export interface Slice {
  start: number;
  end: number;
  padTop: number;
  padBottom: number;
}

/** The rows to mount for a given scroll position. */
export function sliceOf(
  layout: Layout,
  rowCount: number,
  scrollTop: number,
  viewportHeight: number,
): Slice {
  if (rowCount === 0) return { start: 0, end: 0, padTop: 0, padBottom: 0 };
  const at = (index: number) => layout.offsets[index] ?? 0;
  const top = Math.max(0, scrollTop - OVERSCAN_PX);
  const bottom = scrollTop + viewportHeight + OVERSCAN_PX;
  const start = Math.max(0, upperBound(layout.offsets, rowCount, top) - 1);
  let end = start;
  while (end < rowCount && at(end) < bottom) end += 1;
  return {
    start,
    end,
    padTop: at(start),
    padBottom: Math.max(0, layout.total - at(end)),
  };
}

/** First index whose offset exceeds `target`. Binary, because this runs on
 *  every scroll event and history is unbounded now. */
function upperBound(offsets: number[], rowCount: number, target: number): number {
  let low = 0;
  let high = rowCount;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if ((offsets[middle] ?? 0) <= target) low = middle + 1;
    else high = middle;
  }
  return low;
}

/** The row at a scroll position: the anchor a height correction must not move. */
export function indexAt(layout: Layout, rowCount: number, scrollTop: number): number {
  if (rowCount === 0) return 0;
  return Math.max(0, upperBound(layout.offsets, rowCount, scrollTop) - 1);
}

/** Where a row sits, for scrolling to a row that has never been mounted. */
export function offsetOf(layout: Layout, index: number): number {
  const clamped = Math.max(0, Math.min(index, layout.offsets.length - 1));
  return layout.offsets[clamped] ?? 0;
}

/** The index of the unread divider, or -1. */
export function dividerIndex(rows: Row[]): number {
  return rows.findIndex((row) => row.kind === "unread_divider");
}

/** Where a particular message sits in the plan, or -1.
 *
 *  For jumping to a search result: the row may not be mounted, so this asks the
 *  plan rather than the DOM. */
export function indexOfPost(rows: Row[], postId: string): number {
  return rows.findIndex(
    (row) =>
      (row.kind === "post" || row.kind === "continuation") && row.post.post_id === postId,
  );
}
