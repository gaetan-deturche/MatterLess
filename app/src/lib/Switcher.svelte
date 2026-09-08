<script lang="ts">
  // Go anywhere with the keyboard: channels and people in one ranked list.
  //
  // Both halves are scored by the same function in Rust, which is what makes a
  // single list out of two sources meaningful. Picking a person opens the direct
  // message with them -- creating it if there is not one yet, which is how a
  // conversation starts here at all.
  import Avatar from "./Avatar.svelte";
  import * as api from "./api";
  import * as store from "./store.svelte";
  import * as log from "./log";

  let {
    onpick,
    onclose,
  }: {
    /** Called with a channel id, whether it was picked or just created. */
    onpick: (channelId: string) => void;
    onclose: () => void;
  } = $props();

  let query = $state("");
  let rows: api.Suggestion[] = $state([]);
  let chosen = $state(0);
  let opening = $state(false);
  let failure = $state("");

  /** Only the newest answer may write the list; see the composer's completions
   *  for why that is not optional. */
  let asked = 0;

  // Opens on whatever was there last time, which for an empty query is the
  // channels with something happening in them.
  $effect(() => {
    void find(query);
  });

  async function find(text: string) {
    const mine = ++asked;
    try {
      const found = await api.switcher(text);
      if (mine !== asked) return;
      rows = found;
      chosen = 0;

      // Presence for anyone here we do not already know about. The switcher can
      // list people who have never appeared in a channel this client has read,
      // and a *missing* dot reads as offline -- which would be a confident lie
      // in the one place presence is being used to make a decision.
      const unknown = found
        .filter((row) => row.kind === "user" && store.statusOf(row.id) === undefined)
        .map((row) => row.id);
      if (unknown.length > 0) {
        const statuses = await api.statuses(unknown);
        if (mine !== asked) return;
        store.learnStatuses(statuses);
      }
    } catch (thrown) {
      if (mine !== asked) return;
      failure = String(thrown);
      log.failure("switcher.failed", thrown, {});
    }
  }

  async function pick(row: api.Suggestion) {
    if (opening) return;
    if (row.kind === "channel") {
      onpick(row.id);
      return;
    }
    // A person: open the conversation, which the server creates on demand and
    // returns as-is when it already exists.
    const meId = store.meta()?.meId;
    if (!meId) return;
    opening = true;
    try {
      const channelId = await api.openDirectMessage(meId, row.id);
      log.info("switcher.direct", { user: row.id, channel: channelId });
      // The sidebar is built from membership, and a conversation that has just
      // been created is not in the local copy of it yet.
      const meta = store.meta();
      if (meta) {
        void api
          .refreshMembership(meta.meId, meta.displayMode)
          .then((channels) => store.setChannelList(channels))
          .catch((error) => log.warn("switcher.membership.failed", { error: String(error) }));
      }
      onpick(channelId);
    } catch (thrown) {
      failure = String(thrown);
      log.failure("switcher.direct.failed", thrown, { user: row.id });
    } finally {
      opening = false;
    }
  }

  function onKeyDown(event: KeyboardEvent) {
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        chosen = rows.length === 0 ? 0 : (chosen + 1) % rows.length;
        break;
      case "ArrowUp":
        event.preventDefault();
        chosen = rows.length === 0 ? 0 : (chosen - 1 + rows.length) % rows.length;
        break;
      case "Enter": {
        event.preventDefault();
        const row = rows[chosen];
        if (row) void pick(row);
        break;
      }
      case "Escape":
        event.preventDefault();
        onclose();
        break;
    }
  }
</script>

<!-- A backdrop that closes on a click outside, and a dialog that does not
     bubble its own clicks into it. -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="backdrop" onclick={onclose}>
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div
    class="panel"
    role="dialog"
    tabindex="-1"
    aria-modal="true"
    aria-label="Go to channel or person"
    onclick={(event) => event.stopPropagation()}
  >
    <!-- svelte-ignore a11y_autofocus -->
    <input
      bind:value={query}
      onkeydown={onKeyDown}
      placeholder="Go to a channel or a person…"
      autofocus
      spellcheck="false"
    />
    {#if rows.length === 0}
      <p class="empty">{query ? "Nothing matches." : "Start typing."}</p>
    {:else}
      <ul role="listbox" aria-label="Results">
        {#each rows as row, index (row.kind + row.id)}
          <li>
            <button
              type="button"
              role="option"
              aria-selected={index === chosen}
              class:chosen={index === chosen}
              onmouseenter={() => (chosen = index)}
              onclick={() => void pick(row)}
            >
              {#if row.kind === "user"}
                <!-- Presence here decides the choice being made: whether to
                     message somebody now is most of why you opened this. -->
                <Avatar
                  userId={row.id}
                  name={row.label}
                  version={row.avatar_at}
                  size={22}
                  presence
                />
              {:else}
                <span class="glyph">{row.channel_type === "P" ? "🔒" : "#"}</span>
              {/if}
              <span class="what">{row.label}</span>
              {#if row.detail}<span class="who">{row.detail}</span>{/if}
              {#if row.kind === "user"}<span class="tag">direct message</span>{/if}
            </button>
          </li>
        {/each}
      </ul>
    {/if}
    {#if failure}<p class="failure">{failure}</p>{/if}
    <p class="keys">↑↓ to move · Enter to open · Esc to close</p>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 60;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 12vh;
    background: rgb(0 0 0 / 0.4);
  }
  .panel {
    width: min(560px, 92vw);
    display: flex;
    flex-direction: column;
    border: 1px solid var(--rule);
    border-radius: 8px;
    background: var(--surface);
    box-shadow: 0 12px 40px rgb(0 0 0 / 0.35);
    overflow: hidden;
  }
  input {
    font: inherit;
    font-size: 15px;
    padding: 12px 14px;
    border: 0;
    border-bottom: 1px solid var(--rule);
    background: var(--surface);
    color: var(--ink);
  }
  input:focus-visible {
    outline: 0;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 4px;
    max-height: 46vh;
    overflow-y: auto;
  }
  button {
    display: flex;
    align-items: center;
    gap: 9px;
    width: 100%;
    text-align: left;
    font: inherit;
    font-size: 13.5px;
    padding: 6px 8px;
    border: 0;
    border-radius: 5px;
    background: none;
    color: var(--ink);
    cursor: pointer;
  }
  button.chosen {
    background: var(--signal-soft);
  }
  .glyph {
    width: 22px;
    height: 22px;
    display: grid;
    place-items: center;
    color: var(--ink-faint);
  }
  .what {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .who {
    color: var(--ink-faint);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tag {
    margin-left: auto;
    font-size: 10.5px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--ink-faint);
  }
  .empty,
  .keys,
  .failure {
    margin: 0;
    padding: 10px 14px;
    font-size: 12px;
    color: var(--ink-faint);
  }
  .keys {
    border-top: 1px solid var(--rule-soft);
  }
  .failure {
    color: var(--flag);
  }
</style>
