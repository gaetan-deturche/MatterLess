<script lang="ts">
  // Nothing crosses the IPC boundary while typing. The textarea is
  // uncontrolled-in-spirit: Svelte holds the text, drafts are kept per channel
  // in the in-memory store, and only pressing send invokes anything.
  import Avatar from "./Avatar.svelte";
  import * as api from "./api";
  import * as store from "./store.svelte";
  import * as media from "./media";
  import * as log from "./log";

  let {
    channelId,
    rootId,
    maxFileSize = 0,
    accept = $bindable(),
  }: {
    channelId: string;
    rootId?: string;
    /** The server's `MaxFileSize`. Zero means "not known yet", which lets Rust
     *  be the one to refuse rather than guessing a limit here. */
    maxFileSize?: number;
    /** Bound outwards so a drop anywhere in the pane lands in this composer.
     *
     *  The composer bar is fifty pixels tall, which is a mean target for a
     *  dragged file; the official client takes a drop anywhere over the
     *  conversation, and the tray it lands in is this one. */
    accept?: (files: File[]) => void;
  } = $props();

  /** Drafts are per conversation, and a thread is its own conversation: keying
   *  a thread reply by channel alone would have the two overwrite each other. */
  const draftKey = $derived(rootId ? `${channelId}/${rootId}` : channelId);

  let text = $state("");
  let sending = $state(false);
  let failure = $state("");
  let box: HTMLTextAreaElement | undefined = $state();
  let picker: HTMLInputElement | undefined = $state();

  // Switching conversation restores that conversation's draft rather than
  // losing it. Keyed on the draft key, not the channel: a thread pane shares
  // its channel with the stream behind it, so keying on the channel would have
  // the two composers hand each other their text.
  let lastKey = $state("");
  $effect(() => {
    if (draftKey === lastKey) return;
    if (lastKey) store.setDraft(lastKey, text);
    text = store.draftOf(draftKey);
    lastKey = draftKey;
    failure = "";
  });

  /** Attachments uploaded (or uploading) for the message being written.
   *
   *  Keyed by a handle this side makes up, because the row has to exist while
   *  the upload is still in flight -- the file id only arrives with the
   *  server's answer. */
  type Tray = {
    attachId: string;
    name: string;
    size: number;
    /** Set once the upload lands; until then the row shows progress. */
    file: api.FileRef | null;
    error: string;
    /** Bytes on the wire, against the whole multipart body. -1 until the first
     *  report, so a row can say "uploading" before it can say how far. */
    sent: number;
    total: number;
  };
  let tray: Tray[] = $state([]);
  /** Set while a file is over the composer, so the drop target is visible. */
  let hovering = $state(false);

  /** Uploads in flight cannot be sent yet, and neither can a failed one. */
  const ready = $derived(tray.every((entry) => entry.file !== null));
  const attachedIds = $derived(
    tray.map((entry) => entry.file?.id).filter((id): id is string => Boolean(id)),
  );

  // A tray belongs to the conversation it was filled for: switching channel with
  // half an upload showing would send it to the wrong place. Uploads already on
  // the server are released rather than silently abandoned.
  //
  // Keyed on its own copy of the conversation, not on the draft effect's:
  // effects run in declaration order, so the draft effect has already moved
  // `lastKey` on by the time this one runs and comparing against it would never
  // see a change.
  let trayKey = $state("");
  $effect(() => {
    if (draftKey === trayKey) return;
    trayKey = draftKey;
    for (const entry of tray) {
      if (entry.file) void api.releaseAttachment(entry.file.id);
    }
    tray = [];
  });

  let handles = 0;
  const nextHandle = () => `attach-${Date.now()}-${handles++}`;

  // Published as soon as the composer exists, so the pane above can hand files
  // down without knowing anything about uploading.
  accept = (files: File[]) => void attach(files);

  async function attach(files: File[]) {
    for (const file of files) {
      if (maxFileSize > 0 && file.size > maxFileSize) {
        failure = `${file.name} is ${readableSize(file.size)}; this server allows ${readableSize(maxFileSize)}.`;
        log.info("composer.attach.refused", { bytes: file.size, limit: maxFileSize });
        continue;
      }
      const attachId = nextHandle();
      tray = [
        ...tray,
        {
          attachId,
          name: file.name,
          size: file.size,
          file: null,
          error: "",
          sent: -1,
          total: 0,
        },
      ];
      log.info("composer.attach", {
        channel: channelId,
        bytes: file.size,
        // The name is not logged: a filename is content.
        kind: file.type || "unknown",
      });
      try {
        const bytes = await file.arrayBuffer();
        await api.attachFile(attachId, channelId, file.name, bytes);
      } catch (thrown) {
        markFailed(attachId, String(thrown));
        log.failure("composer.attach.failed", thrown, { channel: channelId });
      }
    }
  }

  function markFailed(attachId: string, message: string) {
    tray = tray.map((entry) =>
      entry.attachId === attachId ? { ...entry, error: message } : entry,
    );
  }

  function drop(attachId: string) {
    const entry = tray.find((held) => held.attachId === attachId);
    // Three different things depending on where it got to: stop it if it is
    // still going, give back the claim if it landed, and forget it either way.
    if (entry && !entry.file && !entry.error) void api.cancelAttachment(attachId);
    if (entry?.file) void api.releaseAttachment(entry.file.id);
    tray = tray.filter((held) => held.attachId !== attachId);
  }

  function readableSize(bytes: number): string {
    if (bytes < 1000) return `${bytes} B`;
    if (bytes < 1000 * 1000) return `${Math.round(bytes / 1000)} kB`;
    return `${(bytes / (1000 * 1000)).toFixed(1)} MB`;
  }

  // The upload's outcome arrives as an event, because the command returns as
  // soon as it has the bytes: a 20 MB drop is long enough that the tray has to
  // be able to say "uploading".
  $effect(() => {
    const stop = api.onAttachment((payload) => {
      tray = tray.map((entry) =>
        entry.attachId === payload.attach_id
          ? { ...entry, file: payload.file, error: payload.error ?? "" }
          : entry,
      );
      if (payload.error) log.info("composer.attach.rejected", { attach: payload.attach_id });
    });
    return () => void stop.then((off) => off());
  });

  $effect(() => {
    const stop = api.onAttachmentProgress((payload) => {
      tray = tray.map((entry) =>
        entry.attachId === payload.attach_id
          ? { ...entry, sent: payload.sent, total: payload.total }
          : entry,
      );
    });
    return () => void stop.then((off) => off());
  });

  function onPaste(event: ClipboardEvent) {
    // Pasted files only: a pasted screenshot arrives as an image item with no
    // name, and text should keep going into the textarea untouched.
    const items = Array.from(event.clipboardData?.items ?? []);
    const pasted = items
      .filter((item) => item.kind === "file")
      .map((item) => item.getAsFile())
      .filter((file): file is File => file !== null);
    if (pasted.length === 0) return;
    event.preventDefault();
    void attach(
      pasted.map((file) =>
        file.name && file.name !== "image.png"
          ? file
          : // A pasted image is named `image.png` by the browser, which makes
            // every screenshot in a channel look identical. A timestamp is at
            // least distinguishable.
            new File([file], `pasted-${new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-")}.${(file.type.split("/")[1] ?? "png").replace("jpeg", "jpg")}`, {
              type: file.type,
            }),
      ),
    );
  }

  function onDrop(event: DragEvent) {
    hovering = false;
    const dropped = Array.from(event.dataTransfer?.files ?? []);
    if (dropped.length === 0) return;
    event.preventDefault();
    void attach(dropped);
  }

  // ---- completion of @people, ~channels and :emoji: ----------------------
  //
  // Answered from the local store, so the list is up before the next keystroke;
  // see the `suggest` command. Only the token under the caret is completed, and
  // only while it is being typed.

  /** What is being completed right now, or nothing. */
  let token: { kind: string; start: number; end: number; query: string } | null = $state(null);
  let suggestions: api.Suggestion[] = $state([]);
  let chosen = $state(0);
  /** Every request is numbered and only the newest may write the list.
   *
   *  Without this the *last response to arrive* wins rather than the last one
   *  asked for, and the list flickers between two answers as you type. */
  let asked = 0;

  const TRIGGERS: Record<string, string> = { "@": "user", "~": "channel", ":": "emoji" };

  /** The trigger and the query immediately before the caret.
   *
   *  Anchored on a word boundary, so an address does not complete a channel and
   *  `10:30` does not complete an emoji. A colon needs two characters before it
   *  offers anything, because ": " is far more often punctuation than an emoji.
   */
  function tokenAtCaret(): typeof token {
    if (!box) return null;
    const caret = box.selectionStart ?? 0;
    const before = text.slice(0, caret);
    const match = /(?:^|[\s(])([@~:])([\p{L}\p{N}_.+-]*)$/u.exec(before);
    if (!match) return null;
    const kind = TRIGGERS[match[1] as string];
    const query = match[2] ?? "";
    if (!kind) return null;
    if (kind === "emoji" && query.length < 2) return null;
    return { kind, start: caret - query.length - 1, end: caret, query };
  }

  async function offerCompletions() {
    const found = tokenAtCaret();
    token = found;
    if (!found) {
      suggestions = [];
      return;
    }
    const mine = ++asked;
    try {
      const rows = await api.suggest(found.kind, found.query, channelId);
      // A stale answer is thrown away rather than shown.
      if (mine !== asked) return;
      suggestions = rows;
      chosen = 0;
    } catch (thrown) {
      if (mine !== asked) return;
      suggestions = [];
      log.failure("composer.suggest.failed", thrown, { kind: found.kind });
    }
  }

  function acceptCompletion(suggestion: api.Suggestion) {
    if (!token) return;
    const inserted =
      token.kind === "emoji"
        ? `:${suggestion.value}: `
        : token.kind === "user"
          ? `@${suggestion.value} `
          : `~${suggestion.value} `;
    const caret = token.start + inserted.length;
    text = text.slice(0, token.start) + inserted + text.slice(token.end);
    store.setDraft(draftKey, text);
    suggestions = [];
    token = null;
    // The caret belongs after what was just inserted, which needs the textarea
    // to have the new value first.
    queueMicrotask(() => {
      box?.setSelectionRange(caret, caret);
      box?.focus();
      grow();
    });
  }

  /** Returns true when the key was the completion list's to handle. */
  function completionKey(event: KeyboardEvent): boolean {
    if (suggestions.length === 0) return false;
    switch (event.key) {
      case "ArrowDown":
        chosen = (chosen + 1) % suggestions.length;
        return true;
      case "ArrowUp":
        chosen = (chosen - 1 + suggestions.length) % suggestions.length;
        return true;
      case "Tab":
      case "Enter": {
        const picked = suggestions[chosen];
        if (picked) acceptCompletion(picked);
        return true;
      }
      case "Escape":
        suggestions = [];
        token = null;
        return true;
      default:
        return false;
    }
  }

  function grow() {
    if (!box) return;
    box.style.height = "auto";
    box.style.height = `${Math.min(box.scrollHeight, 200)}px`;
  }

  async function send() {
    const body = text.trim();
    // An image with no words is an ordinary message; an upload still in flight
    // is not ready to be one.
    if ((!body && attachedIds.length === 0) || sending || !ready) return;
    sending = true;
    failure = "";
    log.info("composer.send", {
      channel: channelId,
      chars: body.length,
      files: attachedIds.length,
    });
    // Clear straight away: the message is already on screen as a pending row,
    // so leaving it in the box would show it twice.
    text = "";
    store.setDraft(draftKey, "");
    queueMicrotask(grow);
    // Taken before the tray is cleared: the pending row already shows them, so
    // leaving them in the tray would show each attachment twice.
    const files = attachedIds;
    tray = [];
    try {
      await api.sendPost(channelId, body, rootId, files);
    } catch (thrown) {
      // The pending row is marked failed by Rust and carries the text, so
      // nothing is lost even though the box is empty.
      failure = String(thrown);
      log.failure("composer.send.failed", thrown, { channel: channelId });
    } finally {
      sending = false;
      // Deliberately does NOT refresh the list. The send already emits a delta,
      // and the store is the only thing allowed to drive a render -- a callback
      // that reloaded here would be a second render path.
    }
  }

  function onKeyDown(event: KeyboardEvent) {
    // The completion list gets first refusal: while it is open, Enter and Tab
    // are choosing a name, not sending a message.
    if (completionKey(event)) {
      event.preventDefault();
      return;
    }
    // Enter sends; Shift+Enter is a newline. IME composition must not send.
    if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      void send();
    }
  }

  function onInput() {
    store.setDraft(draftKey, text);
    grow();
    void offerCompletions();
    // Every keystroke; the engine decides what actually reaches the socket.
    // Not for an empty box: clearing what you typed is not typing.
    if (text.length > 0) void api.sendTyping(channelId, rootId);
  }
</script>

{#if tray.length}
  <ul class="tray">
    {#each tray as entry (entry.attachId)}
      <li class:failed={Boolean(entry.error)}>
        {#if entry.file?.image && entry.file.mini_preview}
          <img
            class="chip"
            src={`data:image/jpeg;base64,${entry.file.mini_preview}`}
            alt={entry.name}
          />
        {:else}
          <span class="chip kind">
            {(entry.name.split(".").pop() ?? "file").slice(0, 4).toUpperCase()}
          </span>
        {/if}
        <span class="about">
          <span class="name" title={entry.name}>{entry.name}</span>
          <span class="state">
            {#if entry.error}
              {entry.error}
            {:else if entry.file}
              {readableSize(entry.size)}
            {:else if entry.sent >= 0 && entry.total > 0}
              {Math.min(100, Math.round((entry.sent / entry.total) * 100))}% of
              {readableSize(entry.size)}
            {:else}
              uploading… {readableSize(entry.size)}
            {/if}
          </span>
          {#if !entry.file && !entry.error && entry.sent >= 0 && entry.total > 0}
            <!-- A bar as well as a number: the number says how far, the bar
                 says whether it is still moving. -->
            <span class="bar" aria-hidden="true">
              <i style:width="{Math.min(100, (entry.sent / entry.total) * 100)}%"></i>
            </span>
          {/if}
        </span>
        <button
          type="button"
          class="drop"
          onclick={() => drop(entry.attachId)}
          title={entry.file || entry.error ? "Remove" : "Cancel upload"}
        >
          ×
        </button>
      </li>
    {/each}
  </ul>
{/if}

{#if suggestions.length}
  <!-- Above the box, because the box is at the bottom of the window: a list
       that opened downward would be off screen. -->
  <ul class="completions" role="listbox" aria-label="Completions">
    {#each suggestions as suggestion, index (suggestion.kind + suggestion.value)}
      <li>
        <button
          type="button"
          role="option"
          aria-selected={index === chosen}
          class:chosen={index === chosen}
          onmouseenter={() => (chosen = index)}
          onclick={() => acceptCompletion(suggestion)}
        >
          {#if suggestion.kind === "user"}
            <Avatar
              userId={suggestion.id}
              name={suggestion.label}
              version={suggestion.avatar_at}
              size={20}
            />
          {:else if suggestion.kind === "emoji"}
            {#if suggestion.id}
              <img class="glyph" src={media.emoji(suggestion.id)} alt="" />
            {:else}
              <span class="glyph">{suggestion.detail}</span>
            {/if}
          {:else}
            <span class="glyph">{suggestion.channel_type === "P" ? "🔒" : "#"}</span>
          {/if}
          <span class="what">{suggestion.label}</span>
          {#if suggestion.kind === "user" && suggestion.detail}
            <span class="who">{suggestion.detail}</span>
          {/if}
        </button>
      </li>
    {/each}
  </ul>
{/if}

<!-- The drop target is the composer itself, and `dragDropEnabled` is false in
     tauri.conf.json so the webview sees the drop rather than the window
     swallowing it. -->
<div
  class="composer"
  class:hovering
  ondragover={(event) => {
    event.preventDefault();
    hovering = true;
  }}
  ondragleave={() => (hovering = false)}
  ondrop={onDrop}
  role="group"
>
  <button
    type="button"
    class="clip"
    title="Attach a file"
    onclick={() => picker?.click()}
    disabled={sending}>📎</button
  >
  <input
    bind:this={picker}
    type="file"
    multiple
    hidden
    onchange={(event) => {
      const chosen = Array.from(event.currentTarget.files ?? []);
      // Reset first: choosing the same file twice must fire again.
      event.currentTarget.value = "";
      void attach(chosen);
    }}
  />
  <textarea
    bind:this={box}
    bind:value={text}
    onkeydown={onKeyDown}
    oninput={onInput}
    onpaste={onPaste}
    placeholder="Write a message… (Enter to send, Shift+Enter for a new line)"
    rows="1"
  ></textarea>
  <button
    type="button"
    onclick={send}
    disabled={(!text.trim() && attachedIds.length === 0) || sending || !ready}>Send</button
  >
</div>

{#if failure}
  <p class="failure">{failure}</p>
{/if}

<style>
  .completions {
    list-style: none;
    margin: 0 16px;
    padding: 4px;
    border: 1px solid var(--rule);
    border-radius: 6px;
    background: var(--surface);
    box-shadow: 0 -4px 14px rgb(0 0 0 / 0.18);
    max-height: 240px;
    overflow-y: auto;
  }
  .completions button {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    text-align: left;
    font: inherit;
    font-size: 13px;
    padding: 4px 7px;
    border: 0;
    border-radius: 4px;
    background: none;
    color: var(--ink);
    cursor: pointer;
  }
  .completions button.chosen {
    background: var(--signal-soft);
  }
  .completions .glyph {
    width: 20px;
    height: 20px;
    display: grid;
    place-items: center;
    font-size: 15px;
    color: var(--ink-faint);
  }
  .completions .who {
    color: var(--ink-faint);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .composer {
    display: flex;
    gap: 8px;
    align-items: flex-end;
    padding: 10px 16px 14px;
    border-top: 1px solid var(--rule);
    background: var(--surface);
  }
  textarea {
    flex: 1;
    font: inherit;
    font-size: 14px;
    line-height: 1.45;
    resize: none;
    overflow-y: auto;
    padding: 8px 10px;
    border: 1px solid var(--rule);
    border-radius: 5px;
    background: var(--ground);
    color: var(--ink);
  }
  textarea:focus-visible {
    outline: 2px solid var(--signal);
    outline-offset: -1px;
  }
  button {
    font: inherit;
    padding: 8px 14px;
    border: 0;
    border-radius: 5px;
    background: var(--signal);
    color: #fff;
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .bar {
    display: block;
    height: 3px;
    margin-top: 3px;
    border-radius: 2px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .bar i {
    display: block;
    height: 100%;
    background: var(--signal);
    border-radius: 2px;
  }
  .tray {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    list-style: none;
    margin: 0;
    padding: 8px 16px 0;
  }
  .tray li {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 4px 6px;
    border: 1px solid var(--rule);
    border-radius: 6px;
    background: var(--ground);
    max-width: 260px;
  }
  .tray li.failed {
    border-color: var(--flag);
  }
  .chip {
    width: 28px;
    height: 28px;
    border-radius: 4px;
    object-fit: cover;
    background: var(--rule);
  }
  .chip.kind {
    display: grid;
    place-items: center;
    font-size: 9.5px;
    font-weight: 600;
    color: var(--ink-soft);
  }
  .about {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .about .name {
    font-size: 12.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .about .state {
    font-size: 11px;
    color: var(--ink-soft);
  }
  .failed .about .state {
    color: var(--flag);
  }
  .drop {
    font: inherit;
    font-size: 15px;
    line-height: 1;
    padding: 0 3px;
    border: 0;
    background: none;
    color: var(--ink-soft);
    cursor: pointer;
  }
  .composer.hovering {
    outline: 2px dashed var(--signal);
    outline-offset: -3px;
  }
  .clip {
    font-size: 15px;
    padding: 7px 8px;
    border: 1px solid var(--rule);
    border-radius: 5px;
    background: var(--ground);
    color: var(--ink);
    cursor: pointer;
  }
  .failure {
    color: var(--flag);
    font-size: 12.5px;
    margin: 0;
    padding: 0 16px 10px;
  }
</style>
