<script lang="ts">
  // Sends a message somewhere else, as a link plus whatever the reader wants to
  // say about it.
  //
  // A permalink rather than a copy of the text, which is what the official
  // client does and the honest thing besides: the message stays one message,
  // with one set of replies and one author, and the forward is a pointer to it
  // rather than a second version that can drift.
  //
  // The destination comes from the same `switcher` the quick switcher uses, so
  // channels and people rank the same way here as they do there.
  import * as api from "./api";
  import * as store from "./store.svelte";
  import * as log from "./log";
  import Avatar from "./Avatar.svelte";

  let {
    postId,
    onclose,
  }: {
    postId: string;
    onclose: (said?: string) => void;
  } = $props();

  let query = $state("");
  let choices: api.Suggestion[] = $state([]);
  let chosen: api.Suggestion | null = $state(null);
  let comment = $state("");
  let sending = $state(false);
  let failure = $state("");
  let field: HTMLInputElement | undefined = $state();

  $effect(() => {
    field?.focus();
  });

  $effect(() => {
    const wanted = query.trim();
    const timer = setTimeout(() => {
      void api
        .switcher(wanted)
        .then((found) => {
          if (query.trim() !== wanted) return;
          choices = found;
        })
        .catch((error) => log.warn("forward.search.failed", { error: String(error) }));
    }, 120);
    return () => clearTimeout(timer);
  });

  async function send() {
    if (!chosen || sending) return;
    sending = true;
    failure = "";
    try {
      const link = await api.postPermalink(postId);
      // A person is a channel only once the conversation exists.
      const meId = store.meta()?.meId ?? "";
      const channelId =
        chosen.kind === "user" ? await api.openDirectMessage(meId, chosen.id) : chosen.id;
      const said = comment.trim();
      await api.sendPost(channelId, said ? `${said}\n${link}` : link);
      log.info("message.forwarded", { post: postId, to: chosen.kind });
      onclose(`Forwarded to ${chosen.label}.`);
    } catch (thrown) {
      failure = String(thrown);
      log.failure("message.forward.failed", thrown, { post: postId });
    } finally {
      sending = false;
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
<div class="catcher" aria-hidden="true" onclick={() => onclose()}></div>

<div class="panel" role="dialog" aria-label="Forward this message">
  <h2>Forward</h2>

  {#if chosen}
    <button type="button" class="picked" onclick={() => (chosen = null)}>
      {chosen.label}
      <span class="change">change</span>
    </button>
  {:else}
    <input
      bind:this={field}
      bind:value={query}
      type="text"
      placeholder="Search channels and people…"
      aria-label="Where to forward this"
    />
    <ul class="choices">
      {#each choices as option (option.kind + option.id)}
        <li>
          <button type="button" onclick={() => (chosen = option)}>
            {#if option.kind === "user"}
              <Avatar
                userId={option.id}
                name={option.label}
                version={option.avatar_at}
                size={18}
              />
            {:else}
              <!-- The glyph the composer's own channel suggestions use: a
                   `Suggestion` is not a `ChannelSummary`, and private is worth
                   seeing before you forward into it. -->
              <span class="glyph">{option.channel_type === "P" ? "🔒" : "#"}</span>
            {/if}
            <span class="what">{option.label}</span>
            {#if option.detail}<span class="who">{option.detail}</span>{/if}
          </button>
        </li>
      {/each}
    </ul>
  {/if}

  <textarea
    bind:value={comment}
    rows="2"
    placeholder="Add a comment (optional)"
    aria-label="Comment"
  ></textarea>

  {#if failure}<p class="failure">{failure}</p>{/if}

  <div class="actions">
    <button type="button" class="primary" disabled={!chosen || sending} onclick={send}>
      {sending ? "Forwarding…" : "Forward"}
    </button>
    <button type="button" onclick={() => onclose()}>Cancel</button>
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
    width: 380px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 14px;
    border: 1px solid var(--rule);
    border-radius: 8px;
    background: var(--surface);
    box-shadow: 0 10px 30px rgb(0 0 0 / 0.3);
  }
  h2 {
    margin: 0;
    font-size: 14px;
  }
  input,
  textarea {
    font: inherit;
    font-size: 13px;
    padding: 6px 8px;
    border: 1px solid var(--rule);
    border-radius: 5px;
    background: var(--ground);
    color: var(--ink);
    resize: vertical;
  }
  input:focus,
  textarea:focus {
    outline: none;
    border-color: var(--signal);
  }
  .choices {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 180px;
    overflow-y: auto;
  }
  .choices button,
  .picked {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    font: inherit;
    font-size: 13px;
    text-align: left;
    padding: 5px 6px;
    border: 0;
    border-radius: 5px;
    background: transparent;
    color: var(--ink);
    cursor: pointer;
  }
  .choices button:hover {
    background: var(--ground);
  }
  .picked {
    border: 1px solid var(--rule);
    background: var(--ground);
    justify-content: space-between;
  }
  .change {
    font-size: 11.5px;
    color: var(--ink-faint);
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
  .glyph {
    display: inline-grid;
    place-items: center;
    width: 18px;
    font-size: 12px;
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
