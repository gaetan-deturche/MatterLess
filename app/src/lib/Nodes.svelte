<script lang="ts">
  import * as store from "./store.svelte";
  import * as media from "./media";
  // Renders a node tree as components with text interpolation. Never {@html}:
  // building the tree in Rust and interpolating here makes injection impossible
  // by construction, and avoids innerHTML reflow cost.
  import Nodes from "./Nodes.svelte";
  import type { Node } from "./api";

  let { nodes }: { nodes: Node[] } = $props();
</script>

{#each nodes as node}
  {#if node.t === "text"}{node.value}
  {:else if node.t === "paragraph"}<p class="para"><Nodes nodes={node.children} /></p>
  {:else if node.t === "emphasis"}<em><Nodes nodes={node.children} /></em>
  {:else if node.t === "strong"}<strong><Nodes nodes={node.children} /></strong>
  {:else if node.t === "strike"}<s><Nodes nodes={node.children} /></s>
  {:else if node.t === "inline_code"}<code>{node.value}</code>
  {:else if node.t === "code_block"}
    <pre class="fence" data-language={node.language ?? ""}><code>{node.value}</code></pre>
  {:else if node.t === "link"}
    <a href={node.href} target="_blank" rel="noreferrer noopener"><Nodes nodes={node.children} /></a>
  {:else if node.t === "image"}
    <!-- Drawn in a box of a fixed height, not at its own size.
         An image posted in a message has no dimensions until it loads, and the
         virtualiser reserves a row's height *before* it mounts -- so letting it
         size itself would make every estimate below it wrong, which is the
         exact fault that was chased out of previews and attachments. The box is
         the reserved height, and `object-fit: contain` fits the picture to it
         whatever shape it turns out to be.
         Fetched straight from wherever it was posted: this server runs no image
         proxy, so drawing one is a request from this machine to that host. -->
    <img
      class="inline-image"
      src={node.url}
      alt={node.alt}
      title={node.alt}
      loading="lazy"
      referrerpolicy="no-referrer"
    />
  <!-- Both are buttons rather than spans: they *do* something, and a screen
       reader should be told so. -->
  <!-- `@all`, `@here` and `@channel` address the room: highlighted, but there
       is no profile behind them. -->
  {:else if node.t === "user_mention" && node.everyone}<span class="mention everyone"
      >@{node.username}</span
    >
  {:else if node.t === "user_mention"}<button
      type="button"
      class="mention"
      title="Show profile"
      onclick={(event) =>
        store.openProfile(node.username, {
          x: event.clientX,
          y: event.clientY,
        })}>@{node.username}</button
    >
  {:else if node.t === "channel_link"}<button
      type="button"
      class="channel-link"
      title="Go to channel"
      onclick={() => store.followChannel(node.name)}>~{node.name}</button
    >
  {:else if node.t === "emoji"}
    {@const customId = store.emojiIds()[node.name]}
    {#if node.unicode}
      <span class="emoji" title={`:${node.name}:`}>{node.unicode}</span>
    {:else if customId}
      <!-- Custom: an image behind the session token, through the `mmedia`
           scheme. -->
      <img
        class="emoji custom"
        src={media.emoji(customId)}
        alt={`:${node.name}:`}
        title={`:${node.name}:`}
      />
    {:else}
      <!-- Not resolved yet, or not an emoji at all: the name is readable, which
           beats a blank box. -->
      <span class="emoji" title={node.name}>:{node.name}:</span>
    {/if}
  {:else if node.t === "inline_math"}<code class="math">{node.value}</code>
  {:else if node.t === "heading"}
    <p class="heading" data-level={node.level}><Nodes nodes={node.children} /></p>
  {:else if node.t === "blockquote"}<blockquote><Nodes nodes={node.children} /></blockquote>
  {:else if node.t === "list"}
    {#if node.ordered}
      <ol>{#each node.items as item}<li><Nodes nodes={item} /></li>{/each}</ol>
    {:else}
      <ul>{#each node.items as item}<li><Nodes nodes={item} /></li>{/each}</ul>
    {/if}
  {:else if node.t === "table"}
    <div class="table-scroll">
      <table>
        <thead><tr>{#each node.head as cell}<th><Nodes nodes={cell} /></th>{/each}</tr></thead>
        <tbody>
          {#each node.rows as row}
            <tr>{#each row as cell}<td><Nodes nodes={cell} /></td>{/each}</tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else if node.t === "rule"}<hr />
  <!-- A single newline is a line break here, not whitespace: Mattermost's own
       renderer runs with `breaks: true`, so a message typed on two lines shows
       on two lines. Drawn as a break rather than rewritten in the parser, which
       stays a faithful markdown parse. -->
  {:else if node.t === "soft_break"}<br />
  {:else if node.t === "hard_break"}<br />
  {/if}
{/each}

<style>
  .inline-image {
    display: block;
    max-width: 100%;
    /* The height the layout reserved for it. See `IMAGE_LINE` in `virtual.ts`;
       the two have to agree or the row jumps when the picture arrives. */
    height: 180px;
    width: auto;
    max-height: 180px;
    object-fit: contain;
    object-position: left;
    margin: 4px 0 2px;
    border-radius: 5px;
    background: var(--ground);
  }
  .para { margin: 0 0 0.35em; }
  .para:last-child { margin-bottom: 0; }
  code {
    font-family: var(--mono);
    font-size: 0.9em;
    background: var(--surface-2);
    padding: 0.08em 0.3em;
    border-radius: 3px;
  }
  .fence {
    font-family: var(--mono);
    font-size: 12.5px;
    background: var(--surface-2);
    border: 1px solid var(--rule);
    border-radius: 4px;
    padding: 10px 12px;
    margin: 6px 0;
    overflow-x: auto;
  }
  .fence code { background: none; padding: 0; }
  .mention, .channel-link {
    /* A button that has to sit inside a sentence: no chrome of its own, and
       the text metrics of the line it is in. */
    display: inline;
    font: inherit;
    font-weight: 500;
    border: 0;
    color: var(--signal);
    background: var(--signal-soft);
    padding: 0 3px;
    border-radius: 3px;
    cursor: pointer;
  }
  .mention:hover, .channel-link:hover {
    text-decoration: underline;
  }
  .mention.everyone {
    cursor: default;
  }
  .mention.everyone:hover {
    text-decoration: none;
  }
  .emoji { color: var(--ink-soft); }
  .math { font-style: italic; }
  .heading { font-weight: 600; margin: 0.2em 0; }
  blockquote {
    margin: 4px 0;
    padding-left: 10px;
    border-left: 3px solid var(--rule);
    color: var(--ink-soft);
  }
  ul, ol { margin: 4px 0; padding-left: 22px; }
  .table-scroll { overflow-x: auto; }
  table { border-collapse: collapse; font-size: 13px; }
  th, td { border: 1px solid var(--rule); padding: 4px 8px; text-align: left; }
  a { color: var(--signal); }
  .emoji { font-style: normal; }
  .emoji.custom {
    /* Sized to the line rather than to the image: a custom emoji is typed
       inline with words and has to sit on their baseline. */
    height: 1.35em;
    width: auto;
    vertical-align: -0.25em;
  }
</style>
