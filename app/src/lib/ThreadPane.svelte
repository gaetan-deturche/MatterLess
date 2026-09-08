<script lang="ts">
  // A thread, beside the channel. Rows come from Rust exactly as the channel's
  // do -- `plan_thread` is the same builder with threads forced flat -- so this
  // pane makes no rendering decisions of its own either.
  //
  // Not virtualised: a thread is tens of rows, and mounting them all costs less
  // than the height bookkeeping would.
  import MessageList from "./MessageList.svelte";
  import Composer from "./Composer.svelte";
  import * as api from "./api";
  import * as store from "./store.svelte";
  import * as log from "./log";

  let { rootId, onclose }: { rootId: string; onclose: () => void } = $props();

  let payload = $state<api.ThreadPayload | undefined>();
  let busy = $state(false);

  /** Loads the pane and reports what it drew. Returns the payload so callers
   *  can act on it without re-reading state Svelte may not have narrowed. */
  async function load(): Promise<api.ThreadPayload | undefined> {
    const meta = store.meta();
    if (!meta) return undefined;
    const next = await api.threadRows(rootId, meta.meId, meta.displayMode, store.fullRes());
    payload = next;
    log.debug("thread.rendered", {
      root: rootId,
      rows: next.rows.length,
      replies: next.reply_count,
      unread: next.unread_replies,
      buildMs: next.build_ms.toFixed(2),
    });
    return next;
  }

  /** Paints from SQLite, then reconciles with the server. */
  $effect(() => {
    const target = rootId;
    // Read here on purpose, so this effect depends on it: the setting travels
    // with the request and decides the box each image gets, and the read
    // inside `load` happens after an await where it would not be tracked.
    void store.fullRes();
    payload = undefined;
    void (async () => {
      try {
        await load();
        // The replies held locally are only whatever happened to arrive live,
        // so a thread almost always needs the fetch -- after the paint, not
        // before it.
        await api.refreshThread(target);
        if (target !== rootId) return;
        const fetched = await load();
        // Opening a thread is reading it: those counts are why you clicked.
        if (target === rootId && (fetched?.unread_replies ?? 0) > 0) {
          await api.markThreadRead(target);
          store.markThreadRead(target);
          await load();
        }
      } catch (thrown) {
        log.failure("thread.load.failed", thrown, { root: target });
      }
    })();
  });

  async function toggleFollow() {
    if (!payload || busy) return;
    busy = true;
    try {
      const next = !payload.following;
      await api.setThreadFollowing(rootId, next);
      payload = { ...payload, following: next };
    } catch (thrown) {
      log.failure("thread.follow.failed", thrown, { root: rootId });
    } finally {
      busy = false;
    }
  }
</script>

<aside class="pane">
  <header>
    <strong>Thread</strong>
    <span class="replies">
      {payload?.reply_count ?? 0}
      {(payload?.reply_count ?? 0) === 1 ? "reply" : "replies"}
    </span>
    <button type="button" class="follow" class:on={payload?.following} onclick={toggleFollow}>
      {payload?.following ? "Following" : "Follow"}
    </button>
    <button type="button" class="close" onclick={onclose} aria-label="Close thread">×</button>
  </header>

  <div class="scroll">
    <MessageList
      compact
      rows={payload?.rows}
      channelId={`thread/${rootId}`}
      virtualise={false}
    />
  </div>

  {#if payload?.channel_id}
    <!-- Typing in a thread belongs here rather than under the channel: the
         event arrives on the thread's channel, so it used to read as somebody
         typing in the stream behind this pane. Reserved height, so it does not
         push the composer down when it appears. -->
    <p class="typing" aria-live="polite">
      {#if store.typingIn(payload.channel_id, rootId).length}
        {store.typingIn(payload.channel_id, rootId).length} typing…
      {/if}
    </p>
    <!-- A reply is an ordinary send with a root id, so it reaches the screen
         through the same optimistic path as anything else. -->
    <Composer
      channelId={payload.channel_id}
      {rootId}
      maxFileSize={store.meta()?.maxFileSize ?? 0}
    />
  {/if}
</aside>

<style>
  .typing {
    font-size: 12px;
    line-height: 16px;
    height: 16px;
    color: var(--ink-faint);
    padding: 0 12px 8px;
    margin: 0;
  }
  .pane {
    display: flex;
    flex-direction: column;
    min-height: 0;
    /* Without this a long word in a reply can widen the pane and push the
       stream out of the window rather than wrapping. */
    min-width: 0;
    border-left: 1px solid var(--rule);
    background: var(--ground);
  }
  header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--rule);
  }
  .replies {
    color: var(--ink-faint);
    font-size: 12px;
    margin-right: auto;
    white-space: nowrap;
  }
  header strong { white-space: nowrap; }
  .follow {
    font: inherit;
    font-size: 12px;
    padding: 2px 8px;
    border: 1px solid var(--rule);
    border-radius: 10px;
    background: transparent;
    color: var(--ink-faint);
    cursor: pointer;
  }
  .follow.on { color: var(--signal); border-color: var(--signal); }
  .close {
    font: inherit;
    font-size: 16px;
    line-height: 1;
    border: 0;
    background: transparent;
    color: var(--ink-faint);
    cursor: pointer;
  }
  /* `overflow-anchor: none` for the same reason as the channel scroller: this
     pane grows as replies arrive, and the browser's anchoring fights a drag. */
  .scroll { flex: 1; overflow-y: auto; min-height: 0; overflow-anchor: none; }
</style>
