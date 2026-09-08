<script lang="ts">
  // A list of messages pointing back at where they were said.
  //
  // Shared by search, saved messages and pinned messages. All three are the
  // same thing -- a post, where it came from, who said it, and enough of it to
  // recognise -- and three copies of this markup would be three places for that
  // to drift.
  import Nodes from "./Nodes.svelte";
  import type { SearchHit } from "./api";

  let {
    hits,
    onjump,
  }: {
    hits: SearchHit[];
    /** Go to this message: the channel, then the post inside it. */
    onjump: (channelId: string, postId: string) => void;
  } = $props();

  const when = (ms: number) =>
    new Date(ms).toLocaleString([], {
      day: "numeric",
      month: "short",
      hour: "2-digit",
      minute: "2-digit",
    });
</script>

<div class="results">
  {#each hits as hit (hit.post_id)}
    <button type="button" class="hit" onclick={() => onjump(hit.channel_id, hit.post_id)}>
      <span class="where">
        <strong>{hit.channel_label}</strong>
        <span class="who">{hit.author_name}</span>
        <time>{when(hit.create_at)}</time>
      </span>
      <span class="what"><Nodes nodes={hit.nodes} /></span>
    </button>
  {/each}
</div>

<style>
  .results {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-height: 0;
    overflow-y: auto;
    padding: 0 8px 10px;
  }
  .hit {
    display: flex;
    flex-direction: column;
    gap: 3px;
    width: 100%;
    text-align: left;
    font: inherit;
    padding: 8px 9px;
    border: 0;
    border-radius: 6px;
    background: none;
    color: var(--ink);
    cursor: pointer;
  }
  .hit:hover {
    background: var(--ground);
  }
  .where {
    display: flex;
    align-items: baseline;
    gap: 7px;
    font-size: 11.5px;
    color: var(--ink-faint);
  }
  .where strong {
    font-size: 12.5px;
    color: var(--ink);
    font-weight: 600;
  }
  .what {
    font-size: 13px;
    /* Three lines of context: enough to recognise the message, not enough to
       turn the list into a second message pane. */
    display: -webkit-box;
    line-clamp: 3;
    -webkit-line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
</style>
