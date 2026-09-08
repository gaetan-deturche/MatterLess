<script lang="ts">

  // Our own scrollbar for the message stream.
  //
  // A native thumb is *derived* from the scroll position, so anything else that
  // writes that position -- a height correction, a page of history landing,
  // rows re-measuring -- moves the thumb out from under the reader's hand in
  // the middle of a drag. Measured in this app that cost about half the
  // gesture: the thumb followed the mouse at a gain of 0.4 to 0.8, differently
  // every time, and never caught up.
  //
  // This one runs the other way round. While the thumb is held it is drawn
  // exactly where the pointer put it, and the list is scrolled to match. The
  // reader's hand is the input; the scroll position is the output. Whatever the
  // content does underneath, the thumb stays under the cursor.

  let {
    top,
    viewport,
    content,
    onmove,
    ondragging,
  }: {
    /** Where the list is now. Ignored while the thumb is held. */
    top: number;
    /** The scroller's visible height, which is also the track's length. */
    viewport: number;
    /** The scroller's full content height. */
    content: number;
    onmove: (top: number) => void;
    ondragging: (active: boolean) => void;
  } = $props();

  /** A thumb shorter than this is not worth aiming at. Unbounded history makes
   *  the proportional height tend to nothing, so it needs a floor -- and having
   *  a floor is exactly why the mapping has to be written out rather than left
   *  to a proportion. */
  const MINIMUM_THUMB = 28;
  /** A wheel over the bar scrolls the list rather than doing nothing. */
  const WHEEL_STEP = 1;
  /** Breathing room at each end, so a thumb that has reached the end sits
   *  against something rather than running off the edge of the pane. */
  const EDGE = 2;

  const range = $derived(Math.max(0, content - viewport));
  /** The rail's usable length: the pane, less the margin at each end. */
  const rail = $derived(Math.max(1, viewport - EDGE * 2));
  const thumb = $derived(
    range <= 0 ? 0 : Math.max(MINIMUM_THUMB, Math.min(rail, (viewport / content) * rail)),
  );
  /** How far the thumb can travel: the rail minus the thumb itself. */
  const travel = $derived(Math.max(1, rail - thumb));

  /** Set while the thumb is held, and then it owns where the thumb is drawn. */
  let held: number | undefined = $state();
  /** Clamped, always.
   *
   *  `top` and `content` are read at different moments, so a list that grew
   *  since the last measurement reports a position past the end of its own
   *  range -- and an unclamped thumb is then drawn below the track, where the
   *  pane clips it and it looks cut off. */
  const position = $derived(
    held ?? (range <= 0 ? 0 : Math.max(0, Math.min(travel, (top / range) * travel))),
  );

  /** Within a pixel of an end. Being at the end of the history is worth saying
   *  out loud: a thumb a few pixels short of the rail's end looks the same as
   *  one that still has somewhere to go. */
  const atTop = $derived(position <= 1);
  const atEnd = $derived(position >= travel - 1);

  let grabPointer = 0;
  let grabPosition = 0;

  function scrollToThumb(at: number) {
    onmove((Math.max(0, Math.min(travel, at)) / travel) * range);
  }

  function grab(event: PointerEvent) {
    if (event.button !== 0 || range <= 0) return;
    // Without this the track underneath treats the same press as a jump.
    event.stopPropagation();
    event.preventDefault();
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    grabPointer = event.clientY;
    grabPosition = position;
    held = position;
    ondragging(true);
  }

  function drag(event: PointerEvent) {
    if (held === undefined) return;
    // Measured from where the thumb was grabbed, not accumulated frame by
    // frame: an accumulated delta drifts by whatever each frame fails to apply,
    // which is the failure this component exists to avoid.
    const at = Math.max(0, Math.min(travel, grabPosition + (event.clientY - grabPointer)));
    held = at;
    scrollToThumb(at);
  }

  function release(event: PointerEvent) {
    if (held === undefined) return;
    (event.currentTarget as HTMLElement).releasePointerCapture(event.pointerId);
    held = undefined;
    ondragging(false);
  }

  /** A press on the track jumps there, with the thumb centred on the pointer. */
  function jump(event: PointerEvent) {
    if (event.button !== 0 || range <= 0) return;
    const box = (event.currentTarget as HTMLElement).getBoundingClientRect();
    scrollToThumb(event.clientY - box.top - thumb / 2);
  }
</script>

{#if range > 0}
  <div
    class="track"
    style:--edge="{EDGE}px"
    role="scrollbar"
    tabindex="-1"
    aria-orientation="vertical"
    aria-label="Message history"
    aria-controls="stream-scroll"
    aria-valuemin={0}
    aria-valuemax={Math.round(range)}
    aria-valuenow={Math.round(held === undefined ? top : (held / travel) * range)}
    onpointerdown={jump}
    onwheel={(event) => onmove(top + event.deltaY * WHEEL_STEP)}
  >
    <!-- The rail the thumb runs in. Without something to be flush against,
         there is no way to see that the list has run out. -->
    <div class="rail"></div>
    <div
      class="thumb"
      role="presentation"
      class:held={held !== undefined}
      class:at-top={atTop}
      class:at-end={atEnd}
      style:height="{thumb}px"
      style:transform="translateY({EDGE + position}px)"
      onpointerdown={grab}
      onpointermove={drag}
      onpointerup={release}
      onpointercancel={release}
    ></div>
  </div>
{/if}

<style>
  .track {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    width: 12px;
    z-index: 4;
    /* Nothing behind it should react to a press aimed at the bar. */
    background: transparent;
  }
  .rail {
    position: absolute;
    top: var(--edge);
    right: 4px;
    bottom: var(--edge);
    width: 4px;
    border-radius: 2px;
    background: var(--ink-faint, #7a7f87);
    opacity: 0.14;
    pointer-events: none;
  }
  .thumb {
    position: absolute;
    top: 0;
    right: 2px;
    width: 8px;
    border-radius: 4px;
    background: var(--ink-faint, #7a7f87);
    opacity: 0.42;
    cursor: default;
  }
  .track:hover .rail {
    opacity: 0.24;
  }
  .track:hover .thumb,
  .thumb.held {
    opacity: 0.75;
  }
  /* At an end, the thumb brightens rather than changing shape: a flattened
     corner reads as the thumb being cut off, which is the opposite of the
     "you have run out" this is for. The rail behind it supplies the reference
     for where the ends are. */
  .thumb.at-top,
  .thumb.at-end {
    opacity: 0.6;
  }
  .thumb.held {
    /* Held: no transition, or the thumb would ease towards the cursor instead
       of being at it. */
    transition: none;
  }
</style>
