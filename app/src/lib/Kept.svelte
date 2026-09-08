<script lang="ts">
  // Saved messages and a channel's pinned messages.
  //
  // One pane for both because they are the same view of the same thing: a list
  // of posts you have marked, drawn the way search results are and opening the
  // same way. What differs is the question asked of the server, which is one
  // call either way.
  //
  // Both are fetched rather than read locally. Saving is a *preference*, so the
  // store knows which ids are saved without necessarily holding the posts --
  // and a message saved from a channel this client has never opened is exactly
  // the one worth listing.
  import Hits from "./Hits.svelte";
  import * as api from "./api";
  import * as store from "./store.svelte";
  import * as log from "./log";

  let {
    mode,
    channelId,
    channelLabel,
    onjump,
    onclose,
  }: {
    mode: "saved" | "pinned";
    /** The channel whose pins to show. Ignored when saved. */
    channelId: string;
    channelLabel: string;
    onjump: (channelId: string, postId: string) => void;
    onclose: () => void;
  } = $props();

  let hits: api.SearchHit[] = $state([]);
  let loading = $state(false);
  let failure = $state("");
  /** Only the newest request may write the list. */
  let asked = 0;

  const title = $derived(mode === "saved" ? "Saved messages" : `Pinned in ${channelLabel}`);

  $effect(() => {
    const meta = store.meta();
    if (!meta) return;
    const wanted = mode;
    const where = channelId;
    const mine = ++asked;
    loading = true;
    failure = "";
    void (async () => {
      try {
        const found =
          wanted === "saved"
            ? await api.savedMessages(meta.meId, meta.displayMode)
            : await api.pinnedMessages(where, meta.meId, meta.displayMode);
        if (mine !== asked) return;
        hits = found;
      } catch (thrown) {
        if (mine !== asked) return;
        failure = String(thrown);
        log.failure("kept.failed", thrown, { mode: wanted });
      } finally {
        if (mine === asked) loading = false;
      }
    })();
  });
</script>

<aside class="pane">
  <header>
    <h2>{title}</h2>
    <button type="button" class="close" onclick={onclose} aria-label="Close">×</button>
  </header>

  {#if failure}
    <p class="failure">{failure}</p>
  {:else if loading && hits.length === 0}
    <p class="hint">Loading…</p>
  {:else if !loading && hits.length === 0}
    <p class="hint">
      {mode === "saved"
        ? "Nothing saved yet. Save a message from its ⋯ menu."
        : "Nothing pinned in this channel yet."}
    </p>
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
    gap: 8px;
    padding: 10px 10px 6px;
  }
  h2 {
    flex: 1;
    margin: 0;
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .close {
    font: inherit;
    font-size: 16px;
    line-height: 1;
    padding: 2px 7px;
    border: 1px solid var(--rule);
    border-radius: 5px;
    background: var(--ground);
    color: var(--ink-faint);
    cursor: pointer;
  }
  .close:hover {
    color: var(--ink);
  }
  .hint,
  .failure {
    margin: 0;
    padding: 2px 12px 8px;
    font-size: 12px;
    color: var(--ink-faint);
  }
  .failure {
    color: var(--flag);
    overflow-wrap: anywhere;
  }
</style>
