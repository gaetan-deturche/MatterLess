<script lang="ts">
  // A face, or initials when there is no picture.
  //
  // The image comes through the `mmedia` scheme, which resolves it in Rust with
  // the session token: the webview sends no Authorization header, so a plain
  // https URL would 401. A user with no avatar is ordinary rather than an error,
  // so a 404 falls back to initials instead of a broken-image icon.
  import * as media from "./media";
  import * as store from "./store.svelte";

  let {
    userId,
    name = "",
    version = 0,
    size = 28,
    presence = false,
  }: {
    userId: string;
    name?: string;
    version?: number;
    size?: number;
    /** Show a presence dot. Off by default: in a message list every row would
     *  carry one, which says nothing about the *conversation* and turns the
     *  margin into a light display. */
    presence?: boolean;
  } = $props();

  /** Read from the store rather than passed in, so one websocket event moves
   *  every dot for that person at once. */
  const status = $derived(presence ? store.statusOf(userId) : undefined);
  const shows = $derived(status && status !== "offline" ? status : undefined);

  /** Reset when the person changes, so one missing avatar does not hide the
   *  next one's. */
  let failed = $state(false);
  let shownFor = $state("");
  $effect(() => {
    if (shownFor !== userId) {
      shownFor = userId;
      failed = false;
    }
  });

  const initials = $derived(
    (name || userId)
      .split(/[.\s_-]+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((part) => part[0]?.toUpperCase() ?? "")
      .join(""),
  );

  /** A stable colour per person, so initials are still recognisable at a
   *  glance. Hue only: saturation and lightness stay put so it never fights
   *  the theme. */
  const hue = $derived(
    [...(userId || name)].reduce((total, character) => total + character.charCodeAt(0), 0) % 360,
  );
</script>

<span class="holder" style:width="{size}px" style:height="{size}px">
  {#if userId && !failed}
    <img
      class="avatar"
      style:width="{size}px"
      style:height="{size}px"
      src={media.avatar(userId, version)}
      alt=""
      loading="lazy"
      onerror={() => (failed = true)}
    />
  {:else}
    <span
      class="avatar initials"
      style:width="{size}px"
      style:height="{size}px"
      style:font-size="{Math.round(size * 0.38)}px"
      style:background="hsl({hue} 45% 42%)"
      aria-hidden="true">{initials}</span
    >
  {/if}
  {#if shows}
    <!-- Offline is the absence of a dot rather than a grey one: a list where
         everyone carries a marker communicates nothing. -->
    <span
      class="dot {shows}"
      style:width="{Math.max(7, Math.round(size * 0.32))}px"
      style:height="{Math.max(7, Math.round(size * 0.32))}px"
      title={shows === "dnd" ? "Do not disturb" : shows === "ooo" ? "Out of office" : shows}
    ></span>
  {/if}
</span>

<style>
  .holder {
    position: relative;
    flex: none;
    display: inline-grid;
  }
  .dot {
    position: absolute;
    right: -1px;
    bottom: -1px;
    border-radius: 50%;
    /* A ring in the surrounding colour, so the dot reads as attached to the
       face rather than floating over the next row. */
    box-shadow: 0 0 0 1.5px var(--surface);
  }
  /* Fallbacks on every one of these: a presence dot whose colour fails to
     resolve is a transparent circle, which reads as *offline* -- the opposite
     of what it is saying. */
  .dot.online {
    background: var(--ok, #4ec49b);
  }
  .dot.away {
    background: var(--flag, #e0a13f);
  }
  .dot.dnd {
    background: var(--danger, #c0392b);
  }
  .dot.ooo {
    background: var(--ink-faint, #6d7d8c);
  }
  .avatar {
    flex: none;
    border-radius: 50%;
    object-fit: cover;
    background: var(--surface-2);
  }
  .initials {
    display: grid;
    place-items: center;
    color: #fff;
    font-weight: 600;
    letter-spacing: 0.02em;
    user-select: none;
  }
</style>
