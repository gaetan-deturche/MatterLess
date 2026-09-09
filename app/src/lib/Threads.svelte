<script lang="ts">
  // The threads this reader follows, newest activity first.
  //
  // This view exists because collapsed threads make a reply invisible
  // everywhere else: it never bumps its channel's unread counters, so the
  // sidebar stays silent while the badge counts it. Without somewhere to list
  // them, a thread notification pointed at nothing the app could show.
  //
  // Read from the local table, which the websocket keeps current -- so the list
  // is redrawn from a `thread_changed` delta rather than polled.
  import Avatar from "./Avatar.svelte";
  import * as api from "./api";
  import * as log from "./log";

  let {
    version,
    onopen,
    onclose,
  }: {
    /** Bumped by the shell on every `thread_changed` delta. Reloading on it
     *  rather than on the named root, because the list is ordered by activity:
     *  one reply can move any number of rows. */
    version: number;
    /** Go to a thread: its channel, then the thread itself. */
    onopen: (channelId: string, rootId: string) => void;
    onclose: () => void;
  } = $props();

  let threads: api.ThreadListing[] = $state([]);
  let loading = $state(true);
  let failure = $state("");
  /** Unread first, or everything. */
  let unreadOnly = $state(false);

  const shown = $derived(
    unreadOnly ? threads.filter((thread) => thread.unread_replies > 0) : threads,
  );
  const unreadCount = $derived(threads.filter((thread) => thread.unread_replies > 0).length);

  async function load() {
    try {
      threads = await api.followedThreads();
      failure = "";
      log.debug("threads.listed", { threads: threads.length, unread: unreadCount });
    } catch (thrown) {
      failure = String(thrown);
      log.failure("threads.list.failed", thrown);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    version;
    void load();
  });

  /** Short, relative, and good enough: this list is read by recency, not by
   *  timestamp. */
  function when(at: number): string {
    const ago = Date.now() - at;
    const minutes = Math.round(ago / 60000);
    if (minutes < 1) return "now";
    if (minutes < 60) return `${minutes}m`;
    const hours = Math.round(minutes / 60);
    if (hours < 24) return `${hours}h`;
    return `${Math.round(hours / 24)}d`;
  }
</script>

<svelte:window
  onkeydown={(event) => {
    if (event.key === "Escape") onclose();
  }}
/>

<section class="threads" aria-label="Threads">
  <header>
    <h2>Threads</h2>
    <button
      type="button"
      class="filter"
      class:on={unreadOnly}
      onclick={() => (unreadOnly = !unreadOnly)}
    >
      Unread{unreadCount > 0 ? ` (${unreadCount})` : ""}
    </button>
    <button type="button" class="close" onclick={onclose} aria-label="Close threads">×</button>
  </header>

  {#if failure}
    <p class="hint failure">{failure}</p>
  {:else if loading && threads.length === 0}
    <p class="hint">Loading…</p>
  {:else if shown.length === 0}
    <p class="hint">
      {unreadOnly ? "Nothing unread." : "You are not following any thread yet."}
    </p>
  {/if}

  <ul>
    {#each shown as thread (thread.root_id)}
      <li>
        <button
          type="button"
          class="row"
          class:unread={thread.unread_replies > 0}
          onclick={() => onopen(thread.channel_id, thread.root_id)}
        >
          <Avatar userId={thread.author_id} name={thread.author} size={28} />
          <span class="body">
            <span class="line">
              <span class="who">{thread.author}</span>
              <span class="where">{thread.channel}</span>
              <span class="when">{when(thread.last_reply_at)}</span>
            </span>
            <span class="what">{thread.message}</span>
            <span class="line">
              <span class="count">
                {thread.reply_count}
                {thread.reply_count === 1 ? "reply" : "replies"}
              </span>
              {#if thread.unread_mentions > 0}
                <span class="badge mention">{thread.unread_mentions}</span>
              {:else if thread.unread_replies > 0}
                <span class="badge">{thread.unread_replies} new</span>
              {/if}
            </span>
          </span>
        </button>
      </li>
    {/each}
  </ul>
</section>

<style>
  .threads {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    border-left: 1px solid var(--rule);
    background: var(--ground);
  }
  header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--rule);
  }
  h2 {
    flex: 1;
    margin: 0;
    font-size: 14px;
  }
  .filter {
    font: inherit;
    font-size: 12px;
    padding: 2px 8px;
    border: 1px solid var(--rule);
    border-radius: 10px;
    background: transparent;
    color: var(--ink-faint);
    cursor: pointer;
    white-space: nowrap;
  }
  .filter.on {
    color: var(--signal);
    border-color: var(--signal);
  }
  .close {
    font: inherit;
    font-size: 16px;
    line-height: 1;
    border: 0;
    background: transparent;
    color: var(--ink-faint);
    cursor: pointer;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    min-height: 0;
    overflow-y: auto;
  }
  .row {
    display: flex;
    gap: 9px;
    width: 100%;
    text-align: left;
    font: inherit;
    padding: 9px 12px;
    border: 0;
    border-bottom: 1px solid var(--rule);
    background: none;
    color: var(--ink);
    cursor: pointer;
  }
  .row:hover {
    background: var(--surface);
  }
  .row.unread .who {
    font-weight: 600;
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
    flex: 1;
  }
  .line {
    display: flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
  }
  .who {
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .where {
    flex: 1;
    font-size: 11.5px;
    color: var(--ink-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .when {
    font-size: 11px;
    color: var(--ink-faint);
    white-space: nowrap;
  }
  .what {
    font-size: 12.5px;
    color: var(--ink-faint);
    /* Two lines of the root, so a long opening post cannot push the rest of
       the list off the screen. */
    display: -webkit-box;
    line-clamp: 2;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    overflow-wrap: anywhere;
  }
  .count {
    font-size: 11.5px;
    color: var(--signal);
  }
  .badge {
    font-size: 11px;
    padding: 0 6px;
    border-radius: 8px;
    background: var(--rule);
    color: var(--ink);
  }
  .badge.mention {
    background: var(--flag);
    color: #fff;
  }
  .hint {
    margin: 0;
    padding: 12px;
    font-size: 12px;
    color: var(--ink-faint);
  }
  .failure {
    color: var(--flag);
    overflow-wrap: anywhere;
  }
</style>
