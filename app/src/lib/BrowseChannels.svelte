<script lang="ts">
  // The public channels of a team, to join or to open.
  //
  // From the server rather than the local table, and that is the whole point:
  // the quick switcher searches what the reader is already in, and this is for
  // everything they are not. Membership is the local half, which is what marks
  // the ones already joined.
  //
  // Joined channels are shown rather than filtered out. A browse list that hides
  // them leaves the reader wondering whether the channel exists at all, which is
  // a worse answer than "you are already here".
  import * as api from "./api";
  import * as store from "./store.svelte";
  import * as log from "./log";

  let {
    teamId,
    teamName,
    onopen,
    onclose,
  }: {
    teamId: string;
    teamName: string;
    /** Go to a channel, once it is joined. */
    onopen: (channelId: string) => void;
    onclose: () => void;
  } = $props();

  let query = $state("");
  let found: api.Joinable[] = $state([]);
  let loading = $state(false);
  let failure = $state("");
  let joining = $state("");
  let field: HTMLInputElement | undefined = $state();
  /** Only the newest request may write the list. */
  let asked = 0;

  $effect(() => {
    field?.focus();
  });

  $effect(() => {
    const wanted = query.trim();
    const team = teamId;
    const mine = ++asked;
    loading = true;
    const timer = setTimeout(() => {
      void api
        .browseChannels(team, wanted)
        .then((channels) => {
          if (mine !== asked) return;
          found = channels;
          failure = "";
        })
        .catch((error) => {
          if (mine !== asked) return;
          failure = String(error);
          log.failure("channels.browse.failed", error, { team });
        })
        .finally(() => {
          if (mine === asked) loading = false;
        });
    }, 140);
    return () => clearTimeout(timer);
  });

  async function join(channel: api.Joinable) {
    if (joining) return;
    const meId = store.meta()?.meId;
    if (!meId) return;
    joining = channel.id;
    try {
      if (!channel.joined) {
        await api.joinChannel(channel.id, meId);
        log.info("channel.joined", { channel: channel.id });
        // The sidebar is built from membership, which has just changed.
        const meta = store.meta();
        if (meta) {
          const channels = await api.refreshMembership(meta.meId, meta.displayMode);
          store.setChannelList(channels);
        }
      }
      onopen(channel.id);
    } catch (thrown) {
      failure = String(thrown);
      log.failure("channel.join.failed", thrown, { channel: channel.id });
    } finally {
      joining = "";
    }
  }
</script>

<svelte:window
  onkeydown={(event) => {
    if (event.key === "Escape") onclose();
  }}
/>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="catcher" aria-hidden="true" onclick={onclose}></div>

<div class="panel" role="dialog" aria-label="Browse channels">
  <header>
    <h2>Channels in {teamName}</h2>
    <button type="button" class="close" onclick={onclose} aria-label="Close">×</button>
  </header>

  <input
    bind:this={field}
    bind:value={query}
    type="text"
    placeholder="Search channels…"
    aria-label="Search channels"
  />

  {#if failure}
    <p class="failure">{failure}</p>
  {:else if loading && found.length === 0}
    <p class="hint">Loading…</p>
  {:else if !loading && found.length === 0}
    <p class="hint">No public channel matches that.</p>
  {/if}

  <ul class="list">
    {#each found as channel (channel.id)}
      <li>
        <button
          type="button"
          class="row"
          disabled={joining === channel.id}
          onclick={() => void join(channel)}
        >
          <span class="what">
            <span class="name">
              {channel.channel_type === "P" ? "🔒" : "#"}
              {channel.display_name || channel.name}
            </span>
            {#if channel.purpose}<span class="purpose">{channel.purpose}</span>{/if}
          </span>
          <span class="verb">
            {#if joining === channel.id}
              …
            {:else if channel.joined}
              open
            {:else}
              join
            {/if}
          </span>
        </button>
      </li>
    {/each}
  </ul>
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
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    box-sizing: border-box;
    width: 460px;
    max-height: 70vh;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 14px;
    border: 1px solid var(--rule);
    border-radius: 8px;
    background: var(--surface);
    box-shadow: 0 10px 30px rgb(0 0 0 / 0.3);
  }
  header {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  h2 {
    flex: 1;
    margin: 0;
    font-size: 14px;
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
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    min-height: 0;
    overflow-y: auto;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    text-align: left;
    font: inherit;
    padding: 7px 8px;
    border: 0;
    border-radius: 6px;
    background: none;
    color: var(--ink);
    cursor: pointer;
  }
  .row:hover:not(:disabled) {
    background: var(--ground);
  }
  .row:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .what {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .name {
    font-size: 13.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .purpose {
    font-size: 11.5px;
    color: var(--ink-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .verb {
    font-size: 11.5px;
    color: var(--signal);
  }
  .hint,
  .failure {
    margin: 0;
    font-size: 12px;
    color: var(--ink-faint);
  }
  .failure {
    color: var(--flag);
    overflow-wrap: anywhere;
  }
</style>
