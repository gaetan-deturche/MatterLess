<script lang="ts">
  // The full emoji set, for a reaction the quick row does not offer.
  //
  // Browsed by category the way every other client shows it, and searched when
  // the reader already knows the name. The categories come from the dataset
  // Mattermost's own webapp groups by, cross-checked against the table the
  // renderer draws from -- so a tile can never be a reaction that would come
  // back on screen as `:name:`.
  //
  // Search goes through the same `suggest` the composer's `:` completion uses,
  // so the two cannot disagree about what exists or how it ranks.
  //
  // Positioned `fixed` and clamped into the window, like `ProfileCard`: the
  // message list clips at both ends, and a panel this size would be cut off
  // wherever the message happened to be.
  import * as api from "./api";
  import * as media from "./media";
  import * as store from "./store.svelte";
  import * as log from "./log";

  let {
    at,
    onpick,
    onclose,
  }: {
    /** Where the click was, in viewport coordinates. */
    at: { x: number; y: number };
    onpick: (name: string) => void;
    onclose: () => void;
  } = $props();

  /** Enough to fill the grid several rows deep: `suggest`'s default of eight is
   *  sized for a completion list under a caret, not for browsing. */
  const RESULTS = 64;
  /** Custom emoji lead: they are this server's own, and the usual reason for
   *  opening the full picker rather than taking a face from the quick row. */
  const CUSTOM = "Custom";

  /** Its size before it has been measured. Only ever used for the first frame:
   *  the panel is padded and bordered, so a constant is wrong by that much --
   *  which is how it came to hang off the edge of the window. */
  const ASSUMED = { width: 322, height: 300 };
  const MARGIN = 8;
  const BORDERS = 2;

  let measuredWidth = $state(0);
  let measuredHeight = $state(0);

  const placed = $derived.by(() => {
    const width = (measuredWidth || ASSUMED.width) + BORDERS;
    const height = (measuredHeight || ASSUMED.height) + BORDERS;
    const rightmost = Math.max(MARGIN, window.innerWidth - width - MARGIN);
    const lowest = Math.max(MARGIN, window.innerHeight - height - MARGIN);
    return {
      left: Math.min(Math.max(MARGIN, at.x - width / 2), rightmost),
      // Below the button when it fits, above it when it does not.
      top:
        at.y + 10 + height + MARGIN <= window.innerHeight
          ? at.y + 10
          : Math.min(Math.max(MARGIN, at.y - 10 - height), lowest),
    };
  });

  let query = $state("");
  let found: api.Suggestion[] = $state([]);
  let searching = $state(false);
  let field: HTMLInputElement | undefined = $state();

  let categories: api.EmojiCategory[] = $state([]);
  let active = $state(CUSTOM);

  /** This server's own emoji, from the catalogue bootstrap already fetched. */
  const custom = $derived(
    Object.entries(store.emojiIds()).sort(([one], [other]) => one.localeCompare(other)),
  );

  const tabs = $derived([
    ...(custom.length > 0 ? [CUSTOM] : []),
    ...categories.map((category) => category.label),
  ]);

  /** What the grid shows when nothing is being searched for. */
  const browsing = $derived.by(() => {
    if (active === CUSTOM) {
      return custom.map(([name, id]) => ({ name, face: "", id }));
    }
    const category = categories.find((candidate) => candidate.label === active);
    return (category?.emoji ?? []).map(([name, face]) => ({ name, face, id: "" }));
  });

  /** Search results in the same shape, so the grid draws one kind of thing. */
  const results = $derived(
    found.map((option) => ({ name: option.value, face: option.detail, id: option.id })),
  );

  const showing = $derived(query.trim().length > 0 ? results : browsing);

  /** How many tiles are drawn at once.
   *
   *  Every custom tile is an image request, and on a cold cache the client
   *  paces those at eight a second -- so drawing a whole category at once asked
   *  for 707 images the reader could see fewer than fifty of. More are drawn as
   *  the grid is scrolled. */
  const PAGE = 96;
  let drawn = $state(PAGE);

  $effect(() => {
    // A different tab or a new search starts the window again.
    active;
    query;
    drawn = PAGE;
  });

  const visible = $derived(showing.slice(0, drawn));

  /** Draws the next page when the end of the grid comes into view. */
  function whenSeen(element: HTMLElement) {
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) drawn += PAGE;
      },
      { root: element.parentElement },
    );
    observer.observe(element);
    return {
      destroy() {
        observer.disconnect();
      },
    };
  }

  $effect(() => {
    field?.focus();
  });

  $effect(() => {
    void api
      .emojiCategories()
      .then((groups) => {
        categories = groups;
        // Only fall back to the first standard tab when this server has no
        // custom emoji at all.
        if (custom.length === 0 && groups.length > 0) active = groups[0]!.label;
      })
      .catch((error) => log.warn("emoji.categories.failed", { error: String(error) }));
  });

  $effect(() => {
    const wanted = query.trim();
    if (wanted.length === 0) {
      found = [];
      return;
    }
    // Debounced: this is a keystroke away from a store query, and the reader is
    // still typing.
    searching = true;
    const timer = setTimeout(() => {
      void api
        .suggest("emoji", wanted, undefined, RESULTS)
        .then((matches) => {
          if (query.trim() !== wanted) return;
          found = matches;
        })
        .catch((error) => log.warn("emoji.search.failed", { error: String(error) }))
        .finally(() => {
          searching = false;
        });
    }, 120);
    return () => clearTimeout(timer);
  });
</script>

<svelte:window
  onkeydown={(event) => {
    if (event.key === "Escape") onclose();
  }}
/>

<!-- Catches the next click, the way the profile card's does. -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="catcher" aria-hidden="true" onclick={onclose}></div>

<div
  class="panel"
  role="dialog"
  aria-label="Pick a reaction"
  bind:clientWidth={measuredWidth}
  bind:clientHeight={measuredHeight}
  style:left="{placed.left}px"
  style:top="{placed.top}px"
>
  <input
    bind:this={field}
    bind:value={query}
    type="text"
    placeholder="Search emoji…"
    aria-label="Search emoji"
    onkeydown={(event) => {
      // Enter takes the best match, which is what the ranking is for.
      if (event.key === "Enter" && visible.length > 0) {
        event.preventDefault();
        onpick(visible[0]!.name);
      }
    }}
  />

  {#if query.trim().length === 0}
    <div class="tabs" role="tablist" aria-label="Emoji categories">
      {#each tabs as label (label)}
        <button
          type="button"
          role="tab"
          aria-selected={active === label}
          class:current={active === label}
          title={label}
          onclick={() => (active = label)}>{label}</button
        >
      {/each}
    </div>
  {/if}

  <div class="grid">
    {#each visible as option (option.name)}
      <button type="button" title=":{option.name}:" onclick={() => onpick(option.name)}>
        {#if option.id}
          <img src={media.emoji(option.id)} alt={option.name} loading="lazy" />
        {:else}
          <span>{option.face}</span>
        {/if}
      </button>
    {/each}
    {#if visible.length < showing.length}
      <div class="sentinel" use:whenSeen></div>
    {/if}
  </div>

  {#if showing.length === 0 && !searching}
    <p class="hint">
      {query.trim().length > 0 ? "Nothing matches that." : "Nothing here."}
    </p>
  {/if}
</div>

<style>
  .catcher {
    position: fixed;
    inset: 0;
    z-index: 70;
  }
  .panel {
    position: fixed;
    z-index: 71;
    /* Border-box, so the declared width is the width that has to fit on screen
       rather than the width before padding is added to it. */
    box-sizing: border-box;
    width: 320px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    border: 1px solid var(--rule);
    border-radius: 8px;
    background: var(--surface);
    box-shadow: 0 10px 30px rgb(0 0 0 / 0.3);
  }
  input {
    font: inherit;
    font-size: 13px;
    padding: 6px 8px;
    border: 1px solid var(--rule);
    border-radius: 5px;
    background: var(--ground);
    color: var(--ink);
  }
  input:focus {
    outline: none;
    border-color: var(--signal);
  }
  .tabs {
    display: flex;
    gap: 2px;
    overflow-x: auto;
    /* A control strip, not content: it must not grow the panel. */
    flex: none;
    scrollbar-width: none;
  }
  .tabs::-webkit-scrollbar {
    height: 0;
  }
  .tabs button {
    font: inherit;
    font-size: 11px;
    white-space: nowrap;
    padding: 3px 7px;
    border: 1px solid transparent;
    border-radius: 10px;
    background: transparent;
    color: var(--ink-faint);
    cursor: pointer;
  }
  .tabs button:hover {
    color: var(--ink);
  }
  .tabs button.current {
    border-color: var(--rule);
    background: var(--ground);
    color: var(--ink);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(8, 1fr);
    gap: 2px;
    /* A fixed window, so the panel is the same size whichever category is open
       and cannot grow past the bottom of the screen. */
    height: 216px;
    overflow-y: auto;
    align-content: start;
  }
  .grid button {
    display: grid;
    place-items: center;
    height: 34px;
    font: inherit;
    font-size: 19px;
    line-height: 1;
    border: 0;
    border-radius: 5px;
    background: transparent;
    cursor: pointer;
  }
  .grid button:hover {
    background: var(--ground);
  }
  .grid img {
    width: 21px;
    height: 21px;
    object-fit: contain;
  }
  .sentinel {
    grid-column: 1 / -1;
    height: 1px;
  }
  .hint {
    margin: 0;
    font-size: 12px;
    color: var(--ink-faint);
  }
</style>
