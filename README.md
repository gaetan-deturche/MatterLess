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
| Window | winit |
| Renderer | wgpu on Vulkan |
| Text | cosmic-text shaping, swash rasterisation |
| Core | Rust, tokio |
| Local store | SQLite, WAL |
| Auth | Session login |

There is no web view and no JavaScript. The window is one Vulkan surface, and
every glyph, avatar, picture and rounded rectangle on it is a quad in a single
draw call against four texture atlases.

Four decisions do most of the work:

- **The row plan is built once, in Rust.** Grouping, date separators, markdown
  parsing and thread footers all happen in `matterless-render`, away from
  anything that draws.
- **A row's height is computed, never estimated.** `matterless-layout` shapes the
  real text with the real font and reports how many lines it occupies, so
  nothing has to be measured after the fact and corrected. Estimating heights
  and reconciling them afterwards was the source of every scroll artefact in the
  shell this replaced.
- **One render path.** The window only ever visualises the store; a request's
  only job is to populate it. A post arrives twice by design — once as your own
  optimistic echo, once over the WebSocket — and only one of them draws.
- **The history is unbounded.** There is no cap on how far back you can scroll.
  A channel opens by shaping only enough of its newest end to fill the panel and
  shaping the rest behind the window, so a conversation of four hundred crash
  reports opens as fast as a quiet one.

## Running it

You need [Rust](https://rustup.rs) and a GPU with a Vulkan driver.

```bash
cargo run -p matterless-view
```

It asks which Mattermost server to talk to on first run; no host is compiled in.
Your session token goes to the Windows credential store, never to disk.

A channel id can be passed to open it directly, which is how a specific
conversation gets looked at without clicking:

```bash
cargo run -p matterless-view -- "%APPDATA%\com.gaetandeturche.matterless.dev\matterless.db" <channel-id>
```

## What it says about itself

A dev build times its own hot paths and prints anything slow enough to be felt
(`matterless-view/src/timing.rs`). A release build reads no clock at all.

```
slow: shaping the channel took 81ms for 12 rows, 6.79ms each
ready in 624ms, of which 215ms was Vulkan up to the device
shaped the rest of the channel behind the window: 251 rows in 1610ms
```

Two examples answer the same questions without a window:

```bash
cargo run --release -p matterless-view --example what_opens    # per channel: plan, first screenful, the rest
cargo run --release -p matterless-layout --example what_shapes # where shaping time goes
```

## Tests

```bash
cargo test --workspace          # 496 tests
cargo clippy --workspace --all-targets
cargo fmt --all --check
```

Tests named `live_*` are `#[ignore]`d: they talk to a real server and need
`MATTERLESS_TEST_SERVER` set at build time plus a minted token.

## Releasing

**This does not work at the moment.** `.github/workflows/release.yml` still
builds the retired Tauri shell with `tauri-action`, which produced the
installer, the minisign signature and the `latest.json` manifest. That shell is
gone, so the workflow has nothing to build.

The updater itself is not Tauri's and did not go with it: `update.rs` fetches
the manifest, checks the minisign signature against `update::PUBKEY` and runs
the installer. What is missing is the half that *makes* a release — bundling
`matterless-view.exe` into an NSIS installer, signing it with the private key in
this repository's secrets, and writing a `latest.json` numbered from
`matterless-view`'s own `version`.

## Licence

MIT.
