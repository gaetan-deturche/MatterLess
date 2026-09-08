<script lang="ts">
  // What a right-click on a channel offers.
  //
  // The menu does the work itself and then says it changed something, rather
  // than handing seven callbacks back up: every item here is one call plus a
  // sidebar refresh, and threading that through the caller would put the same
  // three lines in seven places.
  //
  // Items that cannot apply are left out rather than disabled. A direct message
  // has no members to add and cannot be left -- the server refuses -- and a
  // greyed row that never becomes available is a worse answer than no row.
  import * as api from "./api";
  import * as log from "./log";

  let {
    channel,
    groups,
    favorite,
    at,
    meId,
    onchanged,
    onaddmembers,
    onclose,
  }: {
    channel: api.ChannelSummary;
    /** This team's categories, for the move submenu. */
    groups: api.SidebarGroup[];
    favorite: boolean;
    at: { x: number; y: number };
    meId: string;
    /** Something on the server changed; the sidebar needs rebuilding. */
    onchanged: () => void;
    onaddmembers: () => void;
    onclose: () => void;
  } = $props();

  /** A conversation rather than a channel: no membership to manage. */
  const conversation = $derived(channel.channel_type === "D" || channel.channel_type === "G");
  const muted = $derived(channel.muted);
  const favorites = $derived(groups.find((group) => group.category_type === "favorites"));
  /** Where un-favouriting puts a channel back. */
  const plain = $derived(groups.find((group) => group.category_type === "channels"));
  /** Everywhere it could go, minus wherever it already is. */
  const targets = $derived(
    groups.filter(
      (group) =>
        group.category_type !== "direct_messages" &&
        !group.channels.some((held) => held.id === channel.id),
    ),
  );

  let submenu = $state(false);
  let busy = $state(false);
  let panel: HTMLDivElement | undefined = $state();
  /** Null until measured; the pointer position stands in for the first paint. */
  let placed: { x: number; y: number } | null = $state(null);

  // Kept on screen: a right-click near the bottom edge would otherwise open a
  // menu that runs off it.
  $effect(() => {
    if (!panel) return;
    const box = panel.getBoundingClientRect();
    const x = Math.min(at.x, window.innerWidth - box.width - 8);
    const y = Math.min(at.y, window.innerHeight - box.height - 8);
    placed = { x: Math.max(4, x), y: Math.max(4, y) };
  });

  /** Runs one item, then closes. Failures are logged and shown, never silent. */
  async function run(what: string, action: () => Promise<unknown>) {
    if (busy) return;
    busy = true;
    try {
      await action();
      log.info("channel.menu.done", { what, channel: channel.id });
      onchanged();
      onclose();
    } catch (thrown) {
      log.failure("channel.menu.failed", thrown, { what, channel: channel.id });
      failure = String(thrown);
    } finally {
      busy = false;
    }
  }

  let failure = $state("");

  async function copyLink() {
    const link = await api.channelLink(channel.id);
    await navigator.clipboard.writeText(link);
  }
</script>

<svelte:window
  onkeydown={(event) => {
    if (event.key === "Escape") onclose();
  }}
/>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="catcher"
  aria-hidden="true"
  onclick={onclose}
  oncontextmenu={(event) => {
    event.preventDefault();
    onclose();
  }}
></div>

<div
  class="menu"
  bind:this={panel}
  role="menu"
  aria-label={channel.display_name}
  style:left="{placed?.x ?? at.x}px"
  style:top="{placed?.y ?? at.y}px"
>
  <button type="button" role="menuitem" disabled={busy}
    onclick={() => run("unread", () => api.markChannelUnread(channel.id, meId))}
  >
    <span class="glyph">☰</span>Mark as Unread
  </button>

  {#if favorites && plain}
    <button type="button" role="menuitem" disabled={busy}
      onclick={() =>
        run("favorite", () =>
          api.moveChannel(
            channel.id,
            channel.team_id,
            favorite ? plain.id : favorites.id,
            meId,
          ),
        )}
    >
      <span class="glyph">☆</span>{favorite ? "Remove from Favorites" : "Favorite"}
    </button>
  {/if}

  <button type="button" role="menuitem" disabled={busy}
    onclick={() => run("mute", () => api.setChannelMuted(channel.id, meId, !muted))}
  >
    <span class="glyph">🔔</span>{muted ? "Unmute Channel" : "Mute Channel"}
  </button>

  {#if targets.length}
    <div class="rule"></div>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="nest"
      onmouseenter={() => (submenu = true)}
      onmouseleave={() => (submenu = false)}
    >
      <button
        type="button"
        role="menuitem"
        aria-haspopup="true"
        aria-expanded={submenu}
        disabled={busy}
        onclick={() => (submenu = !submenu)}
      >
        <span class="glyph">🗀</span>Move to…<span class="more">›</span>
      </button>
      {#if submenu}
        <div class="sub" role="menu" aria-label="Move to">
          {#each targets as target (target.id)}
            <button type="button" role="menuitem" disabled={busy}
              onclick={() =>
                run("move", () =>
                  api.moveChannel(channel.id, channel.team_id, target.id, meId),
                )}
            >
              {target.display_name}
            </button>
          {/each}
        </div>
      {/if}
    </div>
  {/if}

  <div class="rule"></div>
  <button type="button" role="menuitem" disabled={busy} onclick={() => run("link", copyLink)}>
    <span class="glyph">🔗</span>Copy Link
  </button>

  {#if !conversation}
    <button
      type="button"
      role="menuitem"
      disabled={busy}
      onclick={() => {
        onaddmembers();
        onclose();
      }}
    >
      <span class="glyph">👤</span>Add Members
    </button>

    <div class="rule"></div>
    <button type="button" role="menuitem" class="leave" disabled={busy}
      onclick={() => run("leave", () => api.leaveChannel(channel.id, meId))}
    >
      <span class="glyph">⇥</span>Leave Channel
    </button>
  {/if}

  {#if failure}<p class="failure">{failure}</p>{/if}
</div>

<style>
  .catcher {
    position: fixed;
    inset: 0;
    z-index: 80;
  }
  .menu {
    position: fixed;
    z-index: 81;
    min-width: 200px;
    padding: 4px;
    border: 1px solid var(--rule);
    border-radius: 7px;
    background: var(--surface);
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.35);
  }
  .menu button {
    display: flex;
    align-items: center;
    gap: 9px;
    width: 100%;
    text-align: left;
    font: inherit;
    font-size: 13px;
    padding: 6px 9px;
    border: 0;
    border-radius: 5px;
    background: none;
    color: var(--ink);
    cursor: pointer;
    white-space: nowrap;
  }
  .menu button:hover:not(:disabled) {
    background: var(--ground);
  }
  .menu button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .glyph {
    width: 16px;
    text-align: center;
    color: var(--ink-faint);
    font-size: 12px;
  }
  .leave {
    color: var(--flag);
  }
  .rule {
    height: 1px;
    margin: 4px 6px;
    background: var(--rule);
  }
  .nest {
    position: relative;
  }
  .more {
    flex: 1;
    text-align: right;
    color: var(--ink-faint);
  }
  .sub {
    position: absolute;
    left: calc(100% - 4px);
    top: -4px;
    min-width: 160px;
    max-height: 260px;
    overflow-y: auto;
    padding: 4px;
    border: 1px solid var(--rule);
    border-radius: 7px;
    background: var(--surface);
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.35);
  }
  .failure {
    margin: 4px 8px 2px;
    font-size: 11.5px;
    color: var(--flag);
    max-width: 220px;
    overflow-wrap: anywhere;
  }
</style>
