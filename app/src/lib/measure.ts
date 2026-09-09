// How many lines a message actually occupies, measured rather than guessed.
//
// The virtualiser needs a row's height before that row exists, and the part it
// cannot know is where the browser breaks the text. Estimating it as
// `characters / (width / 6.95)` is an average glyph width, which is wrong per
// message and, worse, wrong *differently* at two widths -- so opening the
// thread pane moved every unmounted row by an unpredictable amount.
//
// A canvas measures the same font the message is drawn in, so the wrap can be
// computed exactly without laying anything out. The greedy algorithm here is
// the one browsers use for `word-wrap: normal`: fill a line until the next word
// does not fit, then break.
//
// What this does not model: bidirectional text, hyphenation, and the exact
// break opportunities inside a long unbroken token. Those are rare in chat and
// each costs at most a line.

/** The fonts a message is drawn in, sampled from the real DOM once. */
interface Fonts {
  body: string;
  bold: string;
  italic: string;
  code: string;
  /** The width a code block gets before it scrolls rather than wraps. */
  ready: boolean;
}

const fonts: Fonts = {
  body: "14px system-ui",
  bold: "600 14px system-ui",
  italic: "italic 14px system-ui",
  code: "12px ui-monospace, monospace",
  ready: false,
};

let pad: CanvasRenderingContext2D | null | undefined;

function context(): CanvasRenderingContext2D | null {
  if (pad === undefined) {
    // `willReadFrequently` is not set: nothing is read back, only measured.
    pad = document.createElement("canvas").getContext("2d");
  }
  return pad;
}

/** Learns the fonts from a row that is on screen.
 *
 *  Sampled rather than hard-coded: the stylesheet owns them, and a measurement
 *  taken against a different font is a confident wrong answer. Called again
 *  whenever a row mounts costs nothing after the first time. */
export function learnFonts(sample: HTMLElement): void {
  if (fonts.ready) return;
  const style = getComputedStyle(sample);
  const family = style.fontFamily;
  const size = style.fontSize;
  if (!family || !size) return;
  fonts.body = `${size} ${family}`;
  fonts.bold = `600 ${size} ${family}`;
  fonts.italic = `italic ${size} ${family}`;
  const code = sample.querySelector("code");
  if (code) {
    const codeStyle = getComputedStyle(code);
    fonts.code = `${codeStyle.fontSize} ${codeStyle.fontFamily}`;
  }
  fonts.ready = true;
}

/** One run of text in one font. A break is a run with no text. */
interface Run {
  text: string;
  font: string;
  /** Ends the line wherever it falls. */
  breaks: boolean;
  /** A fixed width for something that is not text -- an emoji image, or the
   *  padding around a mention pill. */
  box?: number;
}

type Node = {
  t?: string;
  value?: string;
  username?: string;
  name?: string;
  children?: unknown[];
  items?: unknown[][];
};

/** An emoji is drawn as an image at roughly the line's own height, and a
 *  mention as a padded pill; both are wider than their text. */
const EMOJI_WIDTH = 20;
/** The horizontal padding a mention pill draws around its text. */
const MENTION_PADDING = 8;

function runsOf(nodes: readonly unknown[], font: string, into: Run[]): void {
  for (const node of nodes as Node[]) {
    switch (node.t) {
      case "text":
        into.push({ text: node.value ?? "", font, breaks: false });
        break;
      case "strong":
        runsOf(node.children ?? [], fonts.bold, into);
        break;
      case "emphasis":
      case "strike":
        runsOf(node.children ?? [], fonts.italic, into);
        break;
      case "inline_code":
      case "inline_math":
        into.push({ text: node.value ?? "", font: fonts.code, breaks: false });
        break;
      case "link":
        runsOf(node.children ?? [], font, into);
        break;
      case "user_mention":
        // A pill: its own text, plus the padding drawn around it.
        into.push({
          text: "@" + (node.username ?? ""),
          font,
          breaks: false,
          box: MENTION_PADDING,
        });
        break;
      case "channel_link":
        into.push({
          text: "~" + (node.name ?? ""),
          font,
          breaks: false,
          box: MENTION_PADDING,
        });
        break;
      case "emoji":
        // An image at about the line's height, not the letters of its name.
        into.push({ text: "", font, breaks: false, box: EMOJI_WIDTH });
        break;
      case "soft_break":
      case "hard_break":
      case "rule":
        into.push({ text: "", font, breaks: true });
        break;
      // Every block starts on its own line and ends one.
      case "paragraph":
      case "heading":
      case "blockquote":
        into.push({ text: "", font, breaks: true });
        runsOf(node.children ?? [], node.t === "heading" ? fonts.bold : font, into);
        into.push({ text: "", font, breaks: true });
        break;
      case "list":
        for (const item of node.items ?? []) {
          into.push({ text: "", font, breaks: true });
          runsOf(item, font, into);
        }
        into.push({ text: "", font, breaks: true });
        break;
      default:
        if (node.children) runsOf(node.children, font, into);
        if (node.items) for (const item of node.items) runsOf(item, font, into);
        break;
    }
  }
}

/** Greedy wrap, the way `word-wrap: normal` breaks: a line is filled to the
 *  last word boundary that fits, and the rest starts the next one.
 *
 *  Substrings are measured whole rather than word by word and summed. Summing
 *  discards the kerning between words and accumulates each word's sub-pixel
 *  rounding, and a line only has to be wrong by one glyph for the count to be
 *  out by one -- which costs a whole line of height.
 */
function wrap(runs: Run[], width: number, ctx: CanvasRenderingContext2D): number {
  let lines = 1;
  let used = 0;
  for (const run of runs) {
    if (run.breaks) {
      if (used > 0) lines += 1;
      used = 0;
      continue;
    }
    ctx.font = run.font;
    if (run.text === "") {
      // A box of a known width: one unbreakable thing.
      const box = run.box ?? 0;
      if (box === 0) continue;
      if (used + box > width && used > 0) {
        lines += 1;
        used = 0;
      }
      used += box;
      continue;
    }

    let rest = run.text;
    const extra = run.box ?? 0;
    while (rest.length > 0) {
      const room = width - used;
      const whole = ctx.measureText(rest).width + extra;
      if (whole <= room) {
        used += whole;
        break;
      }
      // The last word boundary that fits in what is left of this line.
      const cut = fits(rest, room - extra, ctx);
      if (cut === 0) {
        if (used > 0) {
          // Nothing fits here; the next line is empty and will take more.
          lines += 1;
          used = 0;
          continue;
        }
        // Not even on a line of its own: it breaks mid-word, which is what
        // `overflow-wrap: anywhere` on a message body allows.
        const forced = Math.max(1, breakAt(rest, width, ctx));
        rest = rest.slice(forced);
        lines += 1;
        used = 0;
        continue;
      }
      used += ctx.measureText(rest.slice(0, cut)).width;
      rest = rest.slice(cut).replace(/^\s+/, "");
      if (rest.length > 0) {
        lines += 1;
        used = 0;
      }
    }
  }
  return lines;
}

/** The length of the longest prefix ending at a word boundary that fits in
 *  `room`, or 0 when none does. */
function fits(text: string, room: number, ctx: CanvasRenderingContext2D): number {
  if (room <= 0) return 0;
  let best = 0;
  // Word boundaries only: a browser breaks between words, not inside one.
  for (let at = text.indexOf(" "); at !== -1; at = text.indexOf(" ", at + 1)) {
    if (ctx.measureText(text.slice(0, at)).width > room) break;
    best = at;
  }
  return best;
}

/** How many characters fit in `room`, for a token with no break opportunity. */
function breakAt(text: string, room: number, ctx: CanvasRenderingContext2D): number {
  let low = 1;
  let high = text.length;
  while (low < high) {
    const middle = Math.floor((low + high + 1) / 2);
    if (ctx.measureText(text.slice(0, middle)).width <= room) {
      low = middle;
    } else {
      high = middle - 1;
    }
  }
  return low;
}

/** The number of lines this content wraps to at `width`, or null when nothing
 *  can be measured yet -- before the first row has mounted, or in a context
 *  with no canvas. The caller keeps its estimate for that case. */
export function linesOf(nodes: readonly unknown[], width: number): number | null {
  if (width < 40) return null;
  const ctx = context();
  if (!ctx) return null;
  const runs: Run[] = [];
  runsOf(nodes, fonts.body, runs);
  if (runs.length === 0) return null;
  return Math.max(1, wrap(runs, width, ctx));
}

/** Code blocks scroll rather than wrap, so their lines are the ones written. */
export function codeLines(value: string): number {
  return value.length === 0 ? 0 : value.split("\n").length;
}
