<script lang="ts">
  // Attachments on a post: pictures inline, everything else as a card.
  //
  // The box every image is drawn in comes from Rust, already capped and
  // aspect-preserved. That matters more here than it looks: the virtualiser
  // learns a height per row kind and reserves it before the row exists, so an
  // image whose size only became known when its bytes arrived would make every
  // estimate wrong and shift the scroll under the reader.
  import * as api from "./api";
  import * as media from "./media";
  import * as log from "./log";

  let { files, compact = false }: { files: api.FileRef[]; compact?: boolean } = $props();

  /** More than one picture, so they are drawn as a compact row of thumbnails
   *  at their own size rather than one stretched image each. */
  /** Several things drawn at their own size share the room rather than each
   *  taking it. A player counts the same as a picture: the rule is about what
   *  is drawn inline, not about what kind of file it is. */
  const gallery = $derived(files.filter((file) => file.image || file.video).length > 1);

  /** Which file is open full-size, if any. */
  let opened: api.FileRef | null = $state(null);
  /** Where the last save landed, so the click has an answer. */
  let saved = $state("");
  let saving = $state("");

  /** 1.4 MB, 812 kB, 240 B -- the card's only number, so it is the readable
   *  one rather than the exact one. */
  function readableSize(bytes: number): string {
    if (bytes < 1000) return `${bytes} B`;
    if (bytes < 1000 * 1000) return `${Math.round(bytes / 1000)} kB`;
    return `${(bytes / (1000 * 1000)).toFixed(bytes < 10 * 1000 * 1000 ? 1 : 0)} MB`;
  }

  /** The mini preview the post already carries, as a CSS background.
   *
   *  Shown under the real image rather than instead of it: it is a ~1 KB JPEG
   *  that costs no request, so the box is never empty while the bytes arrive. */
  const placeholder = (file: api.FileRef) =>
    file.mini_preview ? `url("data:image/jpeg;base64,${file.mini_preview}")` : "none";

  /** Files whose thumbnail turned out not to exist, so the original is used.
   *
   *  The plan's thumbnail flag is a good guess rather than a promise -- see
   *  `FileRef::thumbnail` -- and this is what makes a wrong guess cost one 404
   *  instead of a broken image. */
  let whole: string[] = $state([]);

  function fellBack(file: api.FileRef) {
    if (!whole.includes(file.id)) {
      whole = [...whole, file.id];
      log.info("attachment.thumbnail.missing", { file: file.id });
    }
  }

  /** The rendition the plan chose, unless it turned out not to exist. */
  function source(file: api.FileRef): string {
    if (whole.includes(file.id)) return media.file(file.id);
    if (file.variant === "thumb") return media.thumb(file.id);
    if (file.variant === "preview") return media.preview(file.id);
    return media.file(file.id);
  }

  /** The full-size look, which is a bigger question than the inline one: the
   *  preview is capped at 1920 wide and is the right answer for a photograph,
   *  the original for anything the server does not re-encode. */
  const fullSource = (file: api.FileRef) =>
    file.variant === "original" || whole.includes(file.id)
      ? media.file(file.id)
      : media.preview(file.id);

  async function save(file: api.FileRef) {
    saving = file.id;
    try {
      const path = await api.saveAttachment(file.id, file.name);
      saved = path;
      log.info("attachment.saved", { file: file.id, bytes: file.size });
    } catch (thrown) {
      saved = "";
      log.failure("attachment.save.failed", thrown, { file: file.id });
    } finally {
      saving = "";
    }
  }

  /** Everything on this post the overlay can show: pictures and videos, in the
   *  order they were attached. A file card is not among them -- there is
   *  nothing to magnify -- so stepping never lands on one. */
  const viewable = $derived(files.filter((file) => file.image || file.video));
  const at = $derived.by(() => {
    const held = opened;
    return held ? viewable.findIndex((file) => file.id === held.id) : -1;
  });

  /** Steps the overlay, wrapping: with three attachments the step after the
   *  last is the first, which is what a reader flicking through expects. */
  function step(by: number) {
    if (at < 0 || viewable.length < 2) return;
    const next = (at + by + viewable.length) % viewable.length;
    opened = viewable[next] ?? opened;
  }

  function onKeyDown(event: KeyboardEvent) {
    if (!opened) return;
    if (event.key === "Escape") {
      opened = null;
    } else if (event.key === "ArrowLeft") {
      event.preventDefault();
      step(-1);
    } else if (event.key === "ArrowRight") {
      event.preventDefault();
      step(1);
    }
  }
</script>

<svelte:window onkeydown={onKeyDown} />

<div class="attachments" class:compact class:gallery>
  {#each files as file (file.id)}
    {#if file.image}
      <!-- Width plus aspect ratio rather than width and height: in a narrow
           pane `max-width` then shrinks the box proportionally instead of
           squashing the picture inside it. -->
      <button
        type="button"
        class="shot"
        style:width={`${file.box_width || 320}px`}
        style:aspect-ratio={`${file.box_width || 320} / ${file.box_height || 180}`}
        style:background-image={placeholder(file)}
        title={`${file.name} · ${file.width}×${file.height}`}
        onclick={() => (opened = file)}
      >
        <img
          src={source(file)}
          alt={file.name}
          loading="lazy"
          onerror={() => fellBack(file)}
        />
      </button>
    {:else if file.video && gallery}
      <!-- Sharing the room with other attachments: a poster, not a player.
           Controls at 120px are unusable, and a row of players would each hold
           a decoder and fetch metadata for a thumbnail nobody has asked to
           watch. Clicking opens the one at full size. -->
      <button
        type="button"
        class="shot poster"
        style:width={`${file.box_width || 120}px`}
        style:aspect-ratio={`${file.box_width || 16} / ${file.box_height || 9}`}
        style:background-image={placeholder(file)}
        title={file.name}
        onclick={() => (opened = file)}
      >
        <!-- The first frame, decoded from the file itself. The server has no
             thumbnail to offer for a video -- `/files/{id}/thumbnail` answers
             400 for anything that is not an image -- and `#t=0.1` is what makes
             the element decode a frame rather than show nothing: it seeks a
             tenth of a second in, which reaches the file over the same byte
             ranges the player uses. Not interactive; the button takes the
             click. -->
        <!-- svelte-ignore a11y_media_has_caption -->
        <video src={`${media.file(file.id)}#t=0.1`} preload="metadata" muted playsinline
        ></video>
        <span class="play" aria-hidden="true">▶</span>
      </button>
    {:else if file.video}
      <!-- The only attachment on the post, so it gets the room and plays in
           place. `preload="metadata"` rather than `auto`: enough to size the
           player and fill the scrub bar, without fetching the file until it is
           asked for. Seeking works because the media handler answers byte
           ranges. -->
      <!-- svelte-ignore a11y_media_has_caption -->
      <video
        class="film"
        controls
        preload="metadata"
        src={media.file(file.id)}
        style:width={`${file.box_width || 480}px`}
        style:aspect-ratio={`${file.box_width || 16} / ${file.box_height || 9}`}
        title={file.name}
      ></video>
    {:else}
      <div class="card" class:gone={file.archived}>
        <span class="kind">{(file.extension || "file").slice(0, 4).toUpperCase()}</span>
        <span class="about">
          <span class="name" title={file.name}>{file.name}</span>
          <span class="size">
            {readableSize(file.size)}{#if file.archived} · archived{/if}
          </span>
        </span>
        {#if !file.archived}
          <button
            type="button"
            class="save"
            onclick={() => save(file)}
            disabled={saving === file.id}
          >
            {saving === file.id ? "Saving…" : "Save"}
          </button>
        {/if}
      </div>
    {/if}
  {/each}
</div>

{#if saved}
  <p class="saved">
    Saved to {saved}
    <button type="button" onclick={() => (saved = "")}>Dismiss</button>
  </p>
{/if}

{#if opened}
  <!-- A plain overlay rather than a component: it exists only while an image is
       open, and the only interactions are close and save. -->
  <div
    class="lightbox"
    role="button"
    tabindex="-1"
    aria-label="Close the image"
    onclick={() => (opened = null)}
    onkeydown={(event) => {
      if (event.key === "Enter" || event.key === " ") opened = null;
    }}
  >
    {#if opened.video}
      <!-- svelte-ignore a11y_media_has_caption -->
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <video
        src={media.file(opened.id)}
        controls
        autoplay
        onclick={(event) => event.stopPropagation()}
      ></video>
    {:else}
      <img
        src={fullSource(opened)}
        alt={opened.name}
        onerror={() => opened && fellBack(opened)}
      />
    {/if}
    {#if viewable.length > 1}
      <!-- Stopping the click here matters: the overlay closes on any click that
           reaches it, and stepping is not closing. -->
      <button
        type="button"
        class="step back"
        aria-label="Previous attachment"
        onclick={(event) => {
          event.stopPropagation();
          step(-1);
        }}>‹</button
      >
      <button
        type="button"
        class="step on"
        aria-label="Next attachment"
        onclick={(event) => {
          event.stopPropagation();
          step(1);
        }}>›</button
      >
    {/if}
    <div class="bar">
      {#if viewable.length > 1}
        <span class="size">{at + 1} / {viewable.length}</span>
      {/if}
      <span class="name">{opened.name}</span>
      <span class="size">{readableSize(opened.size)}</span>
      {#if opened.width > 0}<span class="size">{opened.width}×{opened.height}</span>{/if}
      <button
        type="button"
        onclick={(event) => {
          event.stopPropagation();
          if (opened) void save(opened);
        }}>Save</button
      >
      <button type="button" onclick={() => (opened = null)}>Close</button>
    </div>
  </div>
{/if}

<style>
  .attachments {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin: 4px 0 2px;
  }
  .shot {
    display: block;
    padding: 0;
    border: 1px solid var(--rule);
    border-radius: 6px;
    overflow: hidden;
    cursor: zoom-in;
    /* The mini preview sits here, scaled to fill: the box is never empty while
       the real bytes are on their way. */
    background-size: cover;
    background-position: center;
    background-color: var(--ground);
    max-width: 100%;
  }
  .shot img {
    display: block;
    width: 100%;
    height: 100%;
    /* Contain, not cover: the box is the rendition's own pixel size, so there
       is nothing to crop -- and if a narrow pane has shrunk the box, the
       picture should shrink with it rather than be cut. */
    object-fit: contain;
  }
  .shot {
    max-width: 100%;
    height: auto;
  }
  /* Thumbnails are 120px at most, so they sit in a row and wrap only when the
     pane is genuinely too narrow for the next one. */
  .attachments.gallery {
    gap: 4px;
    align-items: flex-start;
  }
  .card {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border: 1px solid var(--rule);
    border-radius: 6px;
    background: var(--ground);
    max-width: 340px;
  }
  .card.gone {
    opacity: 0.6;
  }
  .film {
    display: block;
    max-width: 100%;
    border-radius: 8px;
    background: #000;
  }
  /* A poster is a `.shot` with a mark on it, so it sits in the row exactly as a
     picture does. */
  .poster {
    position: relative;
    background-color: #000;
  }
  .poster video {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
    /* The button around it is the control; the frame is only a picture. */
    pointer-events: none;
  }
  .play {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    font-size: 22px;
    color: #fff;
    text-shadow: 0 1px 6px rgb(0 0 0 / 0.8);
    pointer-events: none;
  }
  .step {
    position: absolute;
    top: 50%;
    transform: translateY(-50%);
    display: grid;
    place-items: center;
    width: 40px;
    height: 64px;
    font: inherit;
    font-size: 28px;
    line-height: 1;
    border: 0;
    border-radius: 8px;
    background: rgb(0 0 0 / 0.45);
    color: #fff;
    cursor: pointer;
  }
  .step:hover {
    background: rgb(0 0 0 / 0.7);
  }
  .step.back {
    left: 14px;
  }
  .step.on {
    right: 14px;
  }
  .lightbox video {
    max-width: 92vw;
    max-height: 84vh;
    border-radius: 8px;
    background: #000;
  }
  .kind {
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.04em;
    padding: 4px 5px;
    border-radius: 4px;
    background: var(--rule);
    color: var(--ink-soft);
  }
  .about {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .name {
    font-size: 13px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .size {
    font-size: 11.5px;
    color: var(--ink-soft);
  }
  .save {
    font: inherit;
    font-size: 12px;
    padding: 4px 8px;
    border: 1px solid var(--rule);
    border-radius: 4px;
    background: var(--surface);
    color: var(--ink);
    cursor: pointer;
  }
  .save:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .saved {
    margin: 2px 0 4px;
    font-size: 12px;
    color: var(--ink-soft);
    word-break: break-all;
  }
  .saved button {
    font: inherit;
    font-size: 11.5px;
    margin-left: 6px;
    border: 0;
    background: none;
    color: var(--signal);
    cursor: pointer;
  }
  .lightbox {
    position: fixed;
    inset: 0;
    z-index: 40;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 24px;
    background: rgba(0, 0, 0, 0.72);
    cursor: zoom-out;
  }
  .lightbox img {
    max-width: 100%;
    max-height: calc(100% - 48px);
    object-fit: contain;
    border-radius: 4px;
    background: #fff;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 10px;
    color: #fff;
    font-size: 12.5px;
  }
  .bar .name {
    max-width: 40vw;
  }
  .bar .size {
    color: rgba(255, 255, 255, 0.72);
  }
  .bar button {
    font: inherit;
    padding: 5px 10px;
    border: 1px solid rgba(255, 255, 255, 0.35);
    border-radius: 4px;
    background: transparent;
    color: #fff;
    cursor: pointer;
  }
</style>
