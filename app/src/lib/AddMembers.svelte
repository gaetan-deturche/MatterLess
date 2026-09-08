<script lang="ts">
  // Adds people to a channel.
  //
  // The same picker shape as starting a conversation, deliberately: the reader
  // is doing the same thing -- choosing people from the directory -- and the
  // only difference is where they end up. There is no group-size ceiling here,
  // so the count is a tally rather than a limit.
  //
  // Each person is added on their own call, because the server takes one member
  // at a time. A failure part-way therefore leaves the earlier ones added, which
  // is why the count of what succeeded is reported rather than a bare error.
  import Avatar from "./Avatar.svelte";
  import * as api from "./api";
  import * as log from "./log";

  let {
    channelId,
    channelName,
    onadded,
    onclose,
  }: {
    channelId: string;
    channelName: string;
    /** Somebody was added; the channel's membership has changed. */
    onadded: () => void;
    onclose: () => void;
  } = $props();

  let query = $state("");
  let found: api.Suggestion[] = $state([]);
  let chosen: api.Suggestion[] = $state([]);
  let adding = $state(false);
  let failure = $state("");
  let field: HTMLInputElement | undefined = $state();
  let asked = 0;

  $effect(() => {
    field?.focus();
  });

  $effect(() => {
    const wanted = query.trim();
    const mine = ++asked;
    const timer = setTimeout(() => {
      void api
        .suggest("user", wanted, undefined, 24)
        .then((people) => {
          if (mine !== asked) return;
          found = people;
        })
        .catch((error) => log.warn("members.search.failed", { error: String(error) }));
    }, 120);
    return () => clearTimeout(timer);
  });

  function add(person: api.Suggestion) {
    if (chosen.some((held) => held.id === person.id)) return;
    chosen = [...chosen, person];
    query = "";
    field?.focus();
  }

  function remove(id: string) {
    chosen = chosen.filter((held) => held.id !== id);
  }

  async function commit() {
    if (chosen.length === 0 || adding) return;
    adding = true;
    failure = "";
    let added = 0;
    try {
      for (const person of chosen) {
        await api.addChannelMember(channelId, person.id);
        added += 1;
      }
      log.info("channel.members.added", { channel: channelId, people: added });
      onadded();
      onclose();
    } catch (thrown) {
      failure =
        added > 0
          ? `Added ${added} of ${chosen.length}, then: ${String(thrown)}`
          : String(thrown);
      log.failure("channel.members.failed", thrown, { channel: channelId, added });
      if (added > 0) onadded();
    } finally {
      adding = false;
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

<div class="panel" role="dialog" aria-label="Add members">
  <header>
    <h2>Add to {channelName}</h2>
    <button type="button" class="close" onclick={onclose} aria-label="Close">×</button>
  </header>

  {#if chosen.length}
    <ul class="chosen">
      {#each chosen as person (person.id)}
        <li>
          <Avatar userId={person.id} name={person.label} version={person.avatar_at} size={16} />
          <span>{person.label}</span>
          <button type="button" onclick={() => remove(person.id)} aria-label="Remove">×</button>
        </li>
      {/each}
    </ul>
  {/if}

  <input
    bind:this={field}
    bind:value={query}
    type="text"
    placeholder="Search people…"
    aria-label="Search people"
    onkeydown={(event) => {
      if (event.key === "Backspace" && query === "" && chosen.length > 0) {
        event.preventDefault();
        chosen = chosen.slice(0, -1);
      } else if (event.key === "Enter" && found.length > 0) {
        event.preventDefault();
        add(found[0]!);
      }
    }}
  />

  {#if failure}<p class="failure">{failure}</p>{/if}

  <ul class="list">
    {#each found.filter((person) => !chosen.some((held) => held.id === person.id)) as person (person.id)}
      <li>
        <button type="button" class="row" onclick={() => add(person)}>
          <Avatar userId={person.id} name={person.label} version={person.avatar_at} size={20} />
          <span class="what">{person.label}</span>
          {#if person.detail}<span class="who">{person.detail}</span>{/if}
        </button>
      </li>
    {/each}
  </ul>

  <div class="actions">
    <button type="button" class="primary" disabled={chosen.length === 0 || adding} onclick={commit}>
      {#if adding}
        Adding…
      {:else if chosen.length > 1}
        Add {chosen.length} people
      {:else}
        Add
      {/if}
    </button>
    <button type="button" onclick={onclose}>Cancel</button>
  </div>
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
    width: 420px;
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
  .chosen {
    list-style: none;
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin: 0;
    padding: 0;
  }
  .chosen li {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 2px 4px 2px 3px;
    border: 1px solid var(--rule);
    border-radius: 12px;
    background: var(--ground);
    font-size: 12px;
  }
  .chosen button {
    font: inherit;
    font-size: 13px;
    line-height: 1;
    padding: 0 3px;
    border: 0;
    background: none;
    color: var(--ink-faint);
    cursor: pointer;
  }
  .chosen button:hover {
    color: var(--ink);
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
    gap: 8px;
    width: 100%;
    text-align: left;
    font: inherit;
    font-size: 13px;
    padding: 5px 6px;
    border: 0;
    border-radius: 5px;
    background: none;
    color: var(--ink);
    cursor: pointer;
  }
  .row:hover {
    background: var(--ground);
  }
  .what {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .who {
    font-size: 11.5px;
    color: var(--ink-faint);
  }
  .failure {
    margin: 0;
    font-size: 12px;
    color: var(--flag);
    overflow-wrap: anywhere;
  }
  .actions {
    display: flex;
    gap: 6px;
  }
  .actions button {
    font: inherit;
    font-size: 12.5px;
    padding: 5px 10px;
    border: 1px solid var(--rule);
    border-radius: 5px;
    background: var(--ground);
    color: var(--ink);
    cursor: pointer;
  }
  .actions button.primary {
    background: var(--signal);
    border-color: var(--signal);
    color: #fff;
  }
  .actions button:disabled {
    opacity: 0.55;
    cursor: default;
  }
</style>
