<script lang="ts">
  // What kind of conversation this is, at a glance.
  //
  // A direct message shows the other person's face; everything else gets a
  // glyph, because a public channel and a private one differ in a way that
  // matters before you post in it.
  import Avatar from "./Avatar.svelte";
  import type { ChannelSummary } from "./api";

  let { channel, size = 16 }: { channel: ChannelSummary; size?: number } = $props();
</script>

{#if channel.channel_type === "D" && channel.counterpart_id}
  <!-- The one place a presence dot earns its keep: whether somebody is around
       decides whether you write to them now. -->
  <Avatar
    userId={channel.counterpart_id}
    name={channel.display_name}
    size={size + 2}
    presence
  />
{:else if channel.channel_type === "P"}
  <!-- A padlock, because posting in a private channel is a different act. -->
  <svg class="icon" width={size} height={size} viewBox="0 0 16 16" aria-label="Private channel">
    <path
      d="M4.5 7V5.5a3.5 3.5 0 1 1 7 0V7h.5a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V8a1 1 0 0 1 1-1h.5Zm1.5 0h4V5.5a2 2 0 1 0-4 0V7Z"
      fill="currentColor"
    />
  </svg>
{:else if channel.channel_type === "G"}
  <!-- Several people: a group message has no single face to show. -->
  <svg class="icon" width={size} height={size} viewBox="0 0 16 16" aria-label="Group message">
    <path
      d="M5.5 7a2.25 2.25 0 1 0 0-4.5 2.25 2.25 0 0 0 0 4.5Zm5.25.5a1.75 1.75 0 1 0 0-3.5 1.75 1.75 0 0 0 0 3.5ZM1.5 12.5c0-1.933 1.79-3.5 4-3.5s4 1.567 4 3.5V13h-8v-.5Zm9.06-2.5c1.63.13 2.94 1.4 2.94 2.9V13h-3.1v-.5c0-.92-.31-1.78-.84-2.47.33-.03.67-.04 1-.03Z"
      fill="currentColor"
    />
  </svg>
{:else}
  <!-- Public: the hash every chat client uses, so it needs no explaining. -->
  <svg class="icon" width={size} height={size} viewBox="0 0 16 16" aria-label="Public channel">
    <path
      d="M6.2 2h1.35l-.6 3.1h2.5L10.05 2h1.35l-.6 3.1H13v1.3h-2.45l-.5 2.6H12.5v1.3h-2.7L9.2 14H7.85l.6-3.1h-2.5L5.35 14H4l.6-3.1H2.5V9.6h2.35l.5-2.6H3V5.7h2.6L6.2 2Zm.35 5 -.5 2.6h2.5l.5-2.6h-2.5Z"
      fill="currentColor"
    />
  </svg>
{/if}

<style>
  .icon {
    flex: none;
    color: var(--ink-faint);
    opacity: 0.85;
  }
</style>
