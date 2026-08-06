# CLAUDE.md — tmc-app

Guidance for working in this repository. Specific to `tmc-app`; separate from
`../website-city/CLAUDE.md` and `../tmc-global/CLAUDE.md`.

## What this is

The official **The Modding Community** app: Rust + Tauri 2 + React 19, shipping
to **Windows, macOS, Linux, Android and iOS from one codebase**.

It covers what the website covers — browsing and viewing assets, mods, servers,
server maps, articles, communities, collections and members — plus the things a
browser tab cannot do:

- **One-click installs** through a declarative plugin system
- **Real latency pings** measured from the user's own device
- **Live server queries** over game-specific protocols
- **Local-first settings**, kept deliberately separate from account settings

It is **not** a wrapper around the website. There is no landing page, no
marketing header, no footer. It opens on a browser and stays there.

## Sibling repositories

| Repo | Relationship |
| --- | --- |
| `../website-city` | The Next.js site. **Owns the API this app speaks** (`/api/app/v1`) and the auth flow. Changes there usually need a matching change here. |
| `../tmc-global` | `@modcommunity/shared` — design tokens and a few UI primitives, consumed from GitHub Packages. `npm run shared:local` swaps in the sibling checkout. |
| `../website-processing` | Astro landing site. No relationship to this app beyond sharing the design system. |

## Architecture, and the one rule everything follows

> **Credentials and the filesystem live in Rust. The webview gets data.**

There is no `fetch` in the frontend, no `get_token` command, and no command that
takes an arbitrary path. A mod description is untrusted text rendered inside a
webview that can call `invoke`; the defence is that there is nothing worth
stealing on that side of the bridge. Every layer below assumes an attacker has
already achieved script execution in the webview and asks what they can reach.

```
React (src/)                       ← untrusted content renders here
  │  invoke() via src/lib/ipc      ← every call parsed through a zod schema
  ▼
tmc-app  (src-tauri/src/)          ← the Tauri shell, and nothing else
  ├── commands/   the IPC surface
  ├── state.rs    AppState, assembled once
  └── paths.rs    platform paths, from Tauri's resolver
  │
  ▼
tmc-core (src-tauri/core/src/)     ← NO Tauri dependency; tests anywhere
  ├── auth.rs     PKCE + device grant; tokens never leave this process
  ├── secure.rs   refresh token → OS credential store
  ├── api.rs      the ONLY HTTP client; attaches the bearer, handles refresh
  ├── net/        address guard, transports, latency history, game protocols
  ├── plugins/    manifest → sandbox → step executor, all declarative
  └── logging.rs  the audit trail Settings → Logging reads
  │
  ├──► website-city  /api/app/v1   (catalogue, auth)
  └──► game servers directly       (live queries, latency)
```

**The workspace split is load-bearing.** `tmc-core` holds the plugin sandbox,
the protocol parsers and the token lifecycle — the three things you most want to
compile and fuzz on a CI runner with no display stack, no WebKit and no dbus.
`cargo test -p tmc-core` needs none of them. Anything you add that does not
genuinely need a window belongs on that side of the line.

### Source map

**`src-tauri/core/src/` — `tmc-core`**

| File | Owns |
| --- | --- |
| `api.rs` | HTTP. `API_BASE` is compile-time, not a setting |
| `auth.rs` | PKCE, device-grant state, in-memory access token |
| `secure.rs` | Keychain / Credential Manager / Secret Service, file fallback on mobile |
| `settings.rs` | App-local settings (`settings.json`), clamped on read |
| `logging.rs` | Append-only JSONL audit log + `audit!` macro |
| `net/addr.rs` | **The public-address guard.** Resolve once, connect to that |
| `net/transport.rs` | Bounded UDP/TCP exchanges — every read has a deadline and a cap |
| `net/reader.rs` | The non-panicking byte cursor every parser is built on |
| `net/ping.rs` | TCP connect timing, for games with no protocol |
| `net/latency.rs` | Rolling per-server history, bounded on both axes |
| `net/query/` | The game protocols — see below |
| `plugins/manifest.rs` | The manifest format and its validation |
| `plugins/sandbox.rs` | The path jail |
| `plugins/steps.rs` | The install/uninstall executor |
| `plugins/query.rs` | The declarative parser for Server Live Query plugins |
| `plugins/theme.rs` | Theme token validation |
| `plugins/registry.rs` | Installed plugins, approvals, fingerprint drift |

**`src-tauri/src/` — `tmc-app`**

| File | Owns |
| --- | --- |
| `lib.rs` | Builder, plugin registration, the command list |
| `commands/` | The whole IPC surface. Nothing privileged happens outside it |
| `state.rs` | `AppState`, assembled once — shared locks and caches depend on that |
| `paths.rs` | Every path, from Tauri's resolver — never `$HOME` |

**`src/`**

| Path | Owns |
| --- | --- |
| `lib/ipc/` | `call()` + schemas + `ipc.*`. The only place `invoke` is imported |
| `lib/api/contract.ts` | **Mirror** of website-city's contract. `npm run contract:sync` |
| `lib/api/client.ts` | `api.*`, every response zod-parsed |
| `lib/auth/provider.tsx` | Login state and the poll loop |
| `lib/settings/provider.tsx` | App settings + account settings, kept apart |
| `lib/hooks/use-breakpoint.ts` | Layout decisions, keyed on the window |
| `lib/hooks/use-live-query.tsx` | The live-server registry: one timer, one batch |
| `components/shell.tsx` | Sidebar ≥768px, bottom tabs below |
| `components/markdown.tsx` | The safe renderer for untrusted bodies |
| `components/latency-graph.tsx` | Sparkline + full chart, hand-rolled SVG |
| `components/server-live.tsx` | The live strip on a server card |
| `components/server-panel.tsx` | The live panel on a server's page |
| `routes/` | Browse, view, account, settings panes |

## Authentication

**OAuth 2.0 Device Authorization Grant (RFC 8628) with PKCE.** The user signs in
on the real website in their real browser; the app never sees a password.

```
app                                    website-city
 │ POST /auth/device {client, challenge}
 │ ─────────────────────────────────────▶  AppDeviceGrant (PENDING)
 │ ◀───────────────────────────────────── {deviceCode, userCode, uri}
 │
 │ opens the SYSTEM browser at /login/device?code=XXXX-XXXX
 │                                          user signs in, approves
 │                                          appDevice.approve → APPROVED
 │ POST /auth/token {deviceCode, verifier}
 │ ─────────────────────────────────────▶  atomic APPROVED → CLAIMED
 │ ◀───────────────────────────────────── {accessToken, refreshToken, user}
```

Non-obvious properties, all load-bearing:

- **PKCE is not optional.** A stolen `deviceCode` is inert without the verifier,
  which never leaves the app's memory. A wrong verifier **destroys** the grant.
- **The grant is claimed atomically** (`updateMany` with `status: 'APPROVED'` in
  the WHERE). The app polls on a timer *and* on the deep link, so two claims
  racing is the normal case, not an edge case.
- **Refresh tokens rotate, and reuse kills the family.** A retired token coming
  back means two parties hold one lineage; every device is revoked and
  `reuseDetected` is set. `ApiClient::refresh` holds a mutex so the app never
  does this to itself.
- **The access token is memory-only.** One hour, treated as stale 60s early.
- **`tmc://` is a wake-up, not a credential.** Any app can claim a custom
  scheme, so nothing is read out of the URL — it only triggers an immediate
  poll.
- **System browser, never a webview.** An embedded login hides the address bar
  and the password manager, which is what makes it indistinguishable from
  phishing.

Website-side files: `src/server/auth/app-token.ts`,
`prisma/models/app-device.prisma`, `src/app/api/app/v1/auth/*`,
`src/server/api/routers/app-device.ts`, `src/app/[locale]/login/device/page.tsx`.

## The API contract

`website-city/src/types/app-api/contract.ts` is the **source of truth**;
`src/lib/api/contract.ts` is a **verbatim copy**. When the server side changes,
copy the file across — do not hand-edit the app's copy.

Every response is parsed through these schemas before it reaches a component, so
a drift is a loud error on the first request instead of an `undefined` three
screens deep. That is the entire reason the duplication is acceptable.

Why a separate REST API rather than the website's tRPC:

- The website's routers return **Prisma payloads** shaped by what a page renders.
  An installed app is not redeployed with the server, so its wire format has to
  be versioned and narrow.
- Every content type has a **different** row shape. The app draws one grid; the
  normalisation into `ContentSummary` has to happen once, server-side.
- Rust calls it too.

Endpoints: `/auth/device`, `/auth/token`, `/auth/refresh`, `/auth/revoke`, `/me`
(GET + PATCH), `/browse`, `/content/:kind/:id`, `/facets`.

## Live server queries

**The app talks to game servers itself.** The website can only show you what its
scanner last saw, from wherever the scanner lives; the app sends the game's own
query datagram from the user's device, so the player count is current and the
latency is the one that player will actually experience. A Sydney player and a
Frankfurt player see different numbers for the same box, and only the app can
tell either of them the truth.

### Where the protocol comes from

`App.srvQueryProtocols` on the website — the same list its own scanners use, so
there is no second source to drift. It arrives on every server row as
`server.query`, along with the port rules (`swapGamePort`, `portOffset`,
`timeoutMs`) that decide *which* port to hit. Getting the query port wrong is
the single most common reason a live server shows as dead, which is why those
rules are exposed rather than reimplemented.

`server.query` is null when the owner hid the network details or the game
declares no protocol. In both cases there is nothing the app is entitled to
probe, and it must not fall back to guessing.

### Implemented protocols

| Protocol | Games | Notes |
| --- | --- | --- |
| `A2S` | CS2, TF2, Rust, ARK, Garry's Mod, Squad | Challenge handshake + split-reply reassembly |
| `MINECRAFT` | 1.7+ | TCP handshake → status JSON |
| `MINECRAFT_SLP` | pre-1.7 | `0xFE 0x01`, UTF-16BE reply |
| `QUAKE3` | CoD, Wolfenstein, Xonotic, OpenArena | `getstatus` |
| `GAMESPY1` | Unreal Tournament, early Battlefield | `\status\` |
| `GAMESPY2` | Battlefield 2, UT2004 | Binary, per-section request byte |
| `GAMESPY3` | Minecraft's query port, many others | Signed challenge — see below |
| `SAMP` | SA-MP, open.mp | IPv4 only, by protocol design |
| `FIVEM` | GTA V, RedM | HTTP `/dynamic.json` + `/info.json` |

Everything else in the enum is recognised but falls through to `TCP_ONLY`, which
still yields a real latency figure. A Server Live Query plugin can cover any of
them without an app update.

### Rules for anything under `net/query/`

These parsers read bytes from an unauthenticated machine that may be actively
hostile, and the release profile sets `panic = "abort"` — so **a panic in a
parser is a remote kill switch**. Consequently:

- **No indexing, no slicing.** Everything goes through `net::reader::Reader`,
  which bounds-checks every read and returns `Err` past the end.
- **Every length from the wire is checked against what remains.** A server
  claiming a 4 GB string is the normal case to handle, not an edge case.
- **Every loop is bounded** — fragments, entries, rules, roster length.
- **A truncated-input test is mandatory.** Each protocol has one that feeds
  every prefix of a valid reply through the parser; it must never panic.
- **`resolve_public` is the only way to get a `SocketAddr`.** Resolve once and
  connect to *that* — checking a hostname and reconnecting by name is a DNS
  rebinding hole, and the guard is what stops the browser being an SSRF tool
  against the user's own LAN.

Three protocol details that are easy to get wrong and are commented at the site:

- **A2S's `0x41` challenge.** Skip it and every modern server looks unreachable.
- **A2S's challenge is bound to the source PORT.** Use `net::transport::
  UdpSession` so the retry goes out on the socket that asked; a fresh ephemeral
  port is ignored by several server builds. It is also what makes split replies
  readable, since the continuation datagrams arrive unprompted on that socket.
- **GameSpy v3's challenge is a SIGNED decimal string.** Parse it unsigned and
  roughly half of all servers look unreachable.

### The registry

`LiveQueryProvider` holds one timer and one registry. Cards register via an
`IntersectionObserver` while on screen; every tick sends the whole visible set
to Rust in a single `query_servers` call, which applies the user's concurrency
cap. Fifty cards each polling would be fifty timers, fifty IPC round trips and
fifty simultaneous sockets — which a phone's radio and most home routers handle
badly.

Deregistration has a 5-second grace so a flick-scroll does not cancel work
already in flight, and polling stops entirely while the window is hidden.

### Latency history

`net::latency::LatencyStore` keeps up to 60 samples for up to 512 servers, in
memory only — the series is interesting while the browser is open, is entirely
reconstructible by re-measuring, and persisting it would mean a disk write per
scroll tick. **Failed probes are recorded as `None`, not dropped**: an
intermittently-dead server must not graph as perfectly stable.

Graphs are hand-rolled SVG (`components/latency-graph.tsx`). No chart library:
these render once per card in a scrolling grid, and a general-purpose chart's
layout pass and scales get paid fifty times for a sixty-point line with no axes.
A dropped probe **breaks the line** rather than interpolating across it, and the
Y scale has a 40ms floor so a rock-steady server does not render as a mountain
range.

"Sort by ping" in the server browser is client-side and could only be: latency
is a fact about this device's route, so the API cannot order by it.

## The plugin system

**A plugin is data, never code.** No script engine, no WASM, no `exec` step.
An installer describes file operations; a query plugin describes a datagram and
how to read the reply; a theme describes colour tokens. Everything a plugin can
express is something this crate implements and audits.

That is a real limitation and it is the point: the alternative is third-party
code running with full filesystem access next to the user's saves and
credentials, and nothing bolted on afterwards recovers from that.

### Three types

| Type | Declares | Runtime |
| --- | --- | --- |
| **Installer** | Ordered `install` / `uninstall` steps | `plugins/steps.rs` |
| **Server Live Query** | A hex request + a field list | `plugins/query.rs` |
| **Theme** | A map of CSS custom properties | `plugins/theme.rs` |

### The step vocabulary

`download`, `extract`, `copy`, `move`, `mkdir`, `remove`, `writeText`,
`patchJson`. **Adding a step that runs a program, resolves an absolute path, or
reads an environment variable breaks the model** — do not.

Every path is a `PathRef { root, path }` where `root` is one of `gameDir`,
`pluginData`, `downloads`. The type cannot express an absolute path.

**`path` is relative to the ROOT, not to any grant.** A plugin granted
`{gameDir, "mods", write}` writes to `{ root: gameDir, path: "mods/foo.jar" }`.
Grants are *filters* over the resolved path, so several narrow grants under one
root compose as a union and stay narrow — the alternative, anchoring each grant
at its own subdirectory, cannot say which grant a `PathRef` meant and used to
merge them into the widest one.

### The defences, and what each one catches

| Control | Catches |
| --- | --- |
| Lexical rejection in `join_relative` | `../`, absolute paths, drive prefixes, UNC, NUL, trailing dot/space (Windows), `:` (ADS) |
| Post-join containment | Component sequences that combine badly |
| Canonicalised deepest existing ancestor | A symlink already on disk pointing out of the jail |
| `enclosed_name` + re-check on zip entries | Zip-slip |
| Tar link/special-file refusal | `link → /etc` followed by `link/passwd` |
| Extract byte + entry caps | Zip bombs |
| Streaming download counter | A server lying in `Content-Length` |
| `https`-only + host allow-list, re-checked after placeholder substitution | A template smuggling in a host |
| SHA-256 verification, file deleted on mismatch | A tampered download reaching a later step |
| `is_public()` in `net.rs` | SSRF into the LAN or a cloud metadata endpoint |
| Theme token + colour allow-list | `url()` beacons, `display:none` on the uninstall button |
| Grants as filters over a root-relative path | Two grants for one root merging into the widest |
| Manifest fingerprint | An update silently widening permissions |

### Approval

`plugin_inspect` returns a fingerprint (SHA-256 of the canonical manifest) and
the full permission list; `plugin_approve` takes the fingerprint back. That
closes the window between the user reading the permissions and clicking approve.

`Registry::rescan` runs every launch and re-hashes each installed manifest. A
drifted plugin is **disabled** and flagged `needsReapproval`; the toggle refuses
to re-enable it, because the toggle is not where permissions are shown.

Examples live in `examples/plugins/` and are **validated by two Rust tests**:
one parses every manifest, the other builds each installer's real sandbox and
resolves every step path through it. The second is the one with teeth — parsing
only proves the JSON is well-formed, while a manifest whose grants and step
paths disagree is the mistake an author copying the example would inherit.

## Settings: the two halves

| | App settings | Account settings |
| --- | --- | --- |
| Stored | `settings.json`, this machine | `UserSettings` on the website |
| Written by | `settings.rs` | `PATCH /api/app/v1/me` |
| Read by | `useSettings().app` | `useSettings().user` |
| Contains | Theme, scale, game directories, logging, latency, plugin prompts | Notifications, locale, timezone |
| Can fail | No | Yes — network, auth |

The test for which side something belongs on: **would this be wrong to apply on
a different machine?** A game directory would be. A notification preference
would not.

## Logging

`Settings → Logging` reads an append-only JSONL audit trail. It is a **record of
what the app did on the user's behalf**, not diagnostics — `tracing` handles
those and goes to stderr.

- `Security` level entries are written **even when the user turns logging off**.
  A switch that lets a plugin ask the user to stop watching is not a control.
- Clearing the log writes the "log cleared" entry **after** the truncation.
- Every plugin step, every download URL, every sandbox refusal, every permission
  grant lands here.

Write with the `audit!` macro. Never put a token or a credential in `data`.

## Cross-platform

One layout, keyed on the **window**, not the OS:

- **≥ 768px** — sidebar rail
- **< 768px** — bottom tab bar

A desktop window dragged narrow gets the phone layout, which is both correct and
the only way to test the mobile shell without a device.

- Safe-area insets come from `env(safe-area-inset-*)`, wired through
  `--safe-top` / `--safe-bottom` in `app.css`. Use them on anything fixed to an
  edge.
- **HashRouter**, because a Tauri build is a static bundle with no server to
  answer `/view/mod/12` on reload.
- The build target is `safari13` (`chrome105` on Windows) — the floor for the
  oldest supported macOS and Android webviews.
- `keyring` is desktop-only; `secure.rs` falls back to the app's private
  container on mobile, which the OS isolates and encrypts.
- `tauri-plugin-deep-link` is registered on desktop only. Mobile declares
  `tmc://` in the platform manifests generated by `tauri android init` /
  `tauri ios init`.

## Commands

```bash
npm install                # frontend deps (needs GitHub Packages auth for @modcommunity)
npm run dev                # vite only, no Rust — fastest loop for UI work
npm run desktop            # tauri dev
npm run desktop:build      # tauri build

npm run android:init       # once, generates gen/android
npm run android            # tauri android dev
npm run ios:init           # once, macOS only
npm run ios                # tauri ios dev

npm run check              # everything. Run before declaring done
npm run check:web          # tsc + eslint
npm run check:rust         # clippy -D warnings, whole workspace
npm run test:rust          # cargo test, whole workspace
npm run test:core          # tmc-core only — no GTK stack needed, runs anywhere
npm run fmt                # prettier + cargo fmt

npm run contract:sync      # re-copy the API contract from website-city
npm run sysroot            # build the local GTK sysroot (see below)
npm run shared:local       # install @modcommunity/shared from ../tmc-global
npm run shared:build       # rebuild it in place
```

`TMC_API_BASE` is read at **compile time** to point a dev build at a local
website (`TMC_API_BASE=https://tmcdev.net:3002 npm run desktop`). It is
deliberately not a runtime setting — a "which server?" field is a phishing
primitive.

### Building on Linux without root

The Tauri target needs `webkit2gtk-4.1`, `libsoup-3.0`, `libgtk-3` and
`libdbus-1` headers. Where you have root:

```bash
sudo apt install pkg-config build-essential \
  libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev \
  libsoup-3.0-dev libgtk-3-dev libdbus-1-dev
```

Where you do not — a shared dev box, a CI image, a container — `npm run sysroot`
builds a private one instead. `apt-get download` needs no privileges and
`dpkg -x` unpacks anywhere:

```bash
npm run sysroot                  # ~35 MB into ~/.local/tmc-sysroot
source ./scripts/linux-env.sh    # point pkg-config and the linker at it
npm run desktop
```

`scripts/cargo.sh` — which `check:rust` and `test:rust` go through — sources
that automatically when the system headers are absent, so the same npm script
works in both situations. The sysroot supplies headers and link stubs only;
runtime libraries still come from the system where it has them.

## Working here

### Type safety

- **`any` is banned** and enforced by eslint. `unknown` plus a parse is always
  available.
- **Every IPC call names a schema.** `invoke` returns whatever the caller claims;
  `src/lib/ipc/call()` makes that claim checked. Importing `invoke` anywhere
  else is an eslint error.
- **Every API response is parsed.** Same reason.
- `noUncheckedIndexedAccess` is on. A lot of this code indexes into
  plugin-supplied data.
- Floating promises are an error — a swallowed IPC rejection is an install that
  silently reported nothing.

### Rust

- `AppError` is the only error type crossing IPC. It serialises as
  `{ code, message }`; `code` is stable, `message` is for humans.
- `AppError::Internal`'s `Display` is a fixed sentence. Detail goes to
  `detail()` and the log, never to the webview.
- Prefer returning an `AppApiResult`-style error over panicking. A panic in a
  command is a `Result<_, String>` the frontend cannot classify.
- New privileged capability → new `#[tauri::command]` in `commands.rs`, with the
  policy check *in* the command, not in the caller.

### Adding a content kind

1. Add it to `ContentKindVals` in website-city's contract; `npm run contract:sync`.
2. Add a `KindSpec` entry in `website-city/src/lib/app-api/content.ts`.
3. Add labels to `KIND_LABELS` / `KIND_GROUPS` in `src/routes/browse.tsx`.

Nothing else — the card and the view page are kind-agnostic by construction.

### Adding a query protocol

1. A module under `core/src/net/query/`, exposing
   `async fn query(addr, timeout, …) -> AppResult<ServerQueryResult>`. Take a
   `SocketAddr` — never a hostname.
2. A variant on `QueryProtocol` matching Prisma's `SpyQueryProtocols` name, and
   an arm in `query()`'s match plus `is_native()`.
3. Parse with `net::reader::Reader`. No indexing, no slicing.
4. Tests: a golden reply, the malformed cases the protocol invites, and the
   truncated-prefix loop. **A parser with no truncation test is not finished.**

Nothing above the protocol module changes — the card, the panel and the graph
all read `ServerQueryResult`.

### Adding a step type

1. A variant on `Step` in `plugins/manifest.rs`.
2. An arm in `Executor::run_step`, resolving every path through
   `sandbox.resolve` and auditing the result.
3. An arm in `step_label` and in `required_roots`.
4. A test. **If the step can write, test that it cannot write outside the jail.**

## Gotchas

- **A query string carries no types, so `parseQuery` does not guess.** Values
  arrive as strings and the contract coerces per field (`z.coerce.number()`,
  `QueryBool`). Guessing from shape is the obvious approach and it silently
  broke `?search=2024`, `?search=true` and — because every cursor is a numeric
  id — the second page of every listing.
- **`z.coerce.boolean()` is `Boolean(value)`**, so the string `"false"` is
  `true`. Use `QueryBool` from the contract.
- **The live-query context's accessors are stable; `version` is the render
  signal.** Anything that puts a context accessor in an effect dependency list
  re-runs that effect on its own result — which, for the server panel, was an
  unbounded query loop against a real game server.
- **`lucide-react` is pinned to `0.468.0`.** `@modcommunity/shared@4.2.0`'s
  `Footer` imports `Github` / `Twitter` / `Facebook`, which lucide removed in
  ~0.475. Unpinning breaks the build with a `MISSING_EXPORT`. Fix belongs in
  tmc-global; until then, the pin stays.
- **Vite 8 minifies with oxc.** Setting `minify: 'esbuild'` demands esbuild as a
  separate install.
- **`@source` in `app.css` is required.** Tailwind ignores `node_modules`, so
  without it the shared components' classes are never generated.
- **`Path::join` with an absolute argument discards the base.** That single
  behaviour is why `join_relative` exists and why nothing in the plugin path
  should ever call `join` on untrusted input.
- **A tRPC mutation cannot set a cookie on a streamed response.** Relevant when
  touching the website's device-approval router — see website-city's
  `src/trpc/react.tsx`.
- **`generate_handler!` needs full module paths.** `commands::auth::auth_begin`,
  not a `pub use` of it — the macro also needs the hidden `__cmd__*` items
  `#[tauri::command]` generates, and those do not travel through a re-export.
- **`tauri.conf.json`'s feature allowlist and `Cargo.toml`'s `tauri` features
  must agree**, or the build script refuses with a message about
  `protocol-asset`. The asset protocol is deliberately off in both.
- **`keyring` pulls `libdbus-sys` on Linux**, which needs a dev package the
  security tests have no business requiring. It is behind `tmc-core`'s
  `os-keyring` feature, off by default and enabled by the app crate — which is
  why `npm run test:core` works on a bare machine.
- **A `///` block followed by a blank line documents the NEXT item.** Module
  headers in this crate are `//!` at the top of the file; clippy's
  `empty_line_after_doc_comments` catches the mistake and `check:rust` runs with
  `-D warnings`.
- **`serde(rename_all)` does not rename variant fields.** Enum variants in the
  plugin manifest need `rename_all_fields = "camelCase"` too, or a manifest
  would have to write `max_bytes`.

## Not built yet

Honest list, so nothing here reads as finished when it is not:

- **Auto-updates.** `autoUpdateCheck` is a stored setting with no updater behind
  it. Needs `tauri-plugin-updater` and a signing key.
- **Writes.** The app is read-only against the API — no commenting, rating,
  favouriting or publishing. `api_send` exists and is wired; the screens are not.
- **Plugin distribution.** Plugins install from a local folder. There is no
  registry, and `requireSignedPlugins` has no signature checking behind it yet.
- **Server Live Query plugins in the UI.** `plugin_query_server` works and is
  tested, but the live registry only calls the built-in protocols — a game whose
  protocol needs a plugin falls back to `TCP_ONLY` latency. Wiring the plugin
  path into `commands::servers::run_one` is the remaining step.
- **Protocols left on `TCP_ONLY`.** `FROSTBITE`, `GAMESPY4`, `DISCORD`,
  `TEAMSPEAK3`, `HYTALE_NITRADO`, `GTA_NETWORK`, `GTA_RAGE`, `SCUM`. Each is a
  module under `net/query/` away.
- **A2S compressed split replies.** Reassembly handles the ordinary split
  format; the bzip2-compressed variant (old mods only) is not decoded, so those
  rosters are skipped. The info reply, which is what the browser renders, is
  unaffected.
- **Dependency resolution.** `ContentDetail.dependencies` is always `[]`.
- **Offline cache.** React Query is memory-only; a cold launch offline shows
  nothing.
- **No end-to-end test against a real game server.** The protocol parsers are
  covered by golden-reply and fuzz-shaped unit tests; the socket paths above
  them have been exercised only against the bounds checks, not a live box.
