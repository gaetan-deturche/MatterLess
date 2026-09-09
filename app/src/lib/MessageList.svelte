<script lang="ts">
  // One branch per row kind, and no decisions of its own: grouping, separators,
  // markdown and thread folding all happened in Rust.
  import Attachments from "./Attachments.svelte";
  import Previews from "./Previews.svelte";
  import EmojiPicker from "./EmojiPicker.svelte";
  import ForwardTo from "./ForwardTo.svelte";
  import Avatar from "./Avatar.svelte";
  import Nodes from "./Nodes.svelte";
  import * as api from "./api";
  import * as store from "./store.svelte";
  import * as media from "./media";
  import * as log from "./log";
  import { tick } from "svelte";
  import type { Row } from "./api";
  import * as virtual from "./virtual";

  // `scrollTop` and `viewportHeight` come from the scroller, which App owns
  // along with every decision about where the scroll sits. This component only
  // decides which rows that position needs mounted.
  let {
    rows,
    channelId,
    scrollTop = 0,
    viewportHeight = 0,
    liveScrollTop,
    onlayout,
    virtualise = true,
    compact = false,
    frozen = false,
  }: {
    rows: Row[] | undefined;
    channelId: string;
    /** A thread pane is tens of rows: mounting them all costs less than the
     *  height bookkeeping windowing needs. */
    virtualise?: boolean;
    /** Drawn in a narrow pane, so attachments get a smaller ceiling. */
    compact?: boolean;
    /** The reader is dragging the scrollbar: record measurements but do not
     *  act on them yet.
     *
     *  Applying one changes the total height, which resizes and moves the
     *  scrollbar thumb under the cursor -- so a drag through unmeasured history
     *  fights itself. The heights are still recorded; the layout catches up the
     *  moment the drag ends. */
    frozen?: boolean;
    scrollTop?: number;
    viewportHeight?: number;
    /** The scroller's position right now, for anchoring.
     *
     *  The `scrollTop` prop arrives via a scroll event and can be a frame
     *  behind, which during a fast wheel scroll is enough to anchor on the
     *  wrong row -- and anchoring on the wrong row moves the view rather than
     *  holding it. */
    liveScrollTop?: () => number;
    /** The row to keep still, once the corrected layout has been laid out.
     *
     *  Measured at the *bottom* edge of the viewport, not the top. A message
     *  that re-wraps taller -- which is what opening the thread pane does to
     *  every one of them at once -- grows downwards, so holding the top row
     *  still pushes everything the reader was looking at off the bottom. Held
     *  at the bottom instead, the growth goes into the history above, where
     *  nothing was being read. */
    onlayout?: (anchor: { key: string; within: number }) => void;
  } = $props();

  /** Bumped by a measurement, to recompute a layout the estimates got wrong. */
  let revision = $state(0);

  const layout = $derived.by(() => {
    revision;
    return virtual.layoutOf(channelId, rows ?? []);
  });
  const slice = $derived(
    virtualise
      ? virtual.sliceOf(layout, rows?.length ?? 0, scrollTop, viewportHeight)
      : { start: 0, end: rows?.length ?? 0, padTop: 0, padBottom: 0 },
  );

  /** Bumped by every measurement, acted on or not: the drift below has to be
   *  recomputed even while the layout itself is held still. */
  let measurements = $state(0);

  /** `.stream` is a flex column, and its gap is real height that belongs to no
   *  row's border box. */
  const ROW_GAP = 2;

  /** How much taller the mounted rows draw than the layout reserved for them.
   *
   *  The scroller is padTop + the real rows + padBottom, but the scrollbar
   *  describes `layout.total`. A row measured since the layout was computed
   *  draws at its new height while the layout still reserves the old one, and
   *  because the mounted window slides as you scroll, that disagreement is a
   *  different number every frame. Measured during a scrollbar drag before this
   *  existed: the scroll height moved 700-1300px per frame on a 36000px
   *  channel, which is the thumb being resized and repositioned out from under
   *  the cursor.
   *
   *  Taken out of the bottom spacer, so the total stays exactly `layout.total`
   *  and the error lands below the viewport where nothing can see it. */
  const drift = $derived.by(() => {
    measurements;
    if (!virtualise) return 0;
    const list = rows ?? [];
    let sum = 0;
    for (let index = slice.start; index < slice.end; index += 1) {
      const row = list[index];
      if (!row) continue;
      sum += virtual.heightOf(channelId, row) - virtual.reservedIn(layout, index);
    }
    // One gap between each mounted row, and one to each spacer.
    return sum + ROW_GAP * (slice.end - slice.start + 1);
  });

  /** Tells the layout how wide the message column actually is.
   *
   *  Every wrap estimate in `virtual` divides by this, and it used to be a
   *  hard-coded 660px -- right for a narrow window and a third short at 1280px,
   *  which made `textExtra` claim five lines where two were drawn. The content
   *  box comes from the observer, so it already excludes the stream's padding.
   */
  function columnWidth(element: HTMLElement) {
    // Only the virtualised list may set this. `contentWidth` is one
    // module-level value, and the thread pane mounts a list of its own: its
    // ~300px column was overwriting the channel's ~700px one, so the channel's
    // rows were estimated at the wrong width and every alternation cleared
    // every learned base. Measured as a 2875px anchor correction, which the
    // reader saw as the stream snapping back after they scrolled to the end.
    // A thread mounts all its rows, so it needs no estimate to begin with.
    if (!virtualise) return;
    const observer = new ResizeObserver((entries) => {
      const box = entries[0]?.contentBoxSize?.[0];
      if (box) virtual.setContentWidth(box.inlineSize);
    });
    observer.observe(element);
    return {
      destroy() {
        observer.disconnect();
      },
    };
  }

  /** Reports a mounted row's real height.
   *
   *  An action rather than an effect: it has to run per row, and it has to see
   *  the element. Correcting an estimate changes every offset below it, so the
   *  scroller is told to re-read the layout -- otherwise a wrong estimate would
   *  quietly move the reader's position as they scroll into fresh history.
   */
  /** One relayout per frame, however many rows report in it.
   *
   *  Every row measured on mount, and each correction changes every offset below
   *  it -- so reacting per row meant ~30 recomputes and ~30 re-renders in the
   *  frame a channel opens.
   */
  let pendingLayout = false;
  /** A measurement arrived while the layout was frozen. */
  let heldBack = $state(false);

  $effect(() => {
    // The drag ended and something was measured meanwhile: one relayout now,
    // rather than a hundred during the gesture.
    if (!frozen && heldBack) {
      heldBack = false;
      invalidateLayout();
    }
  });
  function invalidateLayout() {
    if (pendingLayout) return;
    pendingLayout = true;
    requestAnimationFrame(async () => {
      pendingLayout = false;

      // Which row the reader is looking at, and how far into it they are.
      // Correcting a row that sits *above* the viewport shifts everything below
      // it, so without this the text under the reader's eyes jumps every time
      // they scroll into history that has never been measured.
      //
      // Reported as a row rather than a pixel shift, and left for the scroller
      // to act on *after* the new layout is on screen: an earlier version added
      // the shift here, in the same frame, where the spacer had not grown yet --
      // so the browser clamped the write against the old scroll height and the
      // list appeared to rewind.
      const list = rows ?? [];
      // The bottom edge, which is the part of the list being read.
      const at = (liveScrollTop?.() ?? scrollTop) + viewportHeight;
      const index = virtual.indexAt(layout, list.length, at);
      const row = list[index];
      const within = at - virtual.offsetOf(layout, index);

      revision += 1;
      // Flushed here, not left to the next frame: the corrected spacers have to
      // be in the DOM before the scroll is restored, and a second
      // `requestAnimationFrame` bought that at the cost of *painting* the
      // displaced frame first -- which is what the reader sees as the list
      // rolling back before snapping into place. `tick` is a microtask, so the
      // restore still happens before this frame is painted.
      await tick();
      if (row) onlayout?.({ key: virtual.rowKey(row), within });
    });
  }

  function measured(element: HTMLElement, held: { key: string; row: Row }) {
    // Mutable, and read inside the observer: an update hands over a new row,
    // and measuring the old one against the new element would teach the wrong
    // kind.
    let current = held;

    // The size comes from the observer's own entry, never from
    // `getBoundingClientRect`.
    //
    // That call forces a synchronous layout, and this runs once per mounted
    // row -- so a scroll that mounts thirty rows forced thirty reflows in one
    // frame. Measured: 33-37ms per frame while dragging the scrollbar at 152
    // rows, which in Chromium *is* the thumb lagging, because a scrollbar drag
    // is driven on the main thread. The observer has already computed these
    // boxes; reading them costs nothing.
    //
    // `borderBoxSize` rather than `contentRect`: the latter excludes padding,
    // and a row's padding is part of the height the layout reserves.
    const observer = new ResizeObserver((entries) => {
      // Nothing reads the layout when every row is mounted, so measuring it
      // would be work for its own sake.
      if (!virtualise) return;
      for (const entry of entries) {
        const box = entry.borderBoxSize?.[0];
        const height = box ? box.blockSize : entry.contentRect.height;
        if (virtual.measure(channelId, current.key, current.row, height)) {
          measurements += 1;
          if (frozen) {
            heldBack = true;
          } else {
            invalidateLayout();
          }
        }
      }
    });
    // No manual first measurement: a ResizeObserver delivers one for every
    // element it starts observing, and that delivery costs no reflow.
    observer.observe(element);
    return {
      update(next: { key: string; row: Row }) {
        current = next;
      },
      destroy() {
        observer.disconnect();
      },
    };
  }

  /** The reactions offered without typing a name.
   *
   *  Deliberately short: a full picker needs the custom-emoji images, which are
   *  the media handler's job. These are the ones a keyboard cannot produce
   *  faster than a click. */
  const QUICK: [name: string, face: string][] = [
    ["+1", "\u{1F44D}"],
    ["-1", "\u{1F44E}"],
    ["tada", "\u{1F389}"],
    ["eyes", "\u{1F440}"],
    ["heart", "\u{2764}\u{FE0F}"],
    ["joy", "\u{1F602}"],
    ["thinking_face", "\u{1F914}"],
  ];

  /** Who this reader is, for the actions that are per-reader. */
  const meId = $derived(store.meta()?.meId ?? "");
  /** Each message's reaction picker, so choosing one can close it. */
  const pickerOpen: Record<string, boolean> = $state({});
  /** Which of them open upward, decided when they open. */
  const pickerUp: Record<string, boolean> = $state({});
  /** The message being forwarded, if any. */
  let forwarding: string | null = $state(null);
  /** The message whose reminder options are showing. */
  let remindFor: string | null = $state(null);

  /** When to be reminded, as seconds from now.
   *
   *  Fixed offsets except the last: "tomorrow" is nine in the morning local
   *  time, which is a question about this machine's clock rather than an
   *  interval, so it is computed when it is clicked. */
  const REMINDERS: [label: string, seconds: () => number][] = [
    ["30 mins", () => 30 * 60],
    ["1 hour", () => 60 * 60],
    ["2 hours", () => 2 * 60 * 60],
    [
      "Tomorrow",
      () => {
        const morning = new Date();
        morning.setDate(morning.getDate() + 1);
        morning.setHours(9, 0, 0, 0);
        return Math.max(60, Math.round((morning.getTime() - Date.now()) / 1000));
      },
    ],
  ];

  /** The message whose full emoji list is open, and where it was asked for. */
  let browsing: { postId: string; at: { x: number; y: number } } | null = $state(null);

  /** Opens the quick-reaction popup on whichever side has room for it.
   *
   *  The scroller clips in both directions, so a fixed side is wrong at one end
   *  or the other: downward clipped the last message in the channel, and the
   *  upward it replaced clipped the first one on screen. */
  function placePicker(postId: string, event: Event) {
    const details = event.currentTarget as HTMLDetailsElement;
    if (!details.open) return;
    const summary = details.querySelector("summary");
    const choices = details.querySelector<HTMLElement>(".choices");
    const clip = details.closest(".scroll")?.getBoundingClientRect();
    if (!summary || !choices || !clip) return;
    const anchor = summary.getBoundingClientRect();
    // Its own height, since it is already rendered -- with a floor, because a
    // popup measured before its font lands would report almost nothing.
    const needed = Math.max(34, choices.offsetHeight) + 6;
    const below = clip.bottom - anchor.bottom;
    const above = anchor.top - clip.top;
    // Upward only when down genuinely does not fit *and* up fits better.
    pickerUp[postId] = below < needed && above > below;
  }

  /** Which message's actions menu is open, by post id. */
  let menuFor: string | null = $state(null);
  /** The message being edited, with the raw markdown as typed. */
  let editing: { postId: string; text: string; saving: boolean } | null = $state(null);
  /** The message awaiting a delete confirmation. Deleting is irreversible and
   *  a menu click is one slip away from the item above it. */
  let confirmDelete: string | null = $state(null);

  async function startEditing(post: api.PostRow) {
    menuFor = null;
    try {
      // The raw markdown, not the rendered nodes: what was typed is what should
      // come back into the box.
      const text = await api.postText(post.post_id);
      editing = { postId: post.post_id, text, saving: false };
    } catch (thrown) {
      log.failure("message.edit.load.failed", thrown, { post: post.post_id });
      announce(post.post_id, `Could not open that message for editing.`);
    }
  }

  async function saveEdit() {
    if (!editing || editing.saving) return;
    const { postId, text } = editing;
    editing = { ...editing, saving: true };
    try {
      await api.editPost(postId, text);
      editing = null;
      log.info("message.edited", { post: postId, chars: text.length });
    } catch (thrown) {
      editing = { postId, text, saving: false };
      log.failure("message.edit.failed", thrown, { post: postId });
      announce(postId, `Edit failed: ${String(thrown)}`);
    }
  }

  function onEditKey(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      editing = null;
    } else if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      void saveEdit();
    }
  }
  /** What the last action said, shown under the message it acted on. */
  let outcome: { postId: string; text: string } | null = $state(null);

  /** The thread a message belongs to: its own, if it is a root. */
  const threadRootOf = (post: api.PostRow) => post.root_id || post.post_id;

  function announce(postId: string, text: string) {
    outcome = { postId, text };
    // Long enough to read, short enough not to become furniture.
    setTimeout(() => {
      if (outcome?.postId === postId) outcome = null;
    }, 4000);
  }

  /** Runs one action and reports it in place. The menu closes either way: a
   *  menu that stays open over a failed action reads as if nothing happened. */
  async function act(post: api.PostRow, label: string, run: () => Promise<string | void>) {
    menuFor = null;
    try {
      const said = await run();
      log.info("message.action", { action: label, post: post.post_id });
      if (said) announce(post.post_id, said);
    } catch (thrown) {
      log.failure("message.action.failed", thrown, { action: label, post: post.post_id });
      announce(post.post_id, `${label} failed: ${String(thrown)}`);
    }
  }

  /** The clipboard, with the failure surfaced rather than swallowed: in a
   *  webview it can be refused, and silently doing nothing is worse. */
  async function copy(text: string): Promise<string> {
    await navigator.clipboard.writeText(text);
    return "Copied.";
  }

  /** A custom emoji's id, if this name is one. */
  const customEmoji = (name: string) => store.emojiIds()[name];

  /** Everyone who reacted, wrapped: "amy, bob, cass and dee reacted with
   *  :tada:".
   *
   *  Wrapped because a native tooltip is a single line however long it gets,
   *  and the busy posts here carry 48 reactions of one emoji -- which would be
   *  one unreadable line across the whole screen. The "and N others" tail is
   *  kept as a fallback: the plan now names every reactor, so it only appears
   *  if the two ever disagree. */
  const NAMES_PER_LINE = 6;

  function reactedBy(reaction: api.ReactionSummary): string {
    const names = [...reaction.names];
    const unnamed = reaction.count - names.length;
    if (unnamed > 0) names.push(`${unnamed} ${unnamed === 1 ? "other" : "others"}`);
    if (names.length === 0) return `:${reaction.emoji}:`;

    const last = names.pop() as string;
    const lines: string[] = [];
    for (let at = 0; at < names.length; at += NAMES_PER_LINE) {
      lines.push(names.slice(at, at + NAMES_PER_LINE).join(", "));
    }
    const people = lines.length > 0 ? `${lines.join(",\n")} and ${last}` : last;
    return `${people} reacted with :${reaction.emoji}:`;
  }

  async function react(postId: string, name: string) {
    try {
      await api.toggleReaction(postId, name);
    } catch (thrown) {
      log.failure("reaction.failed", thrown, { post: postId, emoji: name });
    }
  }

  const time = (ms: number) =>
    new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const thisYear = new Date().getFullYear();
  /** The year appears once the date is not in this one: scrolling back far
   *  enough that "Tuesday 4 September" is ambiguous is exactly when it matters,
   *  and repeating it on every separator would be noise the rest of the time. */
  const day = (epochDay: number) => {
    const date = new Date(epochDay * 86_400_000);
    return date.toLocaleDateString([], {
      weekday: "long",
      day: "numeric",
      month: "long",
      ...(date.getFullYear() === thisYear ? {} : { year: "numeric" }),
    });
  };
</script>

<div class="stream" use:columnWidth>
  {#if rows === undefined}
    <!-- The one legitimate second path: no entry for this key yet. -->
    <p class="placeholder">loading…</p>
  {:else if rows.length === 0}
    <p class="placeholder">Nothing here yet.</p>
  {:else}
    <!-- Unmounted history, as height. The scrollbar has to describe the whole
         channel even though only a screenful exists in the DOM. -->
    <div class="spacer" style:height="{slice.padTop}px"></div>
    {#each rows.slice(slice.start, slice.end) as row (virtual.rowKey(row))}
      {@const key = virtual.rowKey(row)}
      {#if row.kind === "date_separator"}
        <div class="separator" use:measured={{ key, row }}><span>{day(row.epoch_day)}</span></div>
      {:else if row.kind === "unread_divider"}
        <!-- Tagged so the shell can open the channel at this line rather than
             at the bottom: with many unread messages the divider is well above
             the viewport, which reads as it not being there at all. -->
        <div class="unread" data-unread-divider use:measured={{ key, row }}><span>New messages</span></div>
      {:else if row.kind === "post" || row.kind === "continuation"}
        <article
          use:measured={{ key, row }}
          class="post"
          class:continuation={row.kind === "continuation"}
          class:pending={row.post.pending && !row.post.failed}
          class:failed={row.post.failed}
        >
          {#if row.kind === "post"}
            <!-- Only the first post of a run carries a face; a continuation is
                 the same person still talking. -->
            <div class="face">
              <!-- The face and the name both open the profile: they are the two
                   things a reader points at when they mean "who is this".
                   Presence on the author too -- whether they are around decides
                   whether you reply now or later. -->
              <button
                type="button"
                class="who-button"
                title="Show profile"
                onclick={(event) =>
                  store.openProfile(row.post.author_id, {
                    x: event.clientX,
                    y: event.clientY,
                  })}
              >
                <Avatar
                  userId={row.post.author_id}
                  name={row.post.author_name}
                  version={row.post.avatar_at}
                  presence
                />
              </button>
            </div>
            <header>
              <button
                type="button"
                class="author who-button"
                title="Show profile"
                onclick={(event) =>
                  store.openProfile(row.post.author_id, {
                    x: event.clientX,
                    y: event.clientY,
                  })}>{row.post.author_name}</button
              >
              {#if row.post.bot}
                <!-- A webhook posts under its own name, so without this the
                     name reads as a colleague's. -->
                <span class="bot">BOT</span>
              {/if}
              <time>{time(row.post.create_at)}</time>
              {#if row.post.edited}<span class="edited">edited</span>{/if}
            </header>
          {/if}
          {#if editing?.postId === row.post.post_id}
            <!-- In place, not in the composer: an edit is a change to *this*
                 message, and moving the text elsewhere loses where it belongs.
                 Enter saves, Shift+Enter is a newline, Escape abandons. -->
            <div class="editing">
              <!-- svelte-ignore a11y_autofocus -->
              <textarea
                bind:value={editing.text}
                onkeydown={onEditKey}
                disabled={editing.saving}
                autofocus
                rows="2"
              ></textarea>
              <span class="hint">
                <button type="button" onclick={saveEdit} disabled={editing.saving}>
                  {editing.saving ? "Saving…" : "Save"}
                </button>
                <button type="button" onclick={() => (editing = null)}>Cancel</button>
                Enter saves · Escape cancels
              </span>
            </div>
          {:else}
            <div class="body">
              <Nodes nodes={row.post.nodes} />
              {#if row.post.body_is_attachment_only && row.post.attachments.length === 0}
                <span class="placeholder">(no content)</span>
              {/if}
            </div>
          {/if}
          {#each row.post.attachments as attachment}
            <div class="attachment" style:border-left-color={attachment.color ?? "var(--rule)"}>
              {#if attachment.pretext.length}<div class="pretext"><Nodes nodes={attachment.pretext} /></div>{/if}
              {#if attachment.title}
                <div class="attachment-title">
                  {#if attachment.title_link}
                    <a href={attachment.title_link} target="_blank" rel="noreferrer noopener">{attachment.title}</a>
                  {:else}{attachment.title}{/if}
                </div>
              {/if}
              <Nodes nodes={attachment.text} />
              {#if attachment.fields.length}
                <dl class="fields">
                  {#each attachment.fields as field}
                    <dt>{field.title}</dt>
                    <dd><Nodes nodes={field.value} /></dd>
                  {/each}
                </dl>
              {/if}
            </div>
          {/each}
          {#if row.post.previews.length}
            <Previews previews={row.post.previews} />
          {/if}
          {#if row.post.files.length}
            <Attachments files={row.post.files} {compact} />
          {/if}
          {#if row.post.failed}
            <p class="send-failed">
              Not sent.
              <button type="button" onclick={() => api.retryPost(row.post.post_id)}>Retry</button>
              <button type="button" onclick={() => api.discardPost(row.post.post_id)}>Discard</button>
            </p>
          {:else if row.post.pending}
            <span class="sending">sending…</span>
          {/if}
          {#if row.post.reactions.length}
            <div class="reactions">
              {#each row.post.reactions as reaction (reaction.emoji)}
                {@const custom = customEmoji(reaction.emoji)}
                <button
                  type="button"
                  class="pill"
                  class:mine={reaction.mine}
                  title={reactedBy(reaction)}
                  onclick={() => react(row.post.post_id, reaction.emoji)}
                >
                  {#if custom}
                    <img class="pill-emoji" src={media.emoji(custom)} alt={reaction.emoji} />
                  {:else if reaction.unicode}
                    {reaction.unicode}
                  {:else}
                    :{reaction.emoji}:
                  {/if}
                  <span class="tally">{reaction.count}</span>
                </button>
              {/each}
            </div>
          {/if}
          {#if outcome?.postId === row.post.post_id}
            <p class="outcome">{outcome.text}</p>
          {/if}
          {#if !row.post.pending && !row.post.failed}
          <!-- The message's actions, in one place whether or not it has
               reactions: the reaction button used to move into the pill row as
               soon as somebody reacted, so its position depended on the
               message's history. Floating, so it costs no height at rest. -->
          <div class="tools">
            <details
              class="picker"
              bind:open={pickerOpen[row.post.post_id]}
              ontoggle={(event) => placePicker(row.post.post_id, event)}
            >
              <summary title="Add a reaction" aria-label="Add a reaction">☺</summary>
              <div class="choices" class:up={pickerUp[row.post.post_id]}>
                {#each QUICK as [name, face] (name)}
                  <button
                    type="button"
                    title={`:${name}:`}
                    onclick={() => {
                      pickerOpen[row.post.post_id] = false;
                      void react(row.post.post_id, name);
                    }}>{face}</button
                  >
                {/each}
                <!-- The quick row is the ones a keyboard cannot beat; anything
                     else is a search away rather than absent. -->
                <button
                  type="button"
                  class="more"
                  title="More reactions"
                  aria-label="More reactions"
                  onclick={(event) => {
                    pickerOpen[row.post.post_id] = false;
                    const box = (event.currentTarget as HTMLElement).getBoundingClientRect();
                    browsing = {
                      postId: row.post.post_id,
                      at: { x: box.left + box.width / 2, y: box.bottom },
                    };
                  }}>⋯</button
                >
              </div>
            </details>
            <button
              type="button"
              class="tool"
              title="Reply in thread"
              aria-label="Reply in thread"
              onclick={() => store.setActiveThread(threadRootOf(row.post))}>↩</button
            >
            <button
              type="button"
              class="tool"
              title="More actions"
              aria-label="More actions"
              aria-expanded={menuFor === row.post.post_id}
              onclick={() =>
                (menuFor = menuFor === row.post.post_id ? null : row.post.post_id)}>⋯</button
            >

            {#if menuFor === row.post.post_id}
              <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
              <ul
                class="menu"
                role="menu"
                tabindex="-1"
                onmouseleave={() => (menuFor = null)}
              >
                <li>
                  <button
                    type="button"
                    role="menuitem"
                    onclick={() =>
                      act(row.post, "follow", async () => {
                        const following = !row.post.following;
                        await api.setThreadFollowing(threadRootOf(row.post), following);
                        return following ? "Following this thread." : "Not following.";
                      })}
                  >
                    {row.post.following ? "Unfollow thread" : "Follow thread"}
                  </button>
                </li>
                <li>
                  <button
                    type="button"
                    role="menuitem"
                    onclick={() =>
                      act(row.post, "unread", async () => {
                        await api.markPostUnread(meId, row.post.post_id);
                        return "Unread from here.";
                      })}>Mark as Unread</button
                  >
                </li>
                <li>
                  <button
                    type="button"
                    role="menuitem"
                    onclick={() =>
                      act(row.post, "save", async () => {
                        const saved = !row.post.saved;
                        await api.setPostSaved(meId, row.post.post_id, saved);
                        return saved ? "Saved." : "Removed from saved.";
                      })}
                  >
                    {row.post.saved ? "Remove from Saved" : "Save Message"}
                  </button>
                </li>
                <li>
                  <button
                    type="button"
                    role="menuitem"
                    onclick={() =>
                      act(row.post, "pin", async () => {
                        const pinned = !row.post.pinned;
                        await api.setPostPinned(row.post.post_id, pinned);
                        return pinned ? "Pinned to the channel." : "Unpinned.";
                      })}
                  >
                    {row.post.pinned ? "Unpin from Channel" : "Pin to Channel"}
                  </button>
                </li>
                {#if row.post.author_id === meId}
                  <li class="rule"></li>
                  <li>
                    <button
                      type="button"
                      role="menuitem"
                      onclick={() => void startEditing(row.post)}>Edit</button
                    >
                  </li>
                  <li>
                    {#if confirmDelete === row.post.post_id}
                      <!-- Asked, not assumed: a delete cannot be undone and the
                           menu item above it is one slip away. -->
                      <span class="confirm">
                        Delete?
                        <button
                          type="button"
                          class="danger"
                          onclick={() =>
                            act(row.post, "delete", async () => {
                              confirmDelete = null;
                              await api.deletePostNow(row.post.post_id);
                              return "Deleted.";
                            })}>Delete</button
                        >
                        <button type="button" onclick={() => (confirmDelete = null)}>Keep</button>
                      </span>
                    {:else}
                      <button
                        type="button"
                        role="menuitem"
                        onclick={() => (confirmDelete = row.post.post_id)}>Delete…</button
                      >
                    {/if}
                  </li>
                {/if}
                <li>
                  <button
                    type="button"
                    role="menuitem"
                    onclick={() => {
                      menuFor = null;
                      forwarding = row.post.post_id;
                    }}>Forward…</button
                  >
                </li>
                <li>
                  {#if remindFor === row.post.post_id}
                    <!-- The choices in place of the item, the way the delete
                         confirmation works: a submenu that opens sideways has
                         nowhere to go at the edge of the window. -->
                    <span class="confirm">
                      Remind me in
                      {#each REMINDERS as [label, seconds] (label)}
                        <button
                          type="button"
                          onclick={() =>
                            act(row.post, "remind", async () => {
                              remindFor = null;
                              const when = Math.round(Date.now() / 1000) + seconds();
                              await api.setReminder(row.post.post_id, when);
                              return `Reminder set for ${new Date(when * 1000).toLocaleString([], {
                                hour: "2-digit",
                                minute: "2-digit",
                                day: "numeric",
                                month: "short",
                              })}.`;
                            })}>{label}</button
                        >
                      {/each}
                    </span>
                  {:else}
                    <button
                      type="button"
                      role="menuitem"
                      onclick={() => (remindFor = row.post.post_id)}>Remind me…</button
                    >
                  {/if}
                </li>
                <li class="rule"></li>
                <li>
                  <button
                    type="button"
                    role="menuitem"
                    onclick={() =>
                      act(row.post, "copy.text", async () =>
                        copy(await api.postText(row.post.post_id)),
                      )}>Copy Text</button
                  >
                </li>
                <li>
                  <button
                    type="button"
                    role="menuitem"
                    onclick={() =>
                      act(row.post, "copy.link", async () =>
                        copy(await api.postPermalink(row.post.post_id)),
                      )}>Copy Link</button
                  >
                </li>
              </ul>
            {/if}
          </div>
          {/if}
        </article>
      {:else if row.kind === "thread_footer"}
        <button
          class="footer"
          class:unread={row.unread_replies > 0}
          type="button"
          onclick={() => store.setActiveThread(row.root_id)}
          use:measured={{ key, row }}
        >
          {#if row.participants.length}
            <!-- Who is in it, before how much of it there is: a face is quicker
                 to recognise than a count is to read. -->
            <span class="faces">
              {#each row.participants.slice(0, 3) as person (person.user_id)}
                <span class="face" title={person.name}>
                  <Avatar
                    userId={person.user_id}
                    name={person.name}
                    version={person.avatar_at}
                    size={20}
                  />
                </span>
              {/each}
              {#if row.participants.length > 3}
                <span class="face more">+{row.participants.length - 3}</span>
              {/if}
            </span>
          {/if}
          {row.reply_count} {row.reply_count === 1 ? "reply" : "replies"}
          {#if row.unread_replies > 0}
            <!-- The whole point of this row: with collapsed threads a reply is
                 never a row, so without this a channel can show unread with
                 nothing on screen to explain it. -->
            <span class="new">{row.unread_replies} new</span>
          {/if}
          {#if row.unread_mentions > 0}
            <span class="count mention">{row.unread_mentions}</span>
          {/if}
        </button>
      {:else if row.kind === "system"}
        <!-- The sentence is built in Rust, which can see `props`: this used to
             print the raw type, so a join read "join channel". -->
        <p class="system" use:measured={{ key, row }}>{row.text}</p>
      {:else if row.kind === "deleted_root"}
        <p class="system" use:measured={{ key, row }}>Message deleted — its replies remain.</p>
      {/if}
    {/each}
    <div class="spacer" style:height="{Math.max(0, slice.padBottom - drift)}px"></div>
  {/if}
</div>

{#if forwarding}
  <!-- Outside the list, like the emoji picker: the dialog is `fixed`, and a row
       that scrolls out of the window would otherwise take it with it. -->
  <ForwardTo
    postId={forwarding}
    onclose={(said) => {
      const target = forwarding;
      forwarding = null;
      if (said && target) announce(target, said);
    }}
  />
{/if}

{#if browsing}
  <!-- Outside the list: the panel is `fixed`, and a row that scrolls out of the
       window would otherwise take it with it. -->
  <EmojiPicker
    at={browsing.at}
    onclose={() => (browsing = null)}
    onpick={(name) => {
      const target = browsing?.postId;
      browsing = null;
      if (target) void react(target, name);
    }}
  />
{/if}

<style>
  .stream {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 12px 16px 4px;
    /* A flex column will happily grow to its widest child, which for chat is a
       pasted URL or a log line. */
    min-width: 0;
  }
  .stream :global(pre),
  .stream :global(code) {
    /* Code keeps its own scrollbar rather than stretching the column: the plan
       is explicit that wide content scrolls inside its own container. */
    max-width: 100%;
    overflow-x: auto;
  }
  .post { min-width: 0; overflow-wrap: anywhere; }
  /* Holds the space of rows that are not mounted. `flex: none` so the flex
     container cannot compress it, which would shorten the scrollbar. */
  .spacer { flex: none; }
  .placeholder { color: var(--ink-faint); font-style: italic; margin: 8px 0; }
  .separator, .unread {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 14px 0 6px;
    font-size: 11.5px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--ink-faint);
  }
  .separator::before, .separator::after,
  .unread::before, .unread::after {
    content: "";
    flex: 1;
    height: 1px;
    background: var(--rule);
  }
  .unread { color: var(--flag); }
  .unread::before, .unread::after { background: var(--flag); }
  .post {
    padding: 3px 0;
    /* The corner the bare reaction button floats into. */
    position: relative;
    /* A column for the face, so a continuation's text lines up under the run
       it belongs to. */
    display: grid;
    grid-template-columns: 28px minmax(0, 1fr);
    grid-column-gap: 8px;
    align-items: start;
    /* Deliberately no `content-visibility: auto` here any more. It skipped
       rendering offscreen rows, which sounds like exactly what this list wants
       -- but a skipped element reports its `contain-intrinsic-size` placeholder
       instead of its real height, so measuring the overscan rows would have
       filled the height cache with a made-up 68px, and ResizeObserver never
       fires for them at all. Rows are windowed in JS now, so nothing offscreen
       is mounted to skip. */
  }
  .post.continuation { padding-top: 0; }
  .face { grid-row: 1 / span 2; }
  /* Everything but the face lives in the second column. */
  .post > :global(*:not(.face)) { grid-column: 2; }
  header { display: flex; align-items: baseline; gap: 8px; }
  .author { font-weight: 600; font-size: 13.5px; }
  /* Quiet by design: it labels the name rather than competing with it. */
  .bot {
    font-family: var(--mono);
    font-size: 9.5px;
    letter-spacing: 0.04em;
    padding: 0 4px;
    border-radius: 3px;
    background: var(--surface-2);
    color: var(--ink-faint);
  }
  time, .edited { font-size: 11px; color: var(--ink-faint); }
  .body { font-size: 14px; line-height: 1.5; word-break: break-word; }
  .post.continuation .body { padding-left: 0; }
  .attachment {
    border-left: 3px solid var(--rule);
    background: var(--surface);
    padding: 8px 12px;
    margin: 6px 0;
    border-radius: 0 4px 4px 0;
    font-size: 13.5px;
  }
  .attachment-title { font-weight: 600; margin-bottom: 3px; }
  .pretext { color: var(--ink-soft); margin-bottom: 4px; }
  .fields { display: grid; grid-template-columns: auto 1fr; gap: 2px 12px; margin: 6px 0 0; }
  .fields dt { font-weight: 600; color: var(--ink-soft); }
  .fields dd { margin: 0; }

  /* Even space above and below, which the CSS numbers alone do not give: the
     text line above contributes about 2.5px of half-leading below its glyphs,
     while below there is only the row padding and the footer's own negative
     margin. 4px above read as roughly double the gap below. */
  .reactions {
    display: flex;
    gap: 5px;
    margin: 2px 0;
    flex-wrap: wrap;
  }
  /* The actions float in the message's top-right corner rather than sitting in
     the flow: they are invisible at rest, and an invisible affordance must not
     reserve a line -- the old always-present reaction strip put 22px of nothing
     between a message and its thread footer. Nothing shifts on hover either. */
  /* A control that has to look like what it replaced: a face and a name, not a
     button. */
  .who-button {
    display: inline-flex;
    padding: 0;
    border: 0;
    background: none;
    font: inherit;
    color: inherit;
    cursor: pointer;
  }
  .who-button:hover {
    text-decoration: underline;
  }
  .tools {
    position: absolute;
    top: -2px;
    right: 4px;
    display: flex;
    align-items: center;
    gap: 3px;
    padding: 3px;
    border: 1px solid var(--rule);
    border-radius: 8px;
    /* Its own ground, because it can sit over the end of a long line. */
    background: var(--surface);
    box-shadow: 0 1px 4px rgb(0 0 0 / 0.14);
    opacity: 0;
  }
  .post:hover .tools,
  .tools:focus-within,
  .tools:has(.menu) {
    opacity: 1;
  }
  /* Square, and each glyph centred in its own box: a `summary` is a
     `list-item` by default, which is why the smiley sat off-centre next to the
     two real buttons. */
  .tool, .tools .picker summary {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    padding: 0;
    font: inherit;
    font-size: 14px;
    line-height: 1;
    border: 0;
    border-radius: 5px;
    background: none;
    color: var(--ink-soft);
    cursor: pointer;
  }
  .tool:hover, .tools .picker summary:hover {
    background: var(--ground);
    color: var(--ink);
  }
  /* Downward by default -- anchored to the top of the message, that is where
     the room usually is -- and upward when it is not. `placePicker` decides,
     because the scroller clips at both ends and a fixed side is wrong at one
     of them. */
  .tools .choices {
    top: 100%;
    bottom: auto;
    right: 0;
    left: auto;
  }
  .tools .choices.up {
    top: auto;
    bottom: 100%;
  }
  .menu {
    position: absolute;
    top: 100%;
    right: 0;
    z-index: 5;
    min-width: 190px;
    margin: 2px 0 0;
    padding: 4px;
    list-style: none;
    border: 1px solid var(--rule);
    border-radius: 6px;
    background: var(--surface);
    box-shadow: 0 6px 18px rgb(0 0 0 / 0.22);
  }
  .menu button {
    display: block;
    width: 100%;
    text-align: left;
    font: inherit;
    font-size: 12.5px;
    padding: 5px 8px;
    border: 0;
    border-radius: 4px;
    background: none;
    color: var(--ink);
    cursor: pointer;
  }
  .menu button:hover { background: var(--ground); }
  .menu .rule {
    height: 1px;
    margin: 4px 2px;
    background: var(--rule);
  }
  .editing {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 2px 0 4px;
  }
  .editing textarea {
    font: inherit;
    font-size: 14px;
    line-height: 1.45;
    resize: vertical;
    padding: 6px 8px;
    border: 1px solid var(--signal);
    border-radius: 5px;
    background: var(--ground);
    color: var(--ink);
  }
  .editing .hint {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11.5px;
    color: var(--ink-faint);
  }
  .editing button {
    font: inherit;
    font-size: 12px;
    padding: 3px 9px;
    border: 1px solid var(--rule);
    border-radius: 4px;
    background: var(--surface);
    color: var(--ink);
    cursor: pointer;
  }
  .confirm {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    font-size: 12.5px;
    color: var(--ink);
  }
  .confirm button {
    font: inherit;
    font-size: 12px;
    padding: 2px 8px;
    border: 1px solid var(--rule);
    border-radius: 4px;
    background: var(--surface);
    color: var(--ink);
    cursor: pointer;
  }
  .confirm button.danger {
    border-color: var(--flag);
    color: var(--flag);
  }
  /* What the last action did, under the message it acted on: a menu click with
     no visible result is indistinguishable from one that failed. */
  .outcome {
    margin: 2px 0 0;
    font-size: 11.5px;
    color: var(--ink-faint);
  }
  .pill {
    font: inherit;
    font-size: 11.5px;
    border: 1px solid var(--rule);
    background: var(--surface);
    border-radius: 10px;
    padding: 1px 7px;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .pill:hover { border-color: var(--ink-faint); }
  .pill.mine { border-color: var(--signal); color: var(--signal); }
  .tally { font-family: var(--mono); font-size: 10.5px; }
  .pill-emoji { height: 15px; width: auto; vertical-align: -0.2em; }
  /* The picker is a `details`, so closing it needs no state and no outside
     click handler -- the browser already owns that behaviour. */
  .picker { position: relative; }
  .picker summary {
    list-style: none;
    cursor: pointer;
    font-size: 11.5px;
    color: var(--ink-faint);
    border: 1px solid transparent;
    border-radius: 10px;
    padding: 1px 7px;
  }
  .picker summary::-webkit-details-marker { display: none; }
  .picker summary:hover { border-color: var(--rule); color: var(--ink); }
  .choices {
    position: absolute;
    bottom: 100%;
    left: 0;
    z-index: 2;
    display: flex;
    gap: 2px;
    padding: 4px;
    border: 1px solid var(--rule);
    border-radius: 8px;
    background: var(--surface);
    box-shadow: 0 4px 12px rgb(0 0 0 / 0.18);
  }
  .choices button {
    font: inherit;
    font-size: 15px;
    line-height: 1;
    border: 0;
    background: transparent;
    border-radius: 5px;
    padding: 3px;
    cursor: pointer;
  }
  .choices button:hover { background: var(--surface-2); }
  /* Set apart from the faces: it opens a search rather than reacting. */
  .choices .more {
    font-size: 13px;
    color: var(--ink-faint);
    border-left: 1px solid var(--rule);
    border-radius: 0 5px 5px 0;
    padding-left: 5px;
  }
  /* An empty row of reactions must not take height: the picker only appears on
     hover over the post. */
  .footer.unread { color: var(--ink); font-weight: 600; }
  /* A mention inside a thread is the one count worth colouring: it is the
     difference between "there is more here" and "you are being asked". */
  .count.mention {
    background: var(--flag);
    color: var(--ground);
    border-radius: 8px;
    padding: 0 5px;
    font-size: 11px;
    font-weight: 700;
  }
  .new {
    color: var(--flag);
    font-weight: 600;
  }
  .footer {
    align-self: flex-start;
    display: flex;
    gap: 10px;
    /* Centred, not baseline-aligned: an avatar has no text baseline, so
       `baseline` left the reply count sitting off against the faces. */
    align-items: center;
    background: none;
    border: 0;
    color: var(--signal);
    font: inherit;
    font-size: 12.5px;
    cursor: pointer;
    /* Bound to the message above it rather than floating between two.
       Indented into the text column -- level with the avatar it read as a
       sibling of the *next* post instead of the tail of its own -- and the
       spacing is asymmetric on purpose: almost none above, a clear gap below. */
    margin: -2px 0 10px 36px;
    padding: 0;
  }
  .footer:hover { text-decoration: underline; }
  /* Overlapped, as every chat client draws a group: it reads as one object
     rather than a row of separate people. */
  .faces {
    display: inline-flex;
    align-items: center;
    /* Its own line box would otherwise add height above the faces and push the
       row's centre away from the text. */
    line-height: 1;
  }
  .face {
    display: inline-flex;
    margin-right: -6px;
    border-radius: 50%;
    /* A ring in the page colour is what separates one face from the next. */
    box-shadow: 0 0 0 2px var(--ground);
  }
  /* The footer's own gap provides the space after the group. */
  .faces > :last-child { margin-right: 0; }
  .more {
    width: 20px;
    height: 20px;
    align-items: center;
    justify-content: center;
    background: var(--surface-2);
    color: var(--ink-faint);
    font-size: 10px;
    font-weight: 600;
  }
  .system { font-size: 12.5px; color: var(--ink-faint); font-style: italic; margin: 2px 0; }
  /* A guess on screen must look like a guess. */
  .post.pending { opacity: 0.55; }
  .post.failed { opacity: 1; }
  .sending { font-size: 11px; color: var(--ink-faint); }
  .send-failed { font-size: 12px; color: var(--flag); margin: 3px 0 0; display: flex; gap: 8px; align-items: baseline; }
  .send-failed button {
    font: inherit;
    font-size: 12px;
    background: none;
    border: 0;
    color: var(--signal);
    cursor: pointer;
    padding: 0;
    text-decoration: underline;
  }
</style>
