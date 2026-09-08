<script lang="ts">
  // Message search: the local index answers while you type, the server's answer
  // folds in when it lands. Both halves happen in Rust; this only draws them.
  import Hits from "./Hits.svelte";
  import * as api from "./api";
  import * as store from "./store.svelte";
  import * as log from "./log";

  let {
    initial = "",
    onjump,
    onclose,
  }: {
    /** A query submitted from the header field, run as soon as it arrives. */
    initial?: string;
    /** Go to this message: the channel, then the post inside it. */
    onjump: (channelId: string, postId: string) => void;
    onclose: () => void;
  } = $props();

  let query = $state("");
  let hits: api.SearchHit[] = $state([]);
  let searching = $state(false);
  let ran = $state(false);
  let failure = $state("");
  /** Only the newest search may write the list. */
  let asked = 0;

  // A query handed over by the header field runs immediately, and again if the
  // reader submits a different one while this pane is already open -- otherwise
  // the second search from the header would look like it did nothing.
  let seeded = $state("");
  $effect(() => {
    if (!initial || initial === seeded) return;
    seeded = initial;
    query = initial;
    void run();
  });

  /** Searching is a *submit*, not a keystroke: a query goes to the server
   *  across every team, so firing one per character would be several requests
   *  per word for an answer nobody has finished typing. */
  async function run() {
    const typed = query.trim();
    if (!typed) return;
    const meta = store.meta();
    if (!meta) return;
    const mine = ++asked;
    searching = true;
    failure = "";
    try {
      const found = await api.searchMessages(typed, meta.meId, meta.displayMode);
      if (mine !== asked) return;
      hits = found;
      ran = true;
      log.info("search.ran", { chars: typed.length, hits: found.length });
    } catch (thrown) {
      if (mine !== asked) return;
      failure = String(thrown);
      log.failure("search.failed", thrown, {});
    } finally {
      if (mine === asked) searching = false;
    }
  }

  function onKeyDown(event: KeyboardEvent) {
    if (event.key === "Enter" && !event.isComposing) {
      event.preventDefault();
      void run();
    } else if (event.key === "Escape") {
      event.preventDefault();
      onclose();
    }
  }

  const when = (ms: number) =>
    new Date(ms).toLocaleString([], {
      day: "numeric",
      month: "short",
      hour: "2-digit",
      minute: "2-digit",
    });
</script>

<aside class="pane">
  <header>
    <input
      bind:value={query}
      onkeydown={onKeyDown}
      placeholder="Search messages… (Enter to search)"
      spellcheck="false"
    />
    <button type="button" class="close" onclick={onclose} aria-label="Close search">×</button>
  </header>

  <p class="hint">
    <code>from:amy</code>, <code>in:town-square</code>, <code>on:2026-09-04</code>,
    <code>before:</code> and <code>after:</code> — honoured locally and on the server.
  </p>

  {#if failure}
    <p class="failure">{failure}</p>
  {:else if searching && hits.length === 0}
    <p class="hint">Searching…</p>
  {:else if ran && hits.length === 0}
    <p class="hint">Nothing found.</p>
  {/if}

  <Hits {hits} {onjump} />
</aside>

<style>
  .pane {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-left: 1px solid var(--rule);
    background: var(--surface);
  }
  header {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 10px;
    border-bottom: 1px solid var(--rule);
  }
  input {
    flex: 1;
    font: inherit;
    font-size: 13.5px;
    padding: 6px 8px;
    border: 1px solid var(--rule);
    border-radius: 5px;
    background: var(--ground);
    color: var(--ink);
  }
  .close {
    font: inherit;
    font-size: 16px;
    line-height: 1;
    padding: 2px 7px;
    border: 0;
    border-radius: 4px;
    background: none;
    color: var(--ink-soft);
    cursor: pointer;
  }
  .hint,
  .failure {
    margin: 0;
    padding: 8px 12px;
    font-size: 11.5px;
    line-height: 1.5;
    color: var(--ink-faint);
  }
  .hint code {
    font-family: var(--mono);
    font-size: 11px;
  }
  .failure {
    color: var(--flag);
  }
</style>
