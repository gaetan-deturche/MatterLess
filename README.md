# MatterLess

A Mattermost desktop client for Windows that never makes you wait.

Mattermost speaks plain HTTP REST plus one WebSocket, so the protocol is the easy
half. This is won or lost on where state lives and how few hops sit between a
keystroke and a glyph — so everything the window draws has already been through
a local SQLite database, and a cold start paints the last channel with no
network at all.

## How it is put together

| | |
|---|---|
| Shell | Tauri v2 |
| Core | Rust, tokio |
| View | Svelte 5 (runes) |
| Local store | SQLite, WAL |
| Auth | Session login |

Four decisions do most of the work:

- **Rust builds the row plan, not JavaScript.** Grouping, date separators,
  markdown parsing and thread footers all happen once in Rust and are cached by
  `post.id + update_at`. The frontend maps over a flat array of typed rows and
  makes no decisions of its own, so a row's height is known before it mounts.
- **One render path.** The UI only ever visualises the store; a command's only
  job is to populate it. A post arrives twice by design — once as your own
  optimistic echo, once over the WebSocket — and only one of them draws.
- **The list is virtualised and the history is unbounded.** Only a screenful is
  mounted, between two spacers, with per-kind height estimates each channel
  learns for itself. There is no cap on how far back you can scroll.
- **Its own scrollbar.** A native thumb is derived from the scroll position, so
  anything else that writes that position moves it out from under your hand
  mid-drag. This one is the other way round: while it is held it is drawn where
  the pointer is, and the list follows.

## Running it

You need [Rust](https://rustup.rs), Node 20+, and the WebView2 runtime (already
present on Windows 11).

```bash
cd app
npm ci
npm run dev:app
```

It asks which Mattermost server to talk to on first run; no host is compiled in.
Your session token goes to the Windows credential store, never to disk or to
JavaScript.

## Building an installer

```bash
cd app
npm run tauri build
```

NSIS only. An installed build updates itself from this repository's releases.

## Tests

```bash
cargo test --workspace          # 243 tests
cargo clippy --workspace --all-targets
cargo fmt --all --check
cd app && npx svelte-check
```

Tests named `live_*` are `#[ignore]`d: they talk to a real server and need
`MATTERLESS_TEST_SERVER` set at build time plus a minted token.

## Releasing

Tag a version and CI does the rest:

```bash
git tag v0.1.0 && git push origin v0.1.0
```

The release workflow needs two repository secrets, generated once with
`npm run tauri signer generate`:

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

The public half goes in `app/src-tauri/tauri.conf.json`. Without a signature the
updater refuses an update, which is the point of it.

## Licence

MIT.
