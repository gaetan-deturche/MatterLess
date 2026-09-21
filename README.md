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
| Renderer | Direct3D 11, through the `windows` crate |
| Text | cosmic-text shaping, swash rasterisation |
| Core | Rust, tokio |
| Local store | SQLite, WAL |
| Auth | Session login |

There is no web view and no JavaScript. The window is one Direct3D swap chain,
and every glyph, avatar, picture and rounded rectangle on it is a quad in a
vertex buffer. Three texture atlases — letters, faces, pictures — are bound
at once and each quad names the one it wants, so nothing about a frame splits
it into separate draws except where one has to be clipped to a panel.

Twelve crates, and the split is the point: `-core` speaks to the server,
`-store` keeps the database, `-sync` folds what arrives into it, `-render`
plans a conversation into rows, `-layout` shapes them, `-paint` turns them
into quads, `-ui` answers the pointer, and `-widgets`, `-sidebar` and `-view`
are the window. Nothing that draws knows what a request is.

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

You need [Rust](https://rustup.rs), Windows, and a GPU whose driver reaches
Direct3D feature level 11.0 — which is anything made since about 2010.

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
slow: drawing the frame took 13ms
ready in 624ms
shaped the rest of the channel behind the window: 251 rows in 1610ms
```

Two examples answer the same questions without a window:

```bash
cargo run --release -p matterless-view --example what_opens    # per channel: plan, first screenful, the rest
cargo run --release -p matterless-layout --example what_shapes # where shaping time goes
```

And `--snapshot <file>` draws a channel to a PNG through the same layout and
the same draw list the window uses, with no window and no GPU — which is how
the rendering gets compared against the official client on a machine with no
display.

```bash
cargo run -p matterless-view -- --snapshot list.png
```

## Tests

```bash
cargo test --workspace          # 675, and five more that need a server
cargo clippy --workspace --all-targets
cargo fmt --all --check
```

Tests named `live_*` are `#[ignore]`d: they talk to a real server and need
`MATTERLESS_TEST_SERVER` set at build time plus a minted token.

## Releasing

Tag a version and CI does the rest:

```bash
git tag v0.1.6 && git push origin v0.1.6
```

The tag has to match `matterless-view`'s own `version`, and the workflow
refuses the build if it does not: the manifest is numbered from the tag and the
app compares it against the crate, so a drift means clients either never update
or are offered the build they are already running.

One repository secret, from the minisign keypair:

- `MATTERLESS_SIGNING_KEY` — the secret key file's contents (base64'd onto one
  line is fine too; it is read either way)
- `MATTERLESS_SIGNING_KEY_PASSWORD` — only if the key has a passphrase. A key
  generated without one is still an encrypted key file, and this repository's
  is one, so the secret is not set.

The public half is compiled into `update::PUBKEY`. Without a signature the
updater refuses an update, which is the point of it.

The installer is [`packaging/matterless.nsi`](packaging/matterless.nsi),
per-user under `%LOCALAPPDATA%` — deliberately, because an update has to
install without a UAC prompt: the thing asking for it is a chat window that has
just been told "yes" by somebody who wanted to keep reading. It reads `/P`,
`/UPDATE`, `/R` and `/ARGS`, which are not NSIS switches but what the updater
sends, waits for the build it is replacing to let go of its own file, and
leaves the message store alone on uninstall.

### Rehearsing one

A tag cannot be pushed twice to get right, so the whole pipeline runs locally
against a throwaway key:

```bash
cargo run -p matterless-release --example a_throwaway_key
makensis -DVERSION=0.1.6 "-DPAYLOAD=<abs>\target\release\matterless-view.exe" "-DOUTFILE=<abs>\setup.exe" packaging/matterless.nsi
MATTERLESS_SIGNING_KEY=... cargo run -p matterless-release -- --installer setup.exe --tag v0.1.6 --url <where it will be>
cargo run -p matterless-view --example what_update -- --manifest latest.json --installer setup.exe --pubkey <the throwaway public half>
```

The last line is the app's own reader checking what the release is about to
publish, and it is the same step the workflow runs before it publishes
anything. Backslashes in the NSIS paths are not a style choice: NSIS reads a
leading `/` as the start of an option, so a forward-slash payload is "no files
found".

## Licence

MIT.
