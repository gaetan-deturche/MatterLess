<script lang="ts">
  // Link and permalink previews under a message.
  //
  // Both shapes come resolved from Rust: a page preview knows its title and the
  // box its image draws in, and a permalink preview arrives with the quoted
  // message already parsed. This only lays them out.
  import Nodes from "./Nodes.svelte";
  import { PREVIEW_IMAGE_WIDTH } from "./virtual";
  import * as api from "./api";
  import * as store from "./store.svelte";

  let { previews }: { previews: api.Preview[] } = $props();

  /** Images come straight from the linked site: this server has no image proxy
   *  (`HasImageProxy: false`), so a preview image is a request from this
   *  machine to that host. The reader can turn them off; the card without one
   *  still says what the page is. */
  const showImages = $derived(store.previewImages());

  const when = (ms: number) =>
    new Date(ms).toLocaleString([], {
      day: "numeric",
      month: "short",
      hour: "2-digit",
      minute: "2-digit",
    });
</script>

{#each previews as preview, index (index)}
  {#if preview.kind === "page"}
    <a class="card" href={preview.url} target="_blank" rel="noreferrer noopener">
      {#if preview.image && showImages}
        <!-- Width plus aspect ratio, both from the *declared* size, so the box
             is fixed before the file arrives and cannot change when it does.
             Without this the card was laid out from the OpenGraph dimensions
             and then relaid out to the real ones -- measured, a row flipping
             between 239.9px and 202.0px on every remount, which is a height the
             virtualiser can never settle on. -->
        <img
          class="shot"
          src={preview.image.url}
          alt=""
          width={preview.image.width}
          height={preview.image.height}
          style:width="{Math.min(PREVIEW_IMAGE_WIDTH, preview.image.width)}px"
          style:aspect-ratio={preview.image.height > 0
            ? `${preview.image.width} / ${preview.image.height}`
            : null}
          loading="lazy"
          referrerpolicy="no-referrer"
        />
      {/if}
      <span class="text">
        {#if preview.site_name}<span class="site">{preview.site_name}</span>{/if}
        {#if preview.title}<strong>{preview.title}</strong>{/if}
        {#if preview.description}<span class="blurb">{preview.description}</span>{/if}
      </span>
    </a>
  {:else}
    <!-- A quoted message, not a link: it opens where it was said. -->
    <button
      type="button"
      class="quoted"
      title="Go to the message"
      onclick={() => store.followPermalink(preview.channel_id, preview.post_id)}
    >
      <span class="said">
        <strong>{preview.author_name}</strong>
        {#if preview.channel_label}<span class="where">{preview.channel_label}</span>{/if}
        <time>{when(preview.create_at)}</time>
      </span>
      <span class="body"><Nodes nodes={preview.nodes} /></span>
    </button>
  {/if}
{/each}

<style>
  .card,
  .quoted {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    width: fit-content;
    max-width: 520px;
    margin: 4px 0 2px;
    padding: 8px 10px;
    /* A left bar rather than a full box: it belongs to the message above it. */
    border: 0;
    border-left: 3px solid var(--rule);
    border-radius: 0 5px 5px 0;
    background: var(--ground);
    color: var(--ink);
    text-align: left;
    font: inherit;
    text-decoration: none;
    cursor: pointer;
  }
  .card:hover,
  .quoted:hover {
    border-left-color: var(--signal);
  }
  .quoted {
    flex-direction: column;
    gap: 3px;
  }
  .shot {
    flex: none;
    /* The width is set inline from the declared size; this only stops it
       overflowing a narrow pane. */
    max-width: 100%;
    height: auto;
    border-radius: 4px;
    object-fit: cover;
  }
  .text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .site {
    font-size: 11px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--ink-faint);
  }
  .text strong {
    font-size: 13.5px;
    color: var(--signal);
  }
  .blurb {
    font-size: 12.5px;
    color: var(--ink-soft);
    /* Two lines: enough to know what the page is, not a second message. */
    display: -webkit-box;
    line-clamp: 2;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .said {
    display: flex;
    align-items: baseline;
    gap: 7px;
    font-size: 11.5px;
    color: var(--ink-faint);
  }
  .said strong {
    font-size: 12.5px;
    color: var(--ink);
  }
  .body {
    font-size: 13px;
    display: -webkit-box;
    line-clamp: 3;
    -webkit-line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
</style>
