<script lang="ts">
  // Who somebody is, and the one thing you usually want next: a message to them.
  //
  // Opened from a mention, an avatar or an author's name. Positioned where the
  // click was rather than centred, because it is about *that* name in *that*
  // message -- a modal in the middle of the window loses which one you asked
  // about.
  import Avatar from "./Avatar.svelte";
  import * as api from "./api";
  import * as store from "./store.svelte";
  import * as log from "./log";

  let {
    username,
    at,
    onmessage,
    onclose,
  }: {
    username: string;
    /** Where the click was, in viewport coordinates. */
    at: { x: number; y: number };
    /** Open a conversation with this person. */
    onmessage: (channelId: string) => void;
    onclose: () => void;
  } = $props();

  let card: api.Profile | undefined = $state();
  let failure = $state("");
  let opening = $state(false);

  const CARD = { width: 268, height: 190 };
  /** Clamped into the window: a mention near the right edge or the last line of
   *  the stream would otherwise open a card half off screen. */
  const placed = $derived({
    left: Math.min(Math.max(8, at.x), Math.max(8, window.innerWidth - CARD.width - 8)),
    top: Math.min(Math.max(8, at.y + 12), Math.max(8, window.innerHeight - CARD.height - 8)),
  });

  $effect(() => {
    const meta = store.meta();
    if (!meta) return;
    const wanted = username;
    void (async () => {
      try {
        const found = await api.profile(wanted, meta.meId, meta.displayMode);
        if (wanted !== username) return;
        card = found;
        // The card shows a dot, and this is a person the store may not know.
        store.setStatus(found.user_id, found.status);
      } catch (thrown) {
        failure = String(thrown);
        log.failure("profile.failed", thrown, {});
      }
    })();
  });

  async function message() {
    if (!card || opening) return;
    const meId = store.meta()?.meId;
    if (!meId) return;
    opening = true;
    try {
      const channelId = await api.openDirectMessage(meId, card.user_id);
      log.info("profile.message", { user: card.user_id });
      // The conversation may not be in the local membership yet.
      const meta = store.meta();
      if (meta) {
        void api
          .refreshMembership(meta.meId, meta.displayMode)
          .then((channels) => store.setChannelList(channels))
          .catch((error) => log.warn("profile.membership.failed", { error: String(error) }));
      }
      onmessage(channelId);
    } catch (thrown) {
      failure = String(thrown);
      log.failure("profile.message.failed", thrown, {});
    } finally {
      opening = false;
    }
  }

  const label = (status: string) =>
    status === "dnd"
      ? "Do not disturb"
      : status === "ooo"
        ? "Out of office"
        : status || "offline";
</script>

<svelte:window
  onkeydown={(event) => {
    if (event.key === "Escape") onclose();
  }}
/>

<!-- A backdrop whose only job is to catch the next click. Escape closes the
     card too, which is the keyboard path; this element is not a control. -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="catcher" aria-hidden="true" onclick={onclose}></div>

<!-- No click handler of its own: the catcher is a *sibling*, so a click in
     here never reaches it and there is nothing to stop propagating. -->
<div
  class="card"
  role="dialog"
  tabindex="-1"
  aria-label="Profile"
  style:left="{placed.left}px"
  style:top="{placed.top}px"
>
  {#if failure}
    <p class="failure">{failure}</p>
  {:else if !card}
    <p class="waiting">…</p>
  {:else}
    <div class="head">
      <Avatar
        userId={card.user_id}
        name={card.username}
        version={card.avatar_at}
        size={44}
        presence
      />
      <div class="names">
        <strong>{card.display_name}</strong>
        <span class="handle">@{card.username}</span>
        {#if card.full_name && card.full_name !== card.display_name}
          <span class="real">{card.full_name}</span>
        {/if}
      </div>
    </div>
    <p class="status">{label(card.status)}</p>
    {#if card.email}<p class="email">{card.email}</p>{/if}
    <div class="actions">
      {#if card.is_me}
        <button type="button" onclick={message} disabled={opening}>
          {opening ? "Opening…" : "Notes to self"}
        </button>
      {:else}
        <button type="button" class="primary" onclick={message} disabled={opening}>
          {opening ? "Opening…" : "Message"}
        </button>
      {/if}
      <button type="button" onclick={onclose}>Close</button>
    </div>
  {/if}
</div>

<style>
  .catcher {
    position: fixed;
    inset: 0;
    z-index: 70;
  }
  .card {
    position: fixed;
    z-index: 71;
    width: 268px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    border: 1px solid var(--rule);
    border-radius: 8px;
    background: var(--surface);
    box-shadow: 0 10px 30px rgb(0 0 0 / 0.3);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .names {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .names strong {
    font-size: 14px;
  }
  .handle,
  .real {
    font-size: 12px;
    color: var(--ink-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .status,
  .email,
  .waiting,
  .failure {
    margin: 0;
    font-size: 12px;
    color: var(--ink-soft);
    overflow-wrap: anywhere;
  }
  .status {
    text-transform: capitalize;
  }
  .failure {
    color: var(--flag);
  }
  .actions {
    display: flex;
    gap: 6px;
    margin-top: 2px;
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
