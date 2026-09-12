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
- **Sandboxes** — named mod profiles, deployed by hard link, symbolic link or
  direct copy, with the game folder recoverable either way
- **Real latency pings** measured from the user's own device
- **Live server queries** over game-specific protocols
- **A download queue** with pause, resume, priorities and bandwidth limits
- **RCON**, with the passwords encrypted on the device and never uploaded
- **Finding the games** — Steam, Epic, GOG and the rest, read from what they
  already wrote down
- **Local-first settings**, kept deliberately separate from account settings
- **A catalogue of every game**, with a Play button on the ones that can be
  started from here — in a window of their own, or from the copy installed on
  this machine with a sandbox's mods in front of it
- **Play sessions** — what is running, how long it ran, and what it printed
  before it crashed

It is **not** a wrapper around the website. There is no landing page, no
marketing header, no footer. It opens on a browser and stays there.

## Sibling repositories

| Repo | Relationship |
| --- | --- |
| `../website-city` | The Next.js site. **Owns the API this app speaks** (`/api/app/v1`) and the auth flow. Changes there usually need a matching change here. |
| `../spy` | The Go scanner that populates every server row. **The authority on query ports and protocol quirks** — when this app and the site disagree about how to reach a server, `spy` is what settles it. |
| `../tmc-global` | `@modcommunity/shared` — design tokens and a few UI primitives, consumed from GitHub Packages. `npm run shared:local` swaps in the sibling checkout. |
| `../website-processing` | Astro landing site. No relationship to this app beyond sharing the design system. |

**This repository is public and GPL-3.0-only** (`LICENSE`; every crate declares
it, and so does `package.json`). Two consequences worth holding while writing
code here: a comment is published, and a vulnerability report arrives from a
stranger. [`SECURITY.md`](SECURITY.md) is the front door for the second — it
states the threat model the architecture below is built around, and lists the
things that look like holes and are deliberate, so that a report can be aimed
past them.

## Architecture, and the one rule everything follows

> **Credentials and the filesystem live in Rust. The webview gets data.**

There is no `fetch` in the frontend and no `get_token` command. A mod
description is untrusted text rendered inside a webview that can call `invoke`;
the defence is that there is nothing worth stealing on that side of the bridge.
Every layer below assumes an attacker has already achieved script execution in
the webview and asks what they can reach.

**One command takes a path from the webview**: `fs_list_dirs`, which backs the
app's own folder picker and returns directory *names* only — no files, no reads,
no writes. Its module header states exactly what that widens. Nothing else in
`commands/` accepts a path.

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
  ├── secure.rs   refresh token + named secrets → OS credential store
  ├── crypto.rs   XChaCha20-Poly1305 for what has to be kept, not hashed
  ├── api.rs      the ONLY HTTP client; attaches the bearer, handles refresh
  ├── net/        address guard, transports, latency history, game protocols
  ├── rcon/       Source and Frostbite consoles; the one deliberate SSRF gap
  ├── download/   the queue: priorities, resume, rate limits
  ├── detect/     where the launchers say the games are
  ├── plugins/    manifest → jail → step executor, all declarative
  ├── deploy/     merge tree → link/copy → ledger; how mods reach the game
  ├── library/    subscriptions, sandboxes, staging, deployment
  ├── launch.rs   the only place a launch is RESOLVED
  ├── session.rs  the games that are running, and how long they ran for
  ├── deeplink.rs what a `tmc://` link may mean, which is "show a page"
  └── logging.rs  the audit trail Settings → Logging reads
  │
  ├──► website-city  /api/app/v1   (catalogue, auth, sandboxes)
  ├──► game servers directly       (live queries, latency, RCON)
  └──► wherever a mod's files are  (the download queue)

tmc-usvfs (src-tauri/usvfs/)       ← the virtual filesystem, Windows-only halves
  ├── tree.rs     the merged view a game is shown instead of its own folder
  ├── shm.rs      publishing that tree where an injected process can map it
  ├── hooks.rs    the import-table patch, inside somebody else's game
  └── inject.rs   suspended launch + remote LoadLibrary
```

**`tmc-usvfs` is a third crate rather than a module** for one concrete reason:
it is the only part of the tree that can be `cargo check`ed for Windows from a
Linux box. `tmc-core` cannot — bundled SQLite needs a C cross-compiler — so
code that reaches into another process would otherwise be verified by reading
it. The split is what makes `cargo check -p tmc-usvfs --target
x86_64-pc-windows-gnu` possible, and that command has already caught four real
bugs.

**The workspace split is load-bearing.** `tmc-core` holds the plugin jail,
the protocol parsers and the token lifecycle — the three things you most want to
compile and fuzz on a CI runner with no display stack, no WebKit and no dbus.
`cargo test -p tmc-core` needs none of them. Anything you add that does not
genuinely need a window belongs on that side of the line.

### Source map

**`src-tauri/core/src/` — `tmc-core`**

| File | Owns |
| --- | --- |
| `api.rs` | HTTP. `api_base()` — compile-time, or `TMC_API_BASE` in a debug build. Never a setting |
| `auth.rs` | PKCE, device-grant state, in-memory access token |
| `secure.rs` | Keychain / Credential Manager / Secret Service, file fallback on mobile. Keyed per API base |
| `settings.rs` | App-local settings (`settings.json`), clamped on read |
| `anchor.rs` | **What a jail anchor may be.** Guards `gameDirs` / `downloadDir` |
| `crypto.rs` | The device key, and what it does and does not buy |
| `deeplink.rs` | The closed list of what a `tmc://` link may ask for |
| `logging.rs` | Append-only JSONL audit log + `audit!` macro |
| `net/addr.rs` | **The public-address guard.** Resolve once, connect to that |
| `net/transport.rs` | Bounded UDP/TCP exchanges — every read has a deadline and a cap |
| `net/reader.rs` | The non-panicking byte cursor every parser is built on |
| `net/ping.rs` | TCP connect timing, for games with no protocol |
| `net/latency.rs` | Rolling per-server history, bounded on both axes |
| `net/query/` | The game protocols — see below |
| `plugins/manifest.rs` | The manifest format and its validation |
| `plugins/jail.rs` | **The path jail.** Called a jail, not a sandbox — see below |
| `plugins/steps.rs` | The install/uninstall executor |
| `plugins/apps.rs` | Per-game rules: where mods go, how to launch, sandbox presets, where its settings live |
| `plugins/query.rs` | The declarative parser for Server Live Query plugins |
| `plugins/config.rs` | **A game's own settings files.** Listing, reading and replacing what its `config.json` declares |
| `plugins/theme.rs` | Theme token validation |
| `plugins/registry.rs` | Installed plugins, approvals, fingerprint drift, the signature gate |
| `plugins/signature.rs` | **Who vouched for a plugin.** Ed25519 over the canonical manifest, against the user's own trust store |
| `deploy/merge.rs` | The virtual tree: who wins each path, and what conflicts |
| `deploy/link.rs` | One syscall each, and the platform reason it might fail |
| `deploy/ledger.rs` | What landed, what it displaced, and how to undo both |
| `deploy/engine.rs` | Strategy selection, the diff against last time, placement |
| `library/db.rs` | The device's SQLite store — subscriptions, sandboxes, queue, RCON |
| `library/sync.rs` | Reconciling against the account, on a watermark |
| `library/install.rs` | Materialising one subscription into the main game folder |
| `library/sandbox.rs` | Sandboxes: environment, strategy, options, load order |
| `library/deploy.rs` | Staging a mod into a sandbox, then deploying the sandbox |
| `download/rate.rs` | The token bucket behind both bandwidth limits |
| `download/mod.rs` | The queue itself |
| `download/store.rs` | The queue, across a restart |
| `detect/steam.rs` | `libraryfolders.vdf`, then every `appmanifest_*.acf` |
| `detect/epic.rs` | Epic's `.item` manifests, and Heroic/Legendary on Linux |
| `detect/gog.rs` | Galaxy's SQLite, read-only and `immutable=1` |
| `detect/folders.rs` | Xbox, Ubisoft, EA, Battle.net, and a game's own hints |
| `detect/vdf.rs` | Valve KeyValues, bounded on every axis |
| `rcon/source.rs` | Valve's protocol, which is also Minecraft's |
| `rcon/frostbite.rs` | Battlefield's, with `login.hashed` |
| `rcon/store.rs` | Saved servers. Passwords encrypted; none of it leaves the device |
| `launch.rs` | The only place a launch is RESOLVED. Produces a plan; runs nothing |
| `games/` | **Games TMC publishes, installed here.** Resolve a build for THIS machine, download it through the queue, unpack it, keep it current, plan its launch |
| `version.rs` | Comparing two versions, in the one place that does it. `1.10.0` is after `1.9.0` |
| `session.rs` | **Games that are running.** Keeps the `Child`, times the session, captures its output. What can be known about a launch, and what cannot |
| `detect/walk.rs` | The bounded walk over folders the USER ticked, for what no launcher wrote down |
| `library/share.rs` | A sandbox as a pasteable code. Item ids, never files, and nothing about this machine |
| `local/metadata.rs` | **`tmc.json`** — what an archive claims about where it came from. A hint, never a permission |
| `local/store.rs` | Imported mods on disk: assembling one, laying it out under the game folder, moving it, removing it |
| `local/adopt.rs` | What is already in a game folder that the ledger, a subscription and a previous adoption cannot account for |
| `local/vault.rs` | Paths this process found, named from the webview by token. What that buys, and what it does not |
| `plugins/managers.rs` | **Another mod manager, read.** Where it keeps its mods, declaratively — the fourth plugin type |

**`src-tauri/usvfs/src/` — `tmc-usvfs`**

| File | Owns |
| --- | --- |
| `tree.rs` | The merged view: resolution, directory listings, case rules. Platform-independent, tested everywhere |
| `shm.rs` | The blob format, published atomically and parsed defensively — it is read inside a game |
| `hooks.rs` | IAT patching and the redirecting `CreateFileW`. Windows only |
| `inject.rs` | `CREATE_SUSPENDED` + `CreateRemoteThread(LoadLibraryW)`, and the quoting `CommandLineToArgvW` demands |

**`src-tauri/src/` — `tmc-app`**

| File | Owns |
| --- | --- |
| `lib.rs` | Builder, plugin registration, the command list |
| `commands/` | The whole IPC surface. Nothing privileged happens outside it |
| `commands/fs.rs` | Directory listing for the app's folder picker. Names only, never contents |
| `commands/sandbox.rs` | Sandboxes, staging, deploying. Never takes a directory |
| `commands/downloads.rs` | The queue. No command here takes a URL or a destination — `download_release` takes ids |
| `commands/detect.rs` | Scan, and separately apply. A scan configures nothing |
| `commands/rcon.rs` | Consoles. No command returns a password |
| `commands/library.rs` | Sync, install, uninstall, launch |
| `commands/play.rs` | The two ways a game starts that the SERVER resolves. Names IDS, never a loader URL or a connect link |
| `commands/games.rs` | Installing, updating and starting a TMC build. Names an APP ID — never a URL, never a path |
| `commands/config.rs` | The game-settings editor's three commands, behind one setting |
| `commands/sessions.rs` | What is running, what ran, and the log it printed |
| `commands/import.rs` | Dropped files, adopted folders, other managers. Names TOKENS, never paths |
| `spawn.rs` | The only place in the app that starts a process. Which mechanism is the PLAN's decision, not the caller's |
| `state.rs` | `AppState`, assembled once — shared locks and caches depend on that |
| `paths.rs` | Every path, from Tauri's resolver — never `$HOME` |

**`src/`**

| Path | Owns |
| --- | --- |
| `lib/ipc/` | `call()` + schemas + `ipc.*` + `messageOf`. The only place `invoke` is imported |
| `lib/api/contract.ts` | **Mirror** of website-city's contract. `npm run contract:sync` |
| `lib/api/client.ts` | `api.*`, every response zod-parsed |
| `lib/api/query-params.ts` | The ONLY place a browse URL's encoding is decoded, and its inverse |
| `lib/api/labels.ts` | How a game is named on screen — always its full name |
| `lib/api/offline-cache.ts` | What a cold launch shows before the network answers. Allow-listed, public data only |
| `lib/api/env.ts` | Which site this build talks to, for display and for the site's own links |
| `lib/auth/provider.tsx` | Login state and the poll loop |
| `lib/settings/provider.tsx` | App settings + account settings, kept apart |
| `lib/hooks/use-app-icons.ts` | Game artwork for screens whose data is local |
| `lib/hooks/use-breakpoint.ts` | Layout decisions, keyed on the window |
| `lib/hooks/use-platform.ts` | The few decisions that genuinely are per-OS, not per-window |
| `lib/hooks/use-live-query.tsx` | The live-server registry: one timer, one batch |
| `lib/hooks/use-deep-link.tsx` | Turns a `tmc://` link into a route. Never into an action |
| `lib/downloads/provider.tsx` | The queue, pushed from Rust and coalesced |
| `lib/library/provider.tsx` | The subscription sync loop |
| `lib/external.ts` | Which content kinds are handed to the system browser |
| `components/shell.tsx` | Sidebar ≥768px, bottom tabs below |
| `components/titlebar.tsx` | The app's own window frame — see "Cross-platform" |
| `components/update-banner.tsx` | "There is a newer version", and nothing more. Never installs anything |
| `components/folder-picker.tsx` | The in-app folder chooser, over `commands/fs.rs` |
| `components/game-icon.tsx` | A game's artwork, with a deterministic initials fallback |
| `components/item-thumb.tsx` | An item's cover in a list, from the LOCAL library row |
| `components/launch-dialog.tsx` | The confirmation shown before anything starts, shared by both launchers |
| `components/markdown.tsx` | The safe renderer for untrusted bodies |
| `components/select.tsx` | **The app's own dropdown.** A native `<select>`'s popup cannot be themed |
| `components/speed-graph.tsx` | A download's recent speed, hand-rolled SVG |
| `components/gallery.tsx` | Screenshots, and the lightbox behind them |
| `components/reviews.tsx` | The review list and its distribution bars |
| `components/report-button.tsx` | Reporting, into the website's own moderation queue |
| `components/latency-graph.tsx` | Sparkline + full chart + the latency ladder, hand-rolled SVG |
| `components/server-live.tsx` | The live strip on a server card |
| `components/server-table.tsx` | The server browser's default view: table, expandable rows |
| `components/browse-filters.tsx` | The filter panel: collapsible groups, kind-aware, URL-backed |
| `components/server-panel.tsx` | The live panel on a server's page |
| `routes/apps.tsx` | **The front door.** Every game and app, and which of them can be started here |
| `routes/library.tsx` | Three views: the games on this machine, the games TMC published here, and what the account subscribed to |
| `components/library-tmc.tsx` | TMC's own games: install, update, auto-update, remove |
| `routes/join.tsx` | Where a `tmc://play/<host>:<port>` link lands. Shows what is there; joins nothing on its own |
| `components/library-games.tsx` | Installed games, their sandboxes, play time, and what is running |
| `components/library-content.tsx` | The subscription list, and why each row is or is not installed |
| `components/play-dialog.tsx` | **The launcher.** The four ways to start a game, and which are possible here |
| `components/scan-dialog.tsx` | Finding games: read the launchers, or walk folders the user ticked |
| `components/sandbox-editor.tsx` | Create, edit and delete one sandbox |
| `components/sandbox-share.tsx` | Export a sandbox to a code; import one from a code |
| `components/session-history.tsx` | Recent launches, and the log a crashed game printed |
| `components/config-editor.tsx` | Editing the settings files a game declares |
| `components/quick-install.tsx` | One-click install of a mod or asset INTO a sandbox |
| `components/drop-import.tsx` | The drop overlay, above the router. Listens to Rust, never to the DOM |
| `components/import-dialog.tsx` | What is about to be imported, where it will land, and the three fields worth editing |
| `components/local-mods.tsx` | Imported mods: the list, the game-folder scan, the other-manager import |
| `routes/sandboxes.tsx` | The mod manager: what is in a sandbox, and whether it is applied |
| `routes/downloads.tsx` | The queue, and everything to do when one is stuck |
| `routes/rcon.tsx` | The server console |
| `routes/` | Browse, view, library, installs, account, settings panes |

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
(GET + PATCH), `/browse`, `/content/:kind/:id`, `/facets`, `/apps`,
`/play/launch`, `/version`.

**`/apps` exists because the website has no page that could answer it.** Its
chrome is built around one chosen game, so "what games are there?" is a question
it answers by making you choose first; the app opens on a flat grid across the
whole catalogue and that IS its front door. The endpoint carries the play
vocabulary (`directPlay`, the launch options, which modes an app supports) that
`/facets` has no reason to know about, and per-app counts from three `groupBy`s
over the page's ids rather than a relation `_count` per row.

**`/play/launch` resolves one launch, server-side, and answers `null` for every
refusal.** The app never decides whether a game can be started by reading
columns off a catalogue row — it asks — so the button that is drawn and the
launch that happens agree by construction. `directPlay` is enforced there as
well as hidden in the UI, because a client that ignores the field must not get a
serverless launch out of it.

### The browse filters mirror the website's, deliberately

`BrowseQuerySchema` is a copy of `ServerBrowserPublicGetsInput` plus the
mod/asset equivalents, field for field, so a filter someone used on the site is
reachable here under the same name and with the same meaning. Where the two
could disagree, **the website wins** — `hideFull` drops servers declaring no
player limit because the site's SQL does, even though that is arguably wrong,
because a filter that quietly returns more than the site did is worse than one
that matches.

Two things this endpoint pins that the app cannot ask for, both server-decided
exactly as they are on the website:

- **`hasPenalties: false`.** A live penalty hides an item everywhere. Missing
  here, the app was the one surface still listing `SPY_FLAG`'d servers — the
  redirect farms and fake-player-count boxes. Applied to `/browse` *and*
  `/content/:kind/:id`, since reaching a row by id never passes a list filter.
- **`SRV_STALE_ONLINE_SEC`.** `online` has no expiry, so a server that stopped
  being scanned asserts its last reading forever.

Not every declared field is implemented: **`timeRange` is accepted and
ignored**, on this endpoint and always has been. The website windows aggregates
in-process over a capped candidate set, which an infinite-scroll cursor cannot
do — the ranking moves as the window moves, so rows repeat and vanish between
pages. `downloads` falls back to `createdAt` for the same reason.

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
`server.query`, along with `timeoutMs` and two port fields.

### Which port gets queried, and why not the other two

**Explicit `queryPort` if the row has one, otherwise the GAME port.** That is
the whole rule, and the authority for it is the scanner that writes these rows —
`spy/internal/scanners/server.go`:

```go
qPort := port
if srv.PortQuery != nil && *srv.PortQuery > 0 {
    qPort = *srv.PortQuery
}
```

The two fields the API also sends are **not** inputs to that decision, and
treating them as such is what made every Source server look dead:

- **`portOffset`** (`App.srvGamePortQueryOffset`) is dead config. Grep the whole
  scanner for it and you get one hit: the struct field it deserialises into.
  Nothing reads it. A game whose App row carries a stale non-zero offset is
  scanned by the site on its game port, so probing `game + offset` from here is
  a port nobody is listening on — silence, which renders as a timeout.
- **`swapGamePort`** (`App.srvSwapGamePort`) is post-scan bookkeeping, not port
  selection. In `spy/internal/protocols/a2s.go` it means "once A2S_INFO comes
  back, read the true game port out of the extended-info block and swap the
  stored `port`/`portQuery`". Reading it as "query the game port" gave the right
  number often enough to look correct, which is worse than being wrong outright.

The only per-protocol exceptions are the scanner's own hardcoded ones, and they
apply only when the row has no explicit query port: **FiveM → 30120**,
**SCUM → game + 2**, **Frostbite → game + 22000**. `resolve_port` carries all
three with the Go file named. Frostbite's also declines the arithmetic above a
game port of 43535 rather than wrapping past 65535 — a wrapped port is not just
wrong, it probes a stranger's unrelated service on a low port number.

Getting this wrong is still the single most common reason a live server shows as
dead — which is why the rule is copied rather than invented.

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
| `GAMESPY4` | Games listing v4 | Identical game-server query to v3; shares its implementation, tagged separately |
| `SAMP` | SA-MP, open.mp | IPv4 only, by protocol design |
| `FIVEM` | GTA V, RedM | HTTP `/dynamic.json` + `/info.json` |
| `FROSTBITE` | BF3, BF4, Bad Company 2, Hardline | R-CON over TCP on game + 22000; roster columns keyed by tag name |
| `TEAMSPEAK3` | TeamSpeak 3 | ServerQuery on a fixed 10011; the game port SELECTS a virtual server rather than being connected to |
| `HYTALE_NITRADO` | Hytale on Nitrado | HTTPS status document on game + 3, self-signed |

### The four that stay on `TCP_ONLY`, and why it is not laziness

`DISCORD`, `SCUM`, `GTA_NETWORK` and `GTA_RAGE` are the last unimplemented
entries, and each is unimplementable *here* for the same reason:

| Protocol | What `spy` actually asks |
| --- | --- |
| `SCUM` | `api.hellbz.de` |
| `GTA_NETWORK` | `multiplayerhosting.info` |
| `GTA_RAGE` | the RAGE:MP master list at `cdn.rage.mp` |
| `DISCORD` | nothing — the handler is a stub |

None of them talks to the server. That is fine for a scanner reading a list
once on everybody's behalf, and wrong for this app three times over: **the
latency would describe that third party's hosting**, identical for every row
(`spy` says so itself in three comments, and records no latency for any of
them); it would **tell a third party every server a user scrolls past**; and on
a phone it would be one HTTPS request per row per tick. So they measure a real
TCP handshake to the real box instead, and their player counts come from the
API — which got them from the scanner, which read those lists once.

`net/query/hytale.rs`'s module header carries this, because it is the one of
the five that DID qualify.

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
- **A split A2S reply's `total` and `number` are two SEPARATE bytes.** GoldSrc
  packs them into one (index high nibble, total low) and has no split-size
  field; Orange Box and later use two bytes plus a size. Reading the packed form
  while also consuming a size field parses fragment 0 plausibly — `total = 2,
  index = 0` — and decodes every LATER fragment to index 0 as well, so they
  overwrite each other and every split roster comes out "incomplete". The
  authority is `go-a2s`, which is the library `spy` queries A2S with.

### The registry

`LiveQueryProvider` holds one timer and one registry. Cards register via an
`IntersectionObserver` while on screen; every tick sends the whole visible set
to Rust in a single `query_servers` call, which applies the user's concurrency
cap. Fifty cards each polling would be fifty timers, fifty IPC round trips and
fifty simultaneous sockets — which a phone's radio and most home routers handle
badly.

Deregistration has a 5-second grace so a flick-scroll does not cancel work
already in flight, and polling stops entirely while the window is hidden.

**The cadence is the user's**: `latencyIntervalMs`, one second by default, set
under Settings → App → Servers and clamped to 250ms–5min *in Rust*
(`LATENCY_INTERVAL_MS_MIN`/`MAX`) because it decides how often a third party's
game server is sent a datagram. The provider publishes it as `intervalMs` so
the single-server panel polls on the same number rather than on a constant of
its own. A tick that arrives while one is in flight is remembered, not stacked,
so a fast interval degrades to "as fast as the batch completes" rather than to
overlapping batches.

Three properties of the scheduling that are easy to undo by accident:

- **A row's FIRST probe comes from `watch`'s kick, not from the interval.** The
  provider mounts at app start, so its warm-up has long since fired by the time
  anyone opens the browser; keying first measurements off the refresh interval
  left a fresh screenful showing ellipses, which reads as the feature being
  broken rather than slow. It is also what keeps a slow interval usable: a row
  is measured a quarter-second after it scrolls into view whatever the setting.
- **The batch is sorted unmeasured-first.** Rust caps a batch at 64 and *drops*
  the rest, so an unsorted batch during a fast scroll spends the cap on rows
  that already have a number and strands the ones that do not.
- **A failed `query_servers` call is applied as a failed probe for every row in
  it.** Otherwise a broken IPC boundary — an unregistered command, a drifted
  schema — is indistinguishable on screen from a browser full of slow servers.

### How a latency reading is drawn

| State | Shown | Meaning |
| --- | --- | --- |
| measured | `32ms`, coloured | <90 green · <150 yellow · <200 orange · ≥200 red-orange |
| timeout | `TO` in red | A probe went out and nothing came back |
| waiting | `…` | No probe has resolved yet |
| none | `—` | The owner hid the address; there is nothing to probe |

The ladder lives in `LATENCY_TIERS` / `latencyTone` and the four states in
`latencyState`, both in one place so the card, the table and the server panel
cannot disagree. `TO` exists because "500ms" and "no answer" are different facts
and a dash for both is how a server browser earns a reputation for lying.

### Latency history

`net::latency::LatencyStore` keeps up to 120 samples for up to 512 servers, in
memory only — the series is interesting while the browser is open, is entirely
reconstructible by re-measuring, and persisting it would mean a disk write per
scroll tick. That is a COUNT, not a window — at the default one-second cadence
it is two minutes of history, and proportionally more at a slower one.
**Failed probes are recorded as `None`, not dropped**: an intermittently-dead
server must not graph as perfectly stable.

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

### Four types

| Type | Declares | Runtime |
| --- | --- | --- |
| **Installer** | Ordered `install` / `uninstall` steps | `plugins/steps.rs` |
| **Server Live Query** | A hex request + a field list | `plugins/query.rs` |
| **Theme** | A map of CSS custom properties | `plugins/theme.rs` |
| **Manager** | Where another mod manager keeps its mods | `plugins/managers.rs` |

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
| `is_public()` in `net/addr.rs` | SSRF into the LAN or a cloud metadata endpoint |
| Theme token + colour allow-list | `url()` beacons, `display:none` on the uninstall button |
| Grants as filters over a root-relative path | Two grants for one root merging into the widest |
| Manifest fingerprint | An update silently widening permissions |
| Ed25519 signature over the canonical manifest | An anonymous bundle, when `requireSignedPlugins` is on |

### Approval

`plugin_inspect` returns a fingerprint (SHA-256 of the canonical manifest), the
full permission list and the signature state; `plugin_approve` takes the
fingerprint back. That closes the window between the user reading the
permissions and clicking approve.

`Registry::rescan` runs every launch and re-hashes each installed manifest. A
drifted plugin is **disabled** and flagged `needsReapproval`; the toggle refuses
to re-enable it, because the toggle is not where permissions are shown.

### Signatures

The fingerprint answers *"is this the same plugin the user approved?"*. It
cannot answer *"did anybody the user trusts write it?"* — a hash of whatever is
in the folder is perfectly good for a bundle that anything at all dropped there.
`plugins/signature.rs` is the second question: Ed25519 over the manifest's
**canonical bytes**, the same serialisation the fingerprint hashes, so a bundle
reformatted in transit keeps its signature and a bundle whose declared
permissions changed loses it. The detached signature is `plugin.sig`, hex.

**Three states, not two.** `Unsigned`, `Trusted(keyId)` and `Untrusted` — the
last being a signature that matches nothing the user trusts, which is either a
publisher whose key has not been added yet or a tampered bundle. Folding it into
"unsigned" is how the interesting case disappears, so the UI and the refusal
messages keep them apart.

**The trust store is the user's, and no key is compiled in.** Keys live in
`plugins/trusted-keys.json`, and TMC's own publishing key will be a row in it
like anybody else's — the honest shape while there is no registry to distribute
plugins through, and still correct once there is. Bundling a key nobody signs
with would make `requireSignedPlugins` a switch that refuses everything.

**`requireSignedPlugins` now does something.** It was a stored boolean with
nothing behind it, which is worse than not having it — somebody turns it on,
believes they are protected, and installs accordingly. With it on, `active()`
refuses an unsigned or untrusted plugin before the executor can be reached.

Two properties of the gate that are easy to lose:

  * **It re-verifies rather than reading `record.signature`.** The stored verdict
    is a claim about a check that ran at some earlier time under a trust store
    that may since have changed, and this is the last gate before something
    writes to a game folder.
  * **Trusting a key rescans immediately.** "Add the publisher's key" is advice
    that has to work when followed, not after a restart — and removing a key has
    to stop those plugins at once, which is the entire reason to remove one.

**Producing one** is `cargo run -p tmc-core --example plugin-sign` — `keygen`,
`sign <dir> <secret>`, `verify <dir> <public>`. It exists so the feature is
usable without reimplementing it from a doc comment, which would mean guessing
at the one detail that matters. Its two properties are worth checking by hand
after any change to `Manifest`: a manifest minified and key-sorted still
verifies, and one extra `gameDir` write grant does not.

What a signature buys is bounded and stated in the module header: it says a
holder of that key produced *these* permissions and *these* steps. It says
nothing about whether they are safe. A signed plugin is confined by exactly the
same jail as an unsigned one.

Examples live in `examples/plugins/` and are **validated by two Rust tests**:
one parses every manifest, the other builds each installer's real jail and
resolves every step path through it. The second is the one with teeth — parsing
only proves the JSON is well-formed, while a manifest whose grants and step
paths disagree is the mistake an author copying the example would inherit.

### App plugins: the rules for one game

Distinct from the bundles above, and deliberately:

| | Registry plugin | App plugin |
| --- | --- | --- |
| Where | `plugins/<id>/plugin.json` | `plugins/app/<slug>/*.json\|yaml` |
| Identified by | a reverse-DNS id the author picks | the GAME it handles |
| Answers | "what can this plugin do?" | "where do this game's mods go, and how do its sandboxes work?" |

A bundle is something a user installed; an app plugin is a *rule for a game*,
the app ships one per supported game, and there are dozens of tiny ones. The
directory name is the game's **URL slug**, lower-cased. `disabled/` is skipped
at any depth, which is the whole mechanism for turning a rule off without
deleting it.

| File | Declares |
| --- | --- |
| `manage_mod.json` / `manage_asset.yaml` | Install and uninstall steps for one content kind |
| `launch.json` | How to start the game |
| `sandbox.json` | Deployment strategies, presets, the game's own options, and detection hints |
| `config.json` | Where this game keeps the files a player edits by hand |

**The safety model is unchanged.** Each file compiles to a synthetic `Manifest`
and runs through the same jail and the same executor, so it can express nothing
a registry plugin cannot. What it adds is only the *selection* — which rule
applies to which game, kind and file.

#### Where they come from, and why they are compiled in

Two sources, and `AppPlugins::resolve` is the only thing that reads both:

| Source | Holds |
| --- | --- |
| Compiled into the binary from `<repo>/plugins/app/` | the games the app ships support for |
| `<app data>/plugins/app/` | whatever the user added |

**The shipped half is embedded by `core/build.rs`, not shipped beside the
binary.** A Tauri resource is a separate file next to the exe — which the
portable Windows build does not carry, and neither does an AppImage moved out of
its directory. The first release built from this tree shipped no rules at all,
and what that cost was not "fewer games": **detection found nothing, no game
could be launched, no sandbox knew its strategy and the config editor had
nothing to list — silently**, because every one of those features reads an empty
map perfectly happily. That is far too much to hang on a file being adjacent.

The rules stay files in the repository; the build script only reads them.
`the_compiled_in_rules_match_the_repository` asserts the two agree, so a build
script that quietly stopped embedding one fails a test rather than shipping.

**A slug the user has rules for REPLACES the shipped one** rather than merging
with it. Two authors' rules for one game interleaved by specificity would answer
`choose` with a rule neither of them expected to win.

**A rule never has to name the site it downloads from.** `download_hosts` adds
`api_base()`'s host to every app rule's net allow-list, because `{fileUrl}`
comes from the API and Rust picked the base — an author naming it restates a
constant, and one who gets it wrong writes a rule that cannot download and finds
out when somebody clicks install. That is not hypothetical: the shipped rules
named `moddingcommunity.com`, and a build pointed at the dev site refused every
download because its files come from `tmcdev.net`. The user still sees the host
— `permission_summary` reads the manifest this builds.

#### The games that ship

**Thirty-two, and the canonical list is the README's table** — which game, where
its mods land, and whether anybody has actually run it. Repeating it here would
be a second list to keep in step, and the one in the README is the one a user
reads.

What belongs here instead is the shape of those rules, because it is what a
thirty-third has to fit into:

| Family | Slugs | Mods land in | Default |
| --- | --- | --- | --- |
| Bethesda + script extender | `skyrim`, `skyrim-se`, `skyrim-vr`, `fallout-3`, `fallout-4`, `fallout-new-vegas`, `oblivion`, `starfield`, `morrowind` | `Data/`, plus `Data/<XSE>/Plugins` for the extender's own DLLs | `hardlink` |
| BepInEx | `valheim`, `vrising`, `sotf` | `BepInEx/plugins`, `patchers`, `config` | `hardlink` |
| Source | `tf2`, `gmod` | `tf/custom`, `garrysmod/addons` | `hardlink` |
| Plain `Mods/` folder | `rimworld`, `stardew-valley`, `mount-and-blade-2`, `witcher-3`, `7-days-to-die`, `palworld`, `kerbal-space-program`, `cyberpunk-2077` | one folder, named by the game | `hardlink` |
| Instance model | `minecraft` | `mods`, `resourcepacks`, `shaderpacks`, `datapacks`, `config` | `direct` |
| Documents, not the install | `sims-3`, `sims-4`, `baldurs-gate-3` | the user's documents folder | `direct` |
| Game root | `gtav`, `gta-3`, `gta-4`, `gta-san-andreas`, `gta-vice-city` | `modloader/`, `scripts/`, or the root itself | `direct` |
| Server only | `rust` | `oxide/plugins`, `oxide/config`, `oxide/data` | `direct` |

Four of those rows carry a decision rather than a path:

  * **`antiCheat: "kernel"` is set on `gtav` and `rust`**, which is what forces
    Direct and warns on every other option. Guessing wrong there costs somebody
    their account, not an install.
  * **`rust` is the dedicated SERVER.** EAC is kernel-level, there is no
    client-side Rust modding, and inventing one would be inventing a ban.
  * **The three documents-folder games are why a sandbox's `gameDir` is not
    "where Steam put it".** The install folder holds no mods at all, so a
    sandbox pointed at it deploys perfectly and changes nothing.
  * **`gmod`'s Workshop subscriptions are the game's own business**, not a
    sandbox's — the rule covers `addons/`, `lua/autorun` and the workshop
    cache, and does not try to own what Steam already manages.

**Minecraft is the instance model, deliberately.** A sandbox given its own game
folder gets its own `mods`, `config`, `saves` and `options.txt`, the way
CurseForge, Prism and MultiMC all do it — so `defaultStrategy` is `direct`.
Copying is not laziness there: the launcher is pointed at the folder with
`--workDir` and everything inside is expected to be a real file, loaders resolve
paths, some mods rewrite their own jar on first run, and Windows refuses to make
a symbolic link at all without Developer Mode. Hard links are offered for
somebody running several packs that share mods off one drive.

Everything else defaults to `hardlink`, which is the app's own default and right
when a game folder is shared rather than instanced.

## The library

The device's answer to the account's subscriptions: which of them this machine
holds, which are on disk, and how they got there.

> The Library SCREEN has three views and they are genuinely different nouns:
> **Games** is what somebody else's launcher installed and this app mods,
> **TMC Games** is what this app installed from a build TMC published (see
> "Games TMC publishes"), and **Subscribed** is what the account subscribed to.
> One list holding all three would have to explain, per row, which kind of thing
> it was.

**SQLite, not a JSON file** (`library/db.rs`). Everything else the app persists
— settings, the plugin registry — is a small file rewritten whole, and that is
right for those: read at launch, written on a click, never contended. The
library is not like that. The sync loop writes it on a timer while the UI reads
it on every render and the installer writes single rows mid-install, and a
whole-file rewrite under that pattern loses one writer to another exactly when
somebody is watching a progress bar.

Schema changes are **stepwise** — each migration takes the database from `n-1`
to `n` and runs only if it has not. Re-running one batch happens to be safe
today because every statement is `IF NOT EXISTS`, and stops being safe the first
time a step needs an `ALTER TABLE`.

**It polls** (`library/sync.rs`), because a subscription can be created in a
BROWSER and there is no push channel to an installed desktop app that is
reliable across three desktop platforms, two mobile ones, corporate firewalls
and sleeping laptops. A watermark makes that cheap: each response carries a
`revision`, the next request sends it back, and a device open for an hour has
made sixty requests and transferred one row.

**A full sync still happens** every `FULL_SYNC_EVERY` passes and always on
launch, because a delta cannot express a DELETION — there is nothing left to
poll. Anything the device holds that the server did not send has been
unsubscribed elsewhere.

**An upsert from the server never touches the device-local columns.** That is
what stops a resync forgetting what is installed, which is the single easiest
way to turn a working library into an endless reinstall loop.

**The plan is executed in Rust, not returned to the webview.** A frontend that
decided what to install is one an injected script can talk into installing
something.

## Sandboxes, and how mods reach the game

A **sandbox** is what other managers call a profile (Mod Organizer, Vortex) or
an instance (CurseForge, r2modman): a named set of mods with its own load
order, its own deployment method and its own launch settings.

> The word is why `plugins::jail` is called a jail. It used to be
> `plugins::sandbox::Sandbox` — the path jail — and two `Sandbox` types in one
> crate, one of them a security boundary, is a mistake waiting for somebody to
> reach for the wrong one.

**On the wire a sandbox is an `AppInstall`.** The cloud model predates the word
and `/api/app/v1/installs` is shipped, so renaming the endpoint would 400 every
request from every installed copy of the app. The app says "sandbox"; the API
says "install"; they are the same thing.

### What lives where

By the same test the settings use — *would this be wrong to apply on a
different machine?*

| Fact | Where | Because |
| --- | --- | --- |
| name, mods, order, options, launch flags | the account (`AppInstall`) | signing in on a second machine should reproduce it |
| which folder it deploys into | the device | that path exists on one machine |
| which release of each mod is staged | the device | so does that |
| the deployment ledger | the device | it describes files on one disk |

The cloud half is **optional per sandbox**. `cloudSync` off means it is never
sent anywhere and a reconcile cannot delete it — which is the whole answer to
"I do not want my mod list on your server", and it has to be per-sandbox rather
than global because the useful case is "sync my Minecraft profiles, not the one
I use for testing".

### Two steps, not one

```text
  stage   — run the game's install rule with the sandbox's own staging folder
            standing in for the game directory
  deploy  — mirror every staged file into the real game folder, by whichever
            mechanism the sandbox is set to
```

Splitting them is what makes the whole feature work:

  * **Every install rule written before sandboxes existed still works.** A rule
    that copies to `mods/{fileName}` writes to
    `<staging>/<sandbox>/<mod>/mods/foo.jar`, and deployment puts
    `mods/foo.jar` in the game folder. The rule never learns which strategy is
    in use, and it should not — "where does a Minecraft mod go" and "how do
    files reach the game folder" are different questions.
  * **Switching sandboxes is a link operation, not a download.** Staging
    survives an undeploy.
  * **Nothing half-downloaded is ever in the game folder.**

`pluginData` is scoped per sandbox and per mod (`jail_for_scoped`). Two
sandboxes staging the same mod run identical steps with an identical
`{fileName}`; a shared scratch directory meant the second run's download landed
on the first's, and a sandbox pinned to an older release quietly got the newer
file.

### The four strategies

| Strategy | Game folder | Cost | Fails when |
| --- | --- | --- | --- |
| `direct` | modified | a full copy | never; it is the fallback |
| `hardlink` | link pointers | nothing | staging is on another drive |
| `symlink` | link pointers | nothing | Windows without Developer Mode |
| `usvfs` | untouched | nothing | not Windows, or not built with `usvfs-hooks` |

#### USVFS: the one that deploys nothing

Mod Organizer's approach, in `src-tauri/usvfs/` (`tmc-usvfs`): the mods stay in
staging and the game is *told* they are there, by a DLL injected before its
entry point runs that patches `CreateFileW` and `GetFileAttributesW` in its
import table and answers them from a merged in-memory tree.

```
  app                              game process
  ───                              ────────────
  build the merged tree
  publish it            ──blob──▶
  launch suspended
  inject the hook DLL   ──────▶    DllMain → read the tree, patch the IAT
  resume                ──────▶    every open of a virtual path is answered
                                   from staging
```

**Deploying is not a file operation**, and everything downstream follows from
that:

  * the merged tree is serialised to a blob inside the sandbox's own staging
    folder, and **that publish IS the deploy**;
  * **the ledger comes back empty**, which is the correct record rather than a
    gap — it names files in the game folder and there are none, so `purge`
    correctly does nothing and `verify` correctly reports a healthy folder;
  * **a sandbox switching TO it still purges what the last strategy left**,
    which is the one write to the game folder a virtual deploy performs. Without
    it the user gets both: every file twice, with the virtual copy winning only
    where the tree happens to cover it;
  * **there is no fallback in either direction.** Whether the game folder is
    modified at all is the reason somebody picks this, so silently linking
    instead would dirty a folder chosen to stay clean, and silently going
    virtual would leave a game that was never told about the mods.

**The revision is a content hash, not a counter or a clock.** The injected DLL
compares it to notice that a blob it already read has changed; a counter would
also tick for a redeploy producing an identical tree.

**It is gated twice, at deploy and at launch** (`deploy::usvfs_unavailable`,
`spawn::launch_with_vfs`), on Windows AND on `tmc-core`'s `usvfs-hooks` feature,
which is **off**. The two are halves of one decision: a build with only one on
could publish a tree no launch would carry, or inject with nothing to inject.
The refusals are separate strings because they are separate facts — "this is
Windows-only" can never change for a Mac user, and "this build has it off" can.

Why the gate exists at all is a confidence split, stated in `usvfs/src/lib.rs`:

| Half | State |
| --- | --- |
| `tree` — the merged view, resolution, listings, case rules | **Tested**, on every platform |
| `shm` — publishing and reading the blob atomically | **Tested**, including every malformed input |
| `hooks` — IAT patching, the redirecting `CreateFileW` | **Type-checked against `x86_64-pc-windows-gnu`. Never run against a game.** |
| `inject` — suspended launch, remote `LoadLibrary` | **Type-checked.** Argument quoting and environment building are tested; the launch is not |

`cargo check -p tmc-usvfs --target x86_64-pc-windows-gnu` is how that
type-checking happens and it is worth keeping working — it found four real bugs
the first time it ran, including `IMAGE_NT_HEADERS64` living in
`Diagnostics::Debug` while being gated behind the `Win32_System_SystemInformation`
feature. `tmc-core` cannot be cross-checked the same way (bundled SQLite needs a
C cross-compiler), which is exactly why the injector is its own dependency-light
crate.

**What it cannot do, even when it works:** calls through `GetProcAddress` are
not redirected (the pointer never came from an import table), direct `ntdll`
syscalls are not redirected, writes are not redirected, and every kernel-level
anti-cheat treats injection as an attack — correctly. A game declaring
`antiCheat: "kernel"` in its `sandbox.json` is refused this strategy before it
reaches the engine.

**A URI launch rule and a virtual deploy are refused together.** A game started
through `steam://` is started by Steam, and there is no process of ours to
inject into; the alternative starts the game with none of the sandbox's mods and
reports success, which is the hardest kind of bug to diagnose because the folder
is stock and there is nothing to find.

**Capability is probed, not assumed.** `deploy::link::probe` creates one file
and links it, twice, and reports what worked. Every rules table for this is
wrong somewhere — a Linux box with staging on an exFAT USB drive, a macOS
volume with links disabled, a Windows machine with Developer Mode on.

**Falling back between the two LINK strategies is automatic and reported.
Falling back to copying is not.** Copying is not a worse link, it is a different
decision: it writes gigabytes, it modifies the game's own files, and its failure
mode is a game folder that needs restoring rather than unlinking. A sandbox that
asked for links and can have neither gets an error naming both fixes.

### Three properties that hold across every strategy

  * **Nothing is overwritten.** A file already in the game folder that the app
    did not put there is MOVED to the backup store before its place is taken,
    and a purge puts it back.
  * **Nothing is removed unless it is still ours.** Every removal re-checks the
    file against the ledger row that claims it — a symbolic link must still
    point at its staging file, a hard link must still share its identity, a copy
    must still have its recorded size and mtime. Anything else is left alone and
    reported, so a config the user edited after deploying survives.
  * **Staging is never modified.** Deployment only ever reads from it, which is
    what lets two sandboxes share one downloaded copy of a mod.

### The ledger

`deploy::ledger` is the only record of which files in somebody's game folder
belong to us, and undeploying by rescanning cannot replace it: a rescan can see
that `Data/textures/sky.dds` exists, and cannot see whether the app put it there
or whether it shipped with the game. Guessing wrong either strands a modded file
forever or deletes a base-game asset.

It is written even when a deploy reported errors — a partial deploy put files on
disk, and losing the record of them creates exactly the orphans the ledger
exists to prevent.

### Conflicts

The merge tree decides one winner per path before anything touches the disk, by
priority, with ties broken on the mod key so the answer is stable across runs.
Losers are **reported**, never merged: nothing here understands any game's file
formats, and a manager that silently produces a file neither author wrote is how
"it works for me" bug reports are made.

Sorting happens inside `merge::build`, not in the caller. A tree built from an
unsorted list is wrong in a way that looks right until somebody reorders their
list and nothing changes.

### `sandbox.json`

The PDF blueprint's "strategy matrix", plus the two things it left implicit:

```json
{
  "manifestVersion": 1,
  "sandbox": {
    "deploy": {
      "defaultStrategy": "symlink",
      "supportedStrategies": ["symlink", "hardlink", "direct"],
      "antiCheat": "kernel",
      "modTargets": [{ "type": "loader_mod", "relPath": "mods" }],
      "notes": ["Shown verbatim in the sandbox's settings"]
    },
    "presets": [{ "id": "fabric", "label": "Fabric", "loader": "fabric" }],
    "options": [{ "key": "memoryMb", "label": "Memory", "type": "int", "max": 65536 }],
    "detect": { "steamAppIds": ["271590"], "markers": ["GTA5.exe"] }
  }
}
```

  * **`options`** is a *form description* and nothing more. It cannot express a
    condition, a computation or a dependency between fields — a settings schema
    that can do those is a program, and the plugin model rests on plugins not
    being programs. A game needing one needs a second preset instead.
  * **Values are clamped against the schema on every write**, and a key the
    schema does not declare is dropped. The options reach the cloud and come
    back, so "the server said 900 GB of heap" is answered here rather than by a
    game that will not start.
  * **`antiCheat: "kernel"`** is load-bearing. EAC, BattlEye and Vanguard all
    watch for a game folder whose files are not where they should be, so a game
    declaring it gets Direct as its default and a warning on every other option
    — the cost of guessing wrong is somebody's account, not a failed install.

Options reach a command line only through `optionArgs` in the game's own
`launch.json`. `LaunchOptions` carries them in a flattened `extra` map; an
object or an array contributes nothing, and there is still no shell.

## Playing a game

There are **four** ways a game starts from the app, and which of them is
possible is a fact about THIS machine that no server can answer. That is the
whole reason the app's launcher beats the website's: a browser has exactly one
way to start a game, so the site can offer one button.

| Mode | What it is | Who resolves it |
| --- | --- | --- |
| **native** | A build TMC published and this app unpacked, run as a real process | Entirely local — `games::plan` |
| **sandbox** | The copy installed here, with a profile's mods deployed and its load order applied | Entirely local — `launch::plan` |
| **web** | The app's uploaded JavaScript loader, in a window of its own | `/play/launch` |
| **connect** | The server's own `connectUrl`, handed to the installed game client | The API supplies it; Rust opens it |

`components/play-dialog.tsx` puts the four side by side with the facts that
decide between them — which sandboxes exist, whether a TMC build is installed,
whether a game folder is set, and the latency measured from this device rather
than from a scanner in another hemisphere.

**`native` is offered first when it exists**, because it is the only mode where
the app knows the exact build on disk and can therefore say that it will start.
It is also the only one that does not consult `directPlay`: that flag answers
"does pressing Play with no server start something worth starting", which is a
question about a launch the SITE performs. A binary on this disk opening to its
own menu is the game's business.

**The web player can open full screen**, and the choice is made before the
window exists — `fullscreen` on `play_open_web`, set on the window BUILDER. The
window is a remote page with no IPC, so it cannot ask for this itself; and
calling `set_fullscreen` on a window that is already showing produces a visible
resize plus a WebGL context that has already sized itself to the first geometry.
Godot's canvas reads its size once at boot, so the game would render small
inside a full-screen window with black bars it has no idea about.

### The web player runs on the SITE's origin, and that is the isolation

A game loader is third-party JavaScript. Running it in the app's own webview
would hand it `invoke`, and the window it would reach is the one holding the
sandbox engine, the RCON store and the settings that anchor the plugin jail.

So the player window opens at `<site>/app-player` — a chrome-free page in
website-city with its own root layout, no providers and no session read. Two
properties follow, and both are enforced by something other than care:

  * **The app's CSP forbids the loader here anyway.** `tauri.conf.json` sets
    `script-src 'self'`, so a loader on a CDN does not execute in the app's
    webview at all. Widening that would widen it for the whole UI.
  * **A remote page has no IPC.** Tauri exposes commands to the app's own asset
    origin only; a window pointed at `https://` has no `invoke`, no event
    channel and no plugin access — by the runtime's construction rather than by
    a permission list somebody has to keep correct. A hostile loader is a web
    page with a web page's powers.

The boot descriptor reaches it through `initialization_script`, not a query
string, for the reason the site's own player gives: a server address on a URL
ends up in every cache entry and history record that URL touches.

That window is **decorated**, unlike the main one. Drawing our own frame needs
`core:window:allow-start-dragging`, and this window is remote precisely so that
it has no capabilities at all. A native frame is the price of the isolation.

**`/app-player` is excluded from website-city's locale middleware**, alongside
`/file/…` and for the same reason: it lives outside `[locale]` deliberately, and
rewritten under one it 404s.

### The webview names ids, never URLs

`play_open_web` takes an app id and an optional server id and asks
`/play/launch` itself. A loader URL from the frontend would be a `<script src>`
injected into a page on the site's origin; a connect link from the frontend
would be a request to hand a string to whatever program claimed a scheme. Both
are resolved in Rust, and a connect link is checked against
`plugins::apps::LAUNCH_SCHEMES` — the same closed list a plugin's launch rule is
held to.

This is the rule `download_release` already followed, restated: there is no
`play_open(loaderUrl)`, and nothing an injected script could point at a script
of its choosing.

## Games TMC publishes

Everything above manages somebody else's game: `detect` finds where Steam put
it, `deploy` puts mods into it, `launch` starts the copy that is already there.
`tmc-core/games/` is the one place the app is the **distributor** — it downloads
a build, unpacks it under the app's own data directory, keeps it current and
starts it as a real process.

```
  /apps/:id/build?platform=…   ── the site resolves ONE build for THIS machine
           │
           ▼
  download::DownloadManager    ── the same queue, limits and pause button
           │
           ▼
  plugins::steps::extract      ── the same zip-slip guard and expansion cap
           │
           ▼
  library::db native_game      ── the only record that these files are ours
           │
           ▼
  launch::LaunchPlan → spawn   ── a real process, with a real play session
```

**Nothing here is new machinery, and that is the design.** An install is a
download, an extraction, a row and a launch, and the app already had one of each
with the bounds and failure modes worked out. A second downloader would have
been a second place for a bandwidth limit to be ignored; a second unpacker would
have been a second zip-slip guard, and the one that was wrong would be the one
nobody was reading. What the module contributes is the sequencing and the
refusals.

### Where the builds come from

`AppNativeBuild` on the website — one row per (app, platform), carrying an
`https` URL, a **required** SHA-256, a size, an archive format, the entry path
inside it and the launch arguments. `scripts/publish-native-build.ts` writes one
and computes the checksum from the bytes it uploads, so the row and the artifact
cannot disagree.

**The catalogue says a build EXISTS; `/apps/:id/build` says where it is**, one
platform at a time. A device browsing the whole app list must not come away
holding download addresses for every game on every machine it is not — the same
rule `download_release` follows, applied to a listing.

**Architecture is part of the platform value.** `AppPlatform` — the descriptive
"where is this game available" enum the website draws on an app page — says
`PLATFORM_WINDOWS`, and that is a fine answer to that question. It is not an
answer to this one: an x64 archive handed to an ARM machine either refuses to
start or runs emulated at a cost nobody chose. `games::platform::current()` is
compile-time, from this binary's own target triple, because the binary that is
asking IS the evidence: a build running under Rosetta should ask for the
architecture whose emulation is already working on this machine.

**Mobile is refused, and that is a statement about the OS.** Android and iOS
both decline to execute code an app wrote into its own container; an `.apk` is
installed by the package installer and an `.ipa` by the store. The two targets
are in the enum because the site may publish them and the app should be able to
say "there is a build, get it from the store" — which is true and useful, and is
not an install.

### What the install refuses, and when

| Refusal | Caught before |
| --- | --- |
| Not `https` | a byte is fetched |
| A checksum that is not 64 lower-case hex | a byte is fetched |
| An archive that does not say what to start | a byte is fetched |
| A build needing a newer app (`minClientVersion`) | a byte is fetched |
| A digest that does not match | the file gets its real name (the queue) |
| A `version` that would name a folder elsewhere | a directory is created |
| An `entry` that escapes the install (`join_relative`) | anything is executed |
| An archive that unpacked without the file it named | the row is written |
| The game is running | anything is overwritten |

The last two are the ones worth keeping. An archive that unpacked perfectly and
does not contain its own entry is an install that **reports success and then
cannot be started**, with nothing on screen to explain it. And writing a new
build over files a live process has mapped fails outright on Windows and
succeeds on Unix, leaving a process running code that is no longer on disk.

**The previous version is not removed until the new one is in place.** A build
lands in `<data>/games/<slug>/<version>/` and the row is only repointed once the
entry is verified, so a failed update leaves a working game. Pruning the old
version directories happens LAST, after the row already points somewhere else.

**Uninstalling removes the files first and the row last**, so a failure halfway
leaves a row naming a partly-removed install — which the app can see and offer
to finish — rather than a directory nothing knows about. The delete refuses any
path that is not genuinely under the app's own games root: `dir` came out of a
SQLite file on the user's disk, and a hand-edited row naming `/` must not be
able to turn "uninstall a game" into a recursive delete.

### Why a game is not a subscription

`subscription` is a MIRROR the full sync rewrites: anything the server did not
send is deleted, because that is how an unsubscribe on another device reaches
this one. A game on this disk is a fact about this disk — the account knows
which games exist and cannot know which of them somebody unpacked onto a laptop
— so `native_game` is its own table for the same reason `local_mod` is. A sync
must not be able to forget an install, and signing out must not either.

### Keeping them current

`games::updates::run` on the library's sync pass, which is the device's own
"am I online now?" heartbeat — the same place outstanding play time is drained.
Four rules, all of them written down rather than implied: the install must have
`auto_update` on, the game must not be running, only an install that already
exists is updated, and **a version that cannot be ordered is not an update**.

`games_check_updates` reports and installs nothing, even for a game with
auto-update on. It is the button somebody presses, and a button that silently
downloaded four gigabytes because a row had a flag set is a button nobody
presses twice.

### The launch arguments are a vector, never a command line

`args` is one element per argument and stays that way. A command string would
have to be split by somebody, and there is no shell in this app to split it — so
a server name containing a space would become two arguments and one containing a
quote would become something nobody can predict. Substitution happens INSIDE an
element and can never create a new one, and a value carrying a control character
drops the argument rather than being escaped.

**An argument whose value is missing is dropped, with the flag before it.**
`["--connect", "{host}:{port}"]` with no server must not become
`["--connect", ":"]` — the game would take that as an address, fail to reach it,
and report a connection error for a connection nobody asked for. Dropping the
pair starts the game at its own menu, which is what "launch with no server"
means.

## Play sessions

`spawn::run` used to `Command::spawn` and drop the `Child`, with a comment
saying the game outlives the launcher. The first half was right and the
conclusion was not: dropping the handle dropped every fact about the launch. One
missing piece, three symptoms — the account was told `playedSeconds: 0` on every
start, the Library could not say whether a game was already open, and a game
that died in two seconds left nothing to read.

`session::Sessions` keeps it on a supervisor thread. What that buys, and what it
deliberately does not:

**`SessionKind` is the honest record of what can be known.**

| Kind | Duration | Exit code | Output |
| --- | --- | --- | --- |
| `Process` | measured | real | captured |
| `Handoff` | **none** | none | none |
| `Web` | measured | none | none |

A `steam://` launch is a `Handoff`: the OS opener started a launcher which
started the game, and there is no process of ours. It reports **no** play time
rather than the time between firing a URI and the app noticing — that number
describes the app, and a launcher whose statistics are fiction is worse than one
with none. The Library says "not measured" for one, not "0h".

**A duration is never inferred from a later launch.** Other managers treat "the
app was closed" as the end of a session, or clamp a long one to something
plausible. Both produce a number nobody can distinguish from a right one.

**Output goes to one bounded file per session.** That is the whole crash-report
story: a game that exits in two seconds has almost always printed why, and
without a pipe that goes to a console nobody attached. Bounded because a game
left running overnight with a chatty logger is the normal case.

**`stop` signals by pid.** The supervisor holds the `&mut Child` for its whole
`wait`, so `Child::kill` is unreachable. Signalling by pid is normally a bug —
a pid is reused — and is sound in exactly that position and nowhere else: a
process inside `wait` has not been reaped, so its pid cannot yet have been
handed to anybody else.

**A finished session is WRITTEN, not reported.** It lands in `game_session`
marked unreported, and `sessions_flush` sends it when the device next has an API
to talk to. A game is very often played offline — a laptop on a train is the
case the feature is for — and play time that only counts when the network
happened to be up is play time that silently goes missing. The flush runs on the
library's sync pass, which is the device's own "am I online now?" heartbeat.

**A sandbox that is already running cannot be started twice.** The second copy
opens files the first has mapped, and the play clock for that sandbox would
otherwise be able to exceed wall-clock time.

## Finding games the launchers do not know about

`detect::scan` reads Steam's manifests, Epic's `.item` files and Galaxy's
database. That is the right first answer and covers most machines. It cannot
cover a game copied from another PC, a dedicated server unpacked by hand, or a
drive that was moved.

`detect::walk` walks folders **the user ticked**. `roots` is a required argument
with no default, and that is the design rather than an inconvenience: a scan of
"the filesystem" is a scan of somebody's documents, their photos and every
network share the machine has mounted, and a feature that did that by default
would be indistinguishable from one looking for something else.

Bounded on four axes, each a real machine rather than a hypothetical:

| Bound | The machine it exists for |
| --- | --- |
| Depth | A game's own asset tree, which is where the hundred thousand files are |
| Directory count | A `node_modules` tree, a `/nix/store` |
| Wall clock | A network share that has gone away and answers `readdir` in thirty seconds |
| Never follows a symlink | `~/.wine/dosdevices/z: -> /`, which is a layout people have |

Two properties worth keeping:

  * **A matched folder is not descended into.** Everything below a game's root
    is the game's own data.
  * **An unmatched folder is not a finding.** That is what separates this from a
    file browser: a list of every directory on a drive is not a scan result.

`WalkStop` distinguishes "walked everything" from "hit a limit", because showing
them identically is how somebody concludes their game is undetectable when the
walk simply never reached it.

**Applying is still separate.** `detect_apply_many` runs each candidate through
`detect_apply`'s own validation — a batch where one folder is refused must still
apply the other eleven and say which one was not.

## Sharing a sandbox

A code, not a file. `library::share` encodes a sandbox as `TMC1-<base64>`.

**Why a code.** A file export needs a path to write and a file import needs a
path to read, and the webview names neither — the one command that takes a path
returns directory names and reads nothing. Trading that guarantee for a save
dialog is not a trade worth making, and a code is what people do with an
exported profile anyway: put it in a message.

**Item ids, never files.** Shipping the files would make this a redistribution
channel for other people's work, with no download counted, no licence respected
and no update path.

**Nothing about this machine.** No game directory, no launch environment, no
ledger, no staging path — the same test the settings split uses, and a code is
by definition going somewhere else. A game folder in a shared code is also
somebody's username in a chat message. `a_code_carries_nothing_about_this_machine`
decodes an export and asserts none of it is there.

**Importing previews first**, because a code is a string from somebody else and
pressing Import can make two hundred subscriptions on an account. It subscribes
and adds; it does not stage — two hundred mods is two hundred downloads, and
starting them inside a command the UI is awaiting is a frozen dialog with no
queue to look at. `cloudSync` and `autoUpdate` come from the importer's own
defaults: whether somebody's mod list leaves their machine is their decision,
not the exporter's.

## Importing mods from outside the account

Everything above assumes an account subscribed to something. That is the right
model for what the site holds and it is the whole model for nothing else — and
"nothing else" is most of what is on a modder's disk: the jar from a Discord
thread, the folder they built themselves, the two hundred mods already sitting
in Vortex, the pack downloaded before they had an account.

A **local mod** is one directory of files plus editable text about it, and it is
deployed by exactly the code that deploys a subscribed one.

`deploy::DeployMod` wants a key, a name, a priority and a folder laid out as it
belongs under the game directory. A staged subscription is one of those; an
import is another. So the merge tree, the conflict report, the ledger, the
backup-before-overwrite rule and the "still ours" purge check all apply to an
imported mod with **no new code and no second set of rules to keep in step** —
`library::deploy::mod_root` is the single place the two sources meet, and
`an_imported_mod_and_a_subscribed_one_deploy_through_one_ledger` is what stops
that claim rotting.

| File | Owns |
| --- | --- |
| `local/metadata.rs` | `tmc.json` — what an archive says about where it came from |
| `local/store.rs` | The files: importing, laying out, moving, removing |
| `local/adopt.rs` | What is already in a game folder that nothing here claims |
| `local/vault.rs` | How the webview names a path it is never shown |
| `plugins/managers.rs` | The fourth plugin type: another mod manager, read |

### The webview still never names a path

`commands/import.rs` looks like an exception to `commands/mod.rs`'s second rule
and is not one. Every path it acts on was produced by **this side** — the OS
delivered a drag-and-drop event to the window, or one of the two scans listed a
directory — and reaches the frontend as an opaque token.

What that buys is bounded and `local/vault.rs` says so: a script with execution
in the webview can replay a token for a file the user just dropped, which is
nothing it could not get by asking them to drop it again. It cannot name
`~/.ssh/id_ed25519`, because **no token exists for one**. Tokens expire and the
store is capped, because a capability with no lifetime is one an injected script
can hoard.

`dragDropEnabled` is therefore ON in `tauri.conf.json`, which also means the
webview receives no HTML5 drag events at all — the point, since an `ondrop`
handler there would be the untrusted half deciding what was dropped, holding
real paths to do it with. The hover events are emitted from Rust carrying a
COUNT and nothing else.

### `tmc.json`

The website puts a small JSON document at a release archive's root; the importer
reads it for the item's name, version and link. It is a **hint about identity**,
not a permission and not a signature: it arrives inside a file a stranger
supplied, so `installPath` goes through `join_relative` like any other untrusted
path, and an item link is offered only when `source`'s ORIGIN matches this
build's `api_base()` — a production archive must not hand a dev build an item id
from a different database.

Not signed, deliberately. The app has the machinery, and it would be the wrong
question: a release file is authored by the mod's author and served by the site
unmodified, so a signing key covering every upload is a key on a web server. The
thing worth verifying about a download is its **checksum**, which the API already
carries and the queue already enforces.

### The one genuinely ambiguous decision

Whether to unpack. `.jar` is a zip and must not be unpacked; a Minecraft
resource pack is a `.zip` and must not be either; a mod shipped as a zip of
loose DLLs must be. So the **game's own install rules decide**:
`AppPlugins::mod_extensions` reads `match.extensions` off the `manage_*` rules,
and a file whose extension a rule accepts is copied verbatim. Nothing invents a
second list, and a game that grows a new packaging format gets it for free.

Where the payload lands is a guess and is treated as one — the sidecar's
`installPath`, then "does its top level already hold a declared mod target?",
then the game's first `modTargets` entry — and then the user can change it.
`store::relayout` is what makes that cheap: a directory rename inside the store,
not a re-import.

### Adopting what is already there

`local::adopt` lists the immediate children of the folders a game's
`sandbox.json` declares as mod targets, and subtracts everything the app can
account for: the deployment ledger, a subscription's installed files, and
previously adopted rows. Getting that subtraction wrong is worse than not having
the feature — adopt something the app already deploys and the user has one mod
listed twice, with the merge tree correctly reporting every file as conflicting
with itself.

It is **not a filesystem scan**. A game with no `sandbox.json` offers nothing,
which is the honest answer rather than a guess about where its mods might be.
And it **copies**: the original stays exactly where it was, so backing out is
deleting a row and the game keeps working either way.

### Reading another mod manager

The fourth plugin type. `plugins/manager/*.json` is compiled into the binary by
`core/build.rs` alongside the app rules and for the same reason, with
`<app data>/plugins/manager/` as a per-id overlay; a registry bundle carrying a
`manager` block is the third source and goes through the approval and signature
gate on its way out.

A descriptor names directories and says how they nest — a base the APP resolves
(`home`, `appData`, `localData`, `programData`, `documents`), a relative path, a
`modsPath` with `{game}` and at most **one** `*` segment whose name becomes the
profile/instance label, a game-id-to-slug table, and optionally a per-mod JSON
file with pointers into it. It cannot run a program, read an environment
variable, name an absolute path or reach the network, because nothing in the
format expresses any of those. One wildcard, not any number: two would make a
scan the product of two listings over somebody's whole `AppData`.

**Every shipped descriptor reads local storage, and that is where the answer
is** — not a limitation of the plugin model:

  * **CurseForge's** API needs a key issued per application and does not answer
    "what has this user installed"; the instance folders do.
  * **Nexus's** API is per-user and **Vortex** has no remote API at all; its
    staging folders are the record.
  * **Thunderstore's** API is open but serves the *registry*, not what a
    machine installed; r2modman's profiles are on disk.

Ships: r2modman, Thunderstore Mod Manager, Vortex, CurseForge, Prism, MultiMC.
The game folder names those managers use are their own and are the one thing
likely to be wrong — a corrected descriptor dropped in `plugins/manager/` is the
whole fix, which is exactly why this is data rather than a `vortex.rs`.

MO2 is **not** shipped, and the reason is worth keeping: its instances are named
freely by the user and the game is recorded inside `ModOrganizer.ini`, so no
declarative name match is reliable. A descriptor that guessed would find the
wrong game's mods, which is worse than finding none.

### What a local mod deliberately has not got

**Updates.** There is no release history, no checksum from anybody and no URL to
re-fetch — that is what makes it local. `LocalMod` has no `latest_version`, the
auto-updater never sees one (it looks items up with `find_item`, which returns
nothing for `local:`), and the honest way to update an import is to import the
new file. When a sidecar named an item on this site the row keeps the reference
so the UI can offer "subscribe to this instead", which is what turns a snapshot
into something the app can keep current.

**A second identity space.** The key is `local:<id>`, in the same `sandbox_mod`
table and the same ledger as `mod:1234`. One list, one load order, one deploy.

**A place in `subscription`.** `local_mod` is its own table because
`subscription` is a mirror the full sync rewrites — anything the server did not
send is deleted, and a dropped jar surviving its first sync is the one behaviour
an import must have.

## Editing a game's own settings

Every mod manager studied for this has a config editor, and every one exists for
the same moment: a loader wrote `BepInEx/config/com.author.mod.cfg` the first
time the game ran, one value in it is wrong, and fixing it meant leaving the
manager.

### What the jail is for here, and what it is not

Worth stating exactly, because it is easy to overstate in both directions.

**The app process has the user's filesystem permissions and must.**
`deploy::link` writes into game folders; the plugin executor unpacks archives
into them. Nothing about a config editor changes that and nothing needed to.

What these commands add is the ability for the **webview** to name a file to be
read or written — the rule at the top of `commands/mod.rs`, and the reason a mod
description rendered next to `invoke` is a nuisance rather than a problem. So
the surface is bounded the way every other privileged surface here is, and the
cost is nothing anybody wanted: a config editor that could open `/etc/shadow` is
not a better config editor.

### Two bounds, not one

| Bound | Catches |
| --- | --- |
| The declared locations compile to jail grants | A path outside the game's folders entirely |
| `config::locate` re-runs the spec's own rules on every read and write | A path inside a granted root that the game never offered |

The second is not redundant. A location naming the game's ROOT — which
`{ path: "", files: ["options.txt"] }` legitimately does — grants the jail the
whole game folder, so the jail alone would happily resolve `saves/world.dat`.
The listing's own matching rules are therefore re-run on the requested path,
which means the only files that can be opened are ones the list would have
shown.

### `config.json`

```json
{
  "manifestVersion": 1,
  "config": {
    "locations": [
      { "label": "Mod settings", "root": "gameDir", "path": "config",
        "extensions": ["toml", "cfg"], "recursive": true },
      { "label": "Game options", "root": "gameDir", "path": "",
        "files": ["options.txt"] }
    ]
  }
}
```

  * **A location is a FOLDER, never a single file.** Not a limitation — it is
    what stops the grant it compiles into destroying the thing it exposes.
    `Jail::build` pre-creates every writable grant's prefix, so a location
    naming `options.txt` would create a *directory* called `options.txt` where
    the game's settings file belongs. Name the folder; list the file in `files`.
  * **`files` replaces the extension test rather than adding to it.** A location
    naming both would be asking two questions with one answer.
  * **Grants are DERIVED from the locations**, not declared beside them. An
    author who adds a location and forgets a matching grant would otherwise get
    a folder that lists nothing, with no error anywhere.

### Why the editor is a plain text box

Gale has a schema-driven form with typed fields, and it is genuinely nicer where
it works. It works by understanding BepInEx's own config dialect — and the
moment a game uses TOML, an INI, a properties file or something bespoke, a form
either cannot render it or renders it wrongly and writes back something the
loader will not parse.

A text box round-trips every format exactly. The safety is elsewhere and is
real: the previous contents are copied into the app's backup folder on every
save, the write goes to a temporary file and is renamed (a half-written config
is a game that will not start), and a file whose bytes are not UTF-8 is refused
rather than mangled — reading one as lossy UTF-8 and writing it back replaces
every invalid byte, which is an editor that destroys the file it was opened to
fix.

The backup goes under the app's own directory rather than beside the file: a
`.bak` next to a config is a file the game's own loader may try to parse, and
one studied loader does exactly that.

### `allowConfigEditing`

ON by default — editing a loader's `.cfg` is ordinary work for a mod manager.
It exists because this is the one feature that lets the webview name a file to
be written, and **what it buys is bounded and is stated on the settings row
itself**: it removes those three commands and nothing else. It does not sandbox
the process, which writes to game folders whenever it deploys.

## Downloads

Every file the app fetches goes through `download::DownloadManager` — a mod's
release archive, a sandbox's staging, whatever a plugin's `download` step names.
It is a subsystem rather than a function because a modpack is not one download,
it is four hundred, over a home connection, on a laptop that will be closed
halfway through.

```text
  enqueue ──▶ queued ──▶ running ──┬──▶ done
                 ▲         │       ├──▶ failed ──▶ (retry) ──▶ queued
                 └─────────┴───────┴──▶ paused ──▶ (resume) ──▶ queued
```

Two guarantees worth naming:

  * **The destination filename only ever appears after the last byte and the
    checksum.** Progress lives in a `.tmcpart` file beside the target, so
    nothing downstream can pick up a partial file and treat it as complete.
  * **A `200` in answer to a range request means the server ignored it**, so
    the file restarts rather than being appended to. Appending produces a file
    that is too long, passes every length check, and fails its checksum with an
    error nobody can explain.

**Downloading one named release goes through the queue too.** An item's page
lists its version history, and both its buttons — the header's "Download 1.4"
and each row's — used to hand the URL to the system browser. That is a strange
thing for an app whose whole download story is the queue being bypassed, and
somebody fetching an older release because the newest one broke their save is
exactly the person who wants it pausable. `download_release` takes a kind and
two ids, **never a URL and never a path**: Rust re-fetches the detail, picks the
file, and puts it in the user's download folder. So the rule at the top of
`commands/downloads.rs` still holds — there is no `download_start(url, path)`,
and nothing an injected script could point at a file of its choosing.

**A plugin's `download` step goes through the queue** when the executor has a
handle to one, with a deterministic id derived from the plugin and the
destination — so re-running a failed install is the same row rather than a
second writer for one file. The host allow-list, the size cap and the checksum
are enforced on both paths; the queue moves where the bytes are read, not which
checks run.

**The queue gets its own HTTP client, not the API's.** The API client attaches a
bearer to everything it sends, and a mod archive comes from a CDN that has no
business seeing one — a redirect to a third-party mirror would hand out an
access token.

Bandwidth is a token bucket (`download::rate`), composed as global × per
download, both live-adjustable. A request larger than the bucket can hold is
served by taking the balance negative rather than refused, because a downloader
whose chunk size it does not control would otherwise hang.

The device reports a SNAPSHOT of its queue to `/api/app/v1/downloads` so the
website can show it. That row never drives anything — there is deliberately no
endpoint that tells a device to pause or cancel — and it carries no URLs and no
filesystem paths.

## Finding the games

`detect::scan` reads what the launchers already wrote down rather than asking
somebody to type a path.

| Source | Reads |
| --- | --- |
| Steam | `libraryfolders.vdf`, then every `appmanifest_*.acf` |
| Epic | `Data/Manifests/*.item`, plus Heroic/Legendary on Linux |
| GOG | `galaxy-2.0.db`, and the plain `GOG Games` folder |
| Xbox, Ubisoft, EA, Battle.net | their fixed folder layouts, one level deep |
| a game's own hints | `sandbox.json`'s `detect.paths` |

**Detection suggests; it never configures.** Applying a result goes through
`anchor::validate_root` exactly as a hand-typed path does — a folder is not more
trustworthy for having been found automatically, and a game directory is a jail
anchor.

**It reads no environment variable.** The platform directories arrive as
`DetectRoots`, filled by the Tauri crate from its path resolver.

Matching a folder to a TMC game, in descending order of confidence: the
launcher's own id (exact, language-independent, survives a rename), then a
marker file, then the normalised display name. A name match must also find the
marker; an id match is not second-guessed.

Two details that are easy to get wrong and are commented at the site: Steam's
`installdir` is NOT the display name (`Grand Theft Auto V` lives in `GTAV`), and
GOG's database is opened `mode=ro&immutable=1` so a running Galaxy neither
blocks the read nor hands back a torn one.

## RCON

`rcon::` speaks two dialects: **Source** (which is also Minecraft's, Rust's,
ARK's, Squad's and Palworld's) and **Frostbite** (Battlefield 3/4/Hardline/BC2).
Both parse through `net::reader` and both have the truncation test every parser
in this crate has.

Three protocol details that are easy to get wrong:

  * **A Source multi-packet reply has no end marker.** After the command, a
    second empty packet with a different id is sent; the server answers in
    order, so its echo is the end. Stopping at the first packet truncates
    `status` on a full server; waiting for more hangs on every short reply.
  * **A successful Source auth sends TWO packets**, in an order implementations
    disagree about, so the handshake reads until it sees the auth reply rather
    than counting.
  * **Frostbite's `size` counts the two header fields it is part of.** Omitting
    them leaves four bytes of each packet in the buffer, which presents as "the
    second command always fails".

### The one deliberate SSRF gap

Everywhere else an address goes through `net::addr::resolve_public`. **RCON does
not**, and the module header says so outright: `192.168.1.10` and `127.0.0.1`
are the *normal* answers here, because the feature is "administer my server" and
most people's server is on their own network. A guard that refused them would
refuse the feature.

Bounded instead by a per-host connection cooldown, a session cap, and a
Security-level audit entry on every connect. It remains a widening and it is
listed as one below.

### The passwords

Encrypted on the device with XChaCha20-Poly1305, key in the OS credential store
(`crypto::LocalCipher`), and **never sent to the website**. A server password is
not derived from a TMC account, losing it costs somebody their server rather
than their profile, and no feature on the site needs it.

What that buys is stated honestly in `crypto.rs`: it answers a **copied file** —
a backup, a synced folder, a disk pulled out of a laptop. It does not answer
code running as the user in this session, and nothing on any desktop platform
does. What the design guarantees instead is that the webview is not such code:
`rcon_secret` is `pub(crate)`, `rcon::exec_saved` is its only caller, and
`RconServer` has no password field for a refactor to start returning.

## Deep links

`tmc://` is registered on desktop and declared in the mobile manifests. One rule
governs all of it:

> **A link can ask the app to SHOW something. It can never ask the app to DO
> something.**

Any program on the machine can claim a custom scheme and any web page can
navigate to one without a click, so a link is an untrusted request from an
unknown party and the most it achieves is a screen with a button on it.

| Link | Does |
| --- | --- |
| `tmc://auth` | Polls the API now instead of waiting out the interval. Carries nothing |
| `tmc://install/<kind>/<id>` | Opens that item's page with the install controls ringed |
| `tmc://view/<kind>/<id>` | Opens that item's page |
| `tmc://sandbox/<id>` | Opens one sandbox |
| `tmc://play/<host>:<port>` | Opens a **join screen** for that address |
| `tmc://play/app/<slug\|id>` | Opens that game's launch dialog |

**`tmc://play/<host>:<port>` is the website's Join button**, and it is the only
link whose payload is not an id of ours. It is still only a screen: `/join`
resolves the address through `/servers/lookup`, shows which game is running
there with the latency measured from this device, and offers the launch modes
this machine actually has. The rule is load-bearing here rather than
theoretical, because this is the link an ordinary web page can navigate to
without a click — an auto-joining version would be a remote primitive for making
somebody's machine connect to an address a stranger chose.

Three properties of the grammar:

  * **The address is in the PATH, not a query string.** Custom-scheme openers
    across five platforms disagree about almost everything and agree about path
    segments, and a grammar with two segments is far easier to keep honest than
    one accepting arbitrary parameters — every parameter a link can carry is a
    parameter somebody eventually reads.
  * **Exactly the two shapes, with nothing after them.** `tmc://auth` ignores a
    trailing segment, and that is right for a wake-up with no arguments. Here it
    would be wrong for the opposite reason: this action HAS a grammar, and
    accepting `tmc://play/host:1/join` today is how `/join` quietly becomes
    meaningful the first time somebody adds a segment to the match.
  * **The port is optional**, because `tmc://play/example.com` is a perfectly
    good way to name a box, and `tmc://play/:` — the shape an empty `{host}`
    substitution used to produce — parses to nothing. The website declines to
    build that link now; this is the other half of the same fix, on the side
    that would have to act on it.

`tmc://install/mod/1234` is the **guest** flow: a signed-in user gets a
subscription, which follows them to their other devices and keeps the mod
updated, but a guest has no account to hang one on.

Everything else parses to nothing, and an unrecognised link does nothing at all
— it does not fall through to the auth wake-up, because a default action is a
default action an attacker gets to trigger. `deeplink.rs`'s test names the
shapes that must never start working: `tmc://deploy/1`,
`tmc://settings/gameDir?path=/`, `tmc://rcon/1/exec?command=quit`.

## Settings: the two halves

| | App settings | Account settings |
| --- | --- | --- |
| Stored | `settings.json`, this machine | `UserSettings` on the website |
| Written by | `settings.rs` | `PATCH /api/app/v1/me` |
| Read by | `useSettings().app` | `useSettings().user` |
| Contains | Theme, scale, game directories, logging, latency, download limits, plugin prompts | Notifications, locale, timezone |
| Can fail | No | Yes — network, auth |

The test for which side something belongs on: **would this be wrong to apply on
a different machine?** A game directory would be. A notification preference
would not.

### Two of them are not settings at all

`gameDirs` and `downloadDir` anchor the plugin jail — `plugins::jail`
resolves every `PathRef` beneath them, so an installer holding
`{gameDir, "", write}` can write anywhere below. They are therefore the only
fields with their own commands:

- **`settings_patch` refuses them** (`JAIL_ROOT_FIELDS`), loudly rather than
  by dropping the key. The refusal is in `SettingsStore::patch`, so it holds for
  every caller and not just the one command.
- **`settings_set_game_dir` / `settings_set_download_dir`** run
  `anchor::validate_root`, which rejects a drive root, a system directory, and
  anything that *contains* the app's data, logs, cache, plugins or the user's
  home. A jail anchored above the app's own files would enclose `settings.json`
  and the plugin registry — a plugin that can rewrite the registry can grant
  itself permissions.
- The **canonical** path is what gets stored and what gets audited, so the value
  in `settings.json` is the one the jail will resolve to later.
- **A sandbox's own `gameDir` goes through the same validator**, on
  `sandbox_patch`. It gets no more trust for having arrived on a different
  command.
- Both are audited at **Security** level, so turning logging off cannot hide a
  change to where plugins may write.

What this does *not* establish is that a human chose the path. That came from
the OS dialog and is gone with it; the anchor's **identity** is what is checked
now, not its provenance. `anchor.rs`'s header says so plainly — don't let a
later comment upgrade the claim.

## Logging

`Settings → Logging` reads an append-only JSONL audit trail. It is a **record of
what the app did on the user's behalf**, not diagnostics — `tracing` handles
those and goes to stderr.

- `Security` level entries are written **even when the user turns logging off**.
  A switch that lets a plugin ask the user to stop watching is not a control.
- Clearing the log writes the "log cleared" entry **after** the truncation.
- Every plugin step, every download URL, every jail refusal, every permission
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

### The window frame is the app's, not the OS's

`decorations` is **off** in `tauri.conf.json` and `components/titlebar.tsx`
draws the frame. The reason is Linux: Tauri's backend there is wry → WebKitGTK,
so a decorated window gets a GTK titlebar themed by whatever desktop the user
runs — and there is **no Qt backend** to switch to. The only way a Tauri app
stops looking like a GTK app is to stop letting GTK draw any of it.

That decision cascades, and each piece is load-bearing:

- **Dragging is `data-tauri-drag-region`, never `-webkit-app-region: drag`.**
  The CSS property is a Chromium extension: it works in WebView2 on Windows and
  does nothing at all in WebKitGTK or WKWebView.
- **Resizing is eight fixed strips** calling `startResizeDragging`. With
  decorations off the window manager no longer offers a grab border, so without
  them the window cannot be resized at all.
- **Six `core:window:*` permissions** are in `capabilities/default.json` for
  exactly this. They act only on the main window and carry no data.
- **`tauri-plugin-dialog` is deliberately not registered.** Its folder picker is
  the GTK file chooser on Linux; `components/folder-picker.tsx` replaces it.
- **Native form controls are reset in `app.css`.** A GTK combo box, an Aqua
  select and a Fluent one are three shapes for one screen — the same mismatch
  the frame removes, arriving through a different door.

## Commands

```bash
npm install                # frontend deps (needs GitHub Packages auth for @modcommunity)
npm run dev                # vite only, no Rust — fastest loop for UI work
npm run desktop            # tauri dev
npm run desktop:build      # tauri build, for this machine
npm run build:linux        # AppImage + deb
npm run build:windows      # portable exe + .exe setup + .msi, cross-built

npm run android:init       # once, generates gen/android
npm run android            # tauri android dev
npm run ios:init           # once, macOS only
npm run ios                # tauri ios dev

npm run check              # everything. Run before declaring done
npm run check:web          # tsc + eslint + vitest
npm run check:rust         # clippy -D warnings, whole workspace
npm run test               # both sides
npm run test:web           # vitest
npm run test:rust          # cargo test, whole workspace
npm run test:core          # tmc-core only — no GTK stack needed, runs anywhere
npm run fmt                # prettier + cargo fmt

npm run contract:sync      # re-copy the API contract from website-city
npm run sysroot            # build the local GTK sysroot (see below)
npm run shared:local       # install @modcommunity/shared from ../tmc-global
npm run shared:build       # rebuild it in place
```

### Pointing a build at the dev site

```bash
TMC_API_BASE=https://tmcdev.net npm run desktop        # debug: read at RUN time
TMC_API_BASE=https://tmcdev.net npm run android        # baked in at BUILD time
TMC_API_BASE=https://tmcdev.net npm run build:windows  # so is a release build
```

**`https://tmcdev.net`, not `https://tmcdev.net:3002`.** Port 3002 is the Next
dev server speaking plain HTTP; 443 is the one with TLS in front of it. Pointing
`https://` at 3002 fails the handshake, which surfaces as "could not reach the
site" and reads exactly like the box being down.

Two consequences of the dev site's certificate being signed by a private CA
(`TMC Local Development CA`) rather than a public one, both of which cost an
afternoon if you meet them without expecting them:

- **The machine running the build's OUTPUT has to trust that CA**, not the
  machine that built it. A cross-built exe handed to a Windows box which has
  never seen the dev CA fails every request with a TLS error.
- **`http://tmcdev.net:3002` is not a way around it.** `normalise_base` refuses
  `http` for anything but a loopback or LAN host, and a public-looking domain is
  neither however it resolves. The LAN address is: `http://10.50.0.185:3002` is
  accepted, at the cost of `tauri-plugin-opener` being scoped to `https://*` —
  so sign-in cannot open the browser for you and the device screen prints the
  URL instead, which is exactly the case that behaviour exists for.

`tmc_core::api::api_base()` resolves the base once per process, from
`TMC_API_BASE` in the environment (**debug builds only**) and otherwise from
`TMC_API_BASE` at compile time, falling back to production. `core/build.rs`
carries the `rerun-if-env-changed` that makes the compile-time half honest —
without it a rebuild keeps the base the binary was FIRST built with.

Four properties, none of them incidental:

- **It is never a setting and never an argument from the webview.** There is no
  `api_set_base`; `api_env` is read-only. A "which server?" field is a phishing
  primitive — point the app at a look-alike and it sends that host a bearer
  token — and the same is true of a field a script in a mod description can
  reach.
- **A release build has no runtime path at all.** The environment of the
  process that launched the app is not a trust boundary: a `.desktop` file, a
  shortcut's "Start in", an installer's launch step all set one.
- **The value is validated, not trusted.** Scheme and host only — no path, no
  query, no credentials — reduced to an origin by `Url::origin`, and `http` is
  refused for anything but a loopback or LAN host. A refused override logs and
  falls back to the built-in base, because "my dev server saw no traffic" is a
  better failure than "my token went to a host I fat-fingered".
- **Each base gets its own stored session.** `secure.rs` keys the refresh token
  on `api_base_scope()`, so a dev run neither reads nor overwrites the
  production one. Sharing the entry meant the real refresh token was sent to the
  dev server on its first refresh — and a rejected refresh is terminal, so it
  also signed the developer out of the live site.

A non-production base is **shown** (title bar badge, Settings → App →
Development, and the sign-in copy names the real host) and **audited** at
Security level on launch. Nothing else on screen distinguishes staging from
production, which is how a screenshot of dev data becomes a bug report about
live data.

The OS hand-off (`tauri-plugin-opener`) stays scoped to `https://*`, so with a
plain-`http` local base the app can browse and query but cannot open the login
page or an article in the browser. The device screen prints the URL for exactly
that case.

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

## Shipping it

Four of the five targets can be built without owning the machine they run on.
`docs/BUILDING.md` is the full account; what belongs here is the shape and the
one thing that is easy to undo.

**Windows cross-builds from Linux, installers and all.** `scripts/build-
windows.sh` runs `tauri build --runner cargo-xwin`, which is what makes bundled
SQLite compile for Windows from here — the same gap that made `tmc-usvfs` its
own crate, closed for the rest of the tree by a toolchain rather than by a
split. NSIS already runs anywhere. **The MSI does not**: Tauri builds one by
shelling out to WiX's `candle.exe`, so `scripts/make-msi.sh` compiles
`packaging/windows/tmc.wxs` with `wixl` from msitools instead.

That file is written against **the subset wixl implements**, which is smaller
than WiX's — no `MajorUpgrade`, no launch conditions, no `Platform` attribute,
no WixUI. A change that reaches for one of those does not fail at review, it
fails at `wixl` with a Vala assertion, and the fix is a Windows runner rather
than a workaround. Its header says which elements are there and why.

The MSI registers `tmc://` in `HKLM` itself, because a per-machine install is
the one elevated moment there is and the app cannot claim the scheme for
itself afterwards. It does **not** bootstrap the WebView2 runtime — wixl has no
launch conditions, so it cannot even warn — which is why the `.exe` setup is
the recommended download and the MSI is for policy deployment.

**macOS is the exception and stays one.** The SDK is not redistributable and an
un-notarised `.app` is one Gatekeeper refuses outright rather than warns about,
so a cross-build would produce something that cannot be run. It comes from the
`macos-latest` runner in `.github/workflows/release.yml`.

**Nothing cross-built is signed.** Tauri skips signing on a foreign host and
says so on every run. A build handed to somebody else gets a SmartScreen
warning, and that is a fact about the build rather than about the machine it
was made on.

### The one-line installer

`scripts/install.sh` is what `curl … | sh` runs, so it is written as a document
somebody reads before piping it: POSIX `sh`, no `sudo`, `~/.local` unless run as
root, and `--uninstall` removes exactly what it wrote.

**The checksum is required, not advisory.** It reads `checksums.txt` from the
release and refuses to install without it — which makes the `publish` job's
`sha256sum` step load-bearing: a release cut by hand without one is a release
the installer will not touch.

It resolves the latest version from the **redirect** on `/releases/latest`
rather than the GitHub API, which is rate-limited per IP at sixty an hour and
would be spent by whoever shares the NAT.

Where **libfuse2** is missing — Debian 13, Ubuntu 24.04 — the AppImage is
unpacked into `~/.local/lib/tmc` and `bin/tmc` becomes a symlink to its
`AppRun`. The same problem hits the BUILD side, where linuxdeploy and
appimagetool are themselves AppImages: `APPIMAGE_EXTRACT_AND_RUN=1` is what
both honour, and Tauri reports its absence as the unhelpful `failed to run
linuxdeploy` — as it does the missing `librsvg2-dev` the GTK plugin needs.
`build-linux.sh` checks for both by name.

## Working here

### What CI runs, and why it is split that way

`.github/workflows/ci.yml` on every push and pull request, in four jobs rather
than one:

| Job | Runs | Needs |
| --- | --- | --- |
| `web` | `check:web` — tsc, eslint, vitest | node |
| `core` | clippy, tests and `cargo fmt --check` for `tmc-core` + `tmc-usvfs` | **nothing else** |
| `shell` | `check:rust` and `test:rust` for the whole workspace | the GTK/WebKit stack |
| `windows-crosscheck` | `cargo check -p tmc-usvfs --target x86_64-pc-windows-gnu` | the Windows target |

**The `core` job is the one the workspace split exists for.** Every guard worth
failing a pull request over — the path jail, the address guard, the protocol
parsers, the token lifecycle — is in `tmc-core`, and that job proves them on a
runner with no display stack, no WebKit and no dbus. If it ever starts needing
`apt-get`, something has been added on the wrong side of the line.

`windows-crosscheck` is the only thing that looks at the injector at all: those
two hundred lines have never been run against a game, and type-checking them
against the Windows target is what has caught the four real bugs found in them
so far.

`release.yml` is separate and builds the bundle on all three desktop platforms.
It is not duplicated here — proving that a `.dmg` links on every push would cost
twenty minutes to re-prove something that matters only when a release is cut.

`npm run audit:rust` is deliberately in neither: `cargo-audit` is a separate
install, and wiring it into the definition of done would make a fresh clone fail
its own check.

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
- New privileged capability → new `#[tauri::command]` under `commands/`, with the
  policy check *in* the command, not in the caller.

### Supply chain: `npm run audit:rust`

`cargo audit --deny warnings`, against `src-tauri/Cargo.lock`. Deliberately
**not** part of `npm run check`: `cargo-audit` is a separate `cargo install`
and is not in the toolchain `check` assumes, so wiring it in would make the
definition of done depend on a tool a fresh clone does not have. Install it
with `cargo install cargo-audit --locked`.

**Deny-on-warning, with a reviewed allow-list** in `src-tauri/.cargo/audit.toml`.
An unfiltered run reports the same 17 advisories every time, which trains
everyone to stop reading it — and then a real one lands unnoticed. Filtered, the
command is silent until something new appears. Ignores are per **advisory id**,
not per crate, so a genuine new advisory against `gtk` still fails the run.

None of the 17 is a vulnerability; `cargo audit` reports zero of those across
576 crates. All are `unmaintained` or `unsound`, and all arrive through Tauri's
own tree — 12 from the gtk-rs GTK3 bindings behind the Linux WebKitGTK webview
(Linux-only; absent from the Windows, macOS, Android and iOS builds), and 5
from `urlpattern` inside `tauri-build`, which runs at compile time and is not
linked into the app. `audit.toml` says which is which and why.

**Re-check the list whenever Tauri is upgraded.** An entry that stopped
appearing should be deleted from `audit.toml`, not left behind as a comment.

### Tests on the frontend side

`vitest`, node environment, `src/**/*.test.ts`. It runs inside `check:web`, so
a broken test fails the same command a type error does.

**Pure logic only, and deliberately no component tests.** What is covered is the
things several screens depend on agreeing about: the browse URL's encoding, the
latency ladder, the offline cache's allow-list, the two contract edges the
gotchas list names. A React render assertion mostly pins markup in place, and
this app's markup is still moving — the Rust side holds what would most repay
testing and has over four hundred.

**Node, not jsdom.** Nothing under test needs a DOM; the two things that touch
`window` stub the four methods they use. A DOM implementation would be a
dependency carried for tests that never asked for one.

The one test that is genuinely security-relevant is
`lib/api/offline-cache.test.ts`. Its allow-list decides what gets written to
disk in cleartext, and `me`, `log` and `plugins` are excluded by NOT being on a
list — a property a comment cannot enforce.

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

### Adding a game

1. `plugins/app/<slug>/manage_mod.json` — where its mods go. The slug is the
   game's URL segment on the website, lower-cased.
2. `plugins/app/<slug>/launch.json` — how to start it, if it can be started.
3. `plugins/app/<slug>/sandbox.json` — which deployment strategies suit it, its
   presets, its options, and how to FIND it (`detect`).
4. `plugins/app/<slug>/config.json` — where its settings files live, if it has
   any worth editing. Optional; a game without one has no config editor, which
   is the honest answer rather than a guess.
5. Nothing in Rust. If something needs adding in Rust, the file format is
   missing a field rather than the game being special.

`sandbox.json`'s `modTargets` earns its keep twice over now: it is what an
import falls back to when it cannot tell where a payload belongs, and it is the
complete list of folders the adopt scan looks in. A game whose targets are wrong
imports mods to the wrong place and finds nothing to adopt.

The rules under `plugins/app/` are validated by a test that builds each rule's
real jail and resolves every step path through it — and, for `sandbox.json`,
checks that no preset names a strategy the game excludes and that every option a
preset sets survives its own schema. It runs against `AppPlugins::shipped()`,
the compiled-in copy, because that is the one a user actually gets.

**They are compiled into the binary**, so a new game needs a rebuild to appear —
`cargo` reruns the build script when the directory changes. A rule dropped into
`<app data>/plugins/app/` needs no rebuild and overrides a shipped game outright.

### Adding a mod manager

1. `plugins/manager/<id>.json` — its roots per platform, its `modsPath`, its
   game-directory-to-slug table, and its per-mod metadata file if it has one.
2. Nothing else. It is compiled in by `core/build.rs` on the next build, and
   `every_shipped_manager_descriptor_parses` fails the run if it does not load.

A descriptor that finds nothing fails closed and says so, which is the right
failure: the folder names these managers use are their own, and being wrong
about one costs a scan rather than a game folder.

### Adding a deployment strategy

1. A variant on `deploy::Strategy`, its `as_str`/`parse` arms, and its
   `link_kind`.
2. If it places files, a `LinkKind` and an arm in `link::place`. If it does not
   — a second virtualising strategy — it needs its own path in
   `engine::deploy`, and the ledger needs to describe what it did.
3. An arm in `available_strategies`, so the picker can say why it is
   unavailable rather than greying out a row with no explanation.
4. `DeployStrategyVals` in website-city's contract, and the app's mirror.
5. Tests: place, purge, and the "still ours" check that stops a purge deleting
   a file the user edited.

### Adding a step type

1. A variant on `Step` in `plugins/manifest.rs`.
2. An arm in `Executor::run_step`, resolving every path through
   `jail.resolve` and auditing the result.
3. An arm in `step_label` and in `required_roots`.
4. A test. **If the step can write, test that it cannot write outside the jail.**

## Artwork

Every list that names a game shows the game, and every list that names an item
shows the item. That is not decoration on this app in the way it would be on the
website: the site's chrome is built around one chosen game, so a mod card there
is unambiguous without a picture. Here the browse grid mixes Minecraft mods with
Rust servers, the sandbox list holds six games, and the download queue is forty
rows of similar text.

Two components, and the split between them is where the picture comes from:

  * **`GameIcon`** — a game's artwork. `ContentSummary.app.icon`, `facets.apps[]
    .icon` and `Install.app.icon` all carry it; a screen whose data is LOCAL
    (sandboxes, the library, anything out of SQLite) gets it from
    `useAppIcons()`, which shares a React Query key with the browse filters so
    it usually costs no request at all.
  * **`ItemThumb`** — an item's own cover, from `LibraryRow.image`. Never from a
    request: every row in a sandbox's mod list or the download queue is
    something the user subscribed to, so the picture is already on this device,
    and a lookup is a map hit rather than forty fetches in a list that redraws
    every second. `SandboxMod.modKey`, `Download.meta.item` and `LibraryRow.id`
    are all `kind:itemId`, which is what makes the lookup a one-liner.

**Both fall back to something rather than to nothing.** `GameIcon` draws the
game's initials on a tint hashed from its name — the same game is the same
colour in the browser, the sandbox list and the filter picker, because the hash
does not depend on the list's order or contents. `ItemThumb` draws the kind's
glyph. A blank space is worse than no picture: a list where some rows are
indented by an image and some are not reads as broken, not as sparse.

**Neither is ever the only thing carrying the name.** Every caller puts the
label beside it, and both render `aria-hidden` with `alt=""` — the name is
already in the row as text, so announcing it again makes a screen reader read
every row twice. An icon on its own is a guess, and a wrong guess about which
game a mod is for is how somebody installs it into the wrong folder.

**The local database does not cache artwork URLs**, deliberately. It mirrors
what the user owns; a CDN link cached in it would outlive every sync that
changed it, and the lookup costs nothing.

## Gotchas

- **`openUrl` cannot open a `steam://` link, and fails silently.**
  `tauri-plugin-opener`'s capability is scoped to `https://*` deliberately, so
  the webview cannot open a custom scheme registered by something else on the
  machine. The server page's "Join server" button handed `connectUrl` — which is
  always `steam://` or a sibling — straight to it, and was refused every single
  time it was pressed. Anything that is not `https` goes through Rust, where the
  scheme is checked against `plugins::apps::LAUNCH_SCHEMES`.
- **A `const` in `schemas.ts` referenced before its initializer runs is a
  runtime TDZ error, not a type error.** `tsc` is perfectly happy with a schema
  that uses one declared two hundred lines below it, and the app then throws on
  module load with a message about the wrong file. New schemas go BELOW
  everything they reference.
- **`/app-player` has to be in website-city's middleware exclusion list.** It
  lives outside `[locale]` on purpose; without the exclusion next-intl rewrites
  it to `/en/app-player`, which does not exist. The symptom is an empty player
  window, which reads as a broken game rather than a routing rule.
- **A query-string field that is sometimes a list must be in `ARRAY_FIELDS`.**
  `?ids=42` and `?ids=42&ids=43` reach the schema as different SHAPES otherwise,
  so the one-element case fails validation while the two-element case passes —
  invisible in any test that happens to use two values.
- **A `steam://` launch has no play time and must not be given one.** It is a
  `SessionKind::Handoff`: the OS opener started a launcher which started the
  game, and the only duration available is the time between firing a URI and the
  app noticing. That number describes the app. `Session::seconds` returns `None`
  for one and every caller has to decide what to draw — `0` is the wrong answer
  everywhere except the database column.
- **`Child::kill` is unreachable while the supervisor is in `wait`.** It needs
  `&mut Child` and the supervisor holds that lock for the whole wait, which is
  why `Sessions::stop` signals by pid. That is sound in exactly that position —
  a process inside `wait` has not been reaped — and is a bug anywhere else.

- **A native `<select>`'s popup is drawn by the OS and cannot be styled.** The
  closed control takes CSS; the open list takes none of it, so every settings
  pane in a dark theme had one white rectangle in it. `components/select.tsx`
  is a real listbox and every `<select>` in the app is gone. Adding one back
  brings the white rectangle with it.
- **`plugins::jail` is the path jail; a `sandbox` is a user's mod profile.**
  They were both called `Sandbox` once. If a new type wants either name, it
  wants the other one.
- **A sandbox is an `AppInstall` on the wire.** The cloud model predates the
  word and `/api/app/v1/installs` is shipped; renaming the endpoint would 400
  every request from every installed copy of the app.
- **Epic's `bIsIncompleteInstall` is the one field in its manifest that is not
  PascalCase.** `rename_all = "PascalCase"` derives `BIsIncompleteInstall`,
  which never matches, which reads as "no game is ever incomplete" and offers a
  folder that is still being written into. It carries an explicit `rename`.
- **Steam's `installdir` is not the display name.** `Grand Theft Auto V` lives
  in `steamapps/common/GTAV`. Using the name is the single most common way to
  look in the wrong place.
- **A manifest whose folder is gone is not an installed game.** Steam leaves
  them behind after a failed uninstall and after moving a game between
  libraries; offering one produces a game directory that fails every check the
  moment somebody accepts it.
- **GOG Galaxy's database may be open.** It is read with `mode=ro&immutable=1`;
  read-only alone still takes a shared lock and still reads the WAL, so a
  Galaxy mid-write either blocks the read or hands back a torn one.
- **A `200` in answer to a `Range` request means the server ignored it.**
  Appending that to a `.tmcpart` produces a file that is too long, passes every
  length check, and fails its checksum with an error nobody can explain. The
  file restarts instead.
- **A download's checksum is computed from the FILE, not incrementally.** An
  incremental hash cannot survive a resume — the bytes from the first attempt
  never pass through this process — and a checksum that silently stops being
  checked on resumed downloads is worse than none.
- **Frostbite's packet `size` counts the two header fields it is part of.**
  Omitting them leaves four bytes of every packet in the buffer, which presents
  as "the second command always fails".
- **A merge tree built from an unsorted list is wrong in a way that looks
  right.** `merge::build` sorts internally; the last mod added wins instead of
  the highest priority, which nobody notices until they reorder their list and
  nothing changes.
- **`pluginData` is scoped per sandbox and per mod.** Two sandboxes staging the
  same mod run identical steps with an identical `{fileName}`. With one shared
  scratch directory the second run's download lands on the first's.
- **`tokio::spawn` inside a function that the spawned task calls back into is
  an infinitely large future type.** The download queue's `pump` → `run_one` →
  `pump` cycle is broken with one `Box<dyn Future>`; without it the compiler
  reports "cannot satisfy `impl Future: Send`", which does not obviously mean
  "you wrote a recursive future".

- **A query string carries no types, so `parseQuery` does not guess.** Values
  arrive as strings and the contract coerces per field (`z.coerce.number()`,
  `QueryBool`). Guessing from shape is the obvious approach and it silently
  broke `?search=2024`, `?search=true` and — because every cursor is a numeric
  id — the second page of every listing.
- **Version comparison is numeric per component, and refuses what it cannot
  order.** `1.10.0` sorts BEFORE `1.9.0` as a string, so an update banner built
  on a string compare either never appears or never goes away. `tmc_core::
  version::is_newer` parses dotted components and returns false for anything
  non-numeric — staying quiet beats nagging somebody toward a version they
  already have. It lives in `tmc-core` and **not** in `commands/api.rs`, where
  it used to: the game installer asks the same question about a build on disk,
  and a second copy would be a second place for that trap to be re-entered.
- **`meets_minimum` is not `!is_newer(want, have)`.** That expression answers
  true when EITHER side is unorderable, which would wave through exactly the
  build that declared a `minClientVersion`. An unorderable requirement is
  refused; an unorderable own version passes, because a development build has no
  business being told it is too old.
- **A game build's launch `args` is a VECTOR, not a command line.** There is no
  shell in this app to split one with, so a server name containing a space would
  become two arguments. Substitution happens inside an element and can never
  create one — and an argument whose value is missing is dropped along with the
  flag before it, because `["--connect", ":"]` is an address the game will try.
- **An archive that unpacked without the file it named is an install that
  reports success and then cannot be started.** `games::entry_path` resolves the
  declared entry through `join_relative` and checks that it is a file, because
  neither the extraction nor the checksum can notice a publisher naming the
  wrong path — and the symptom arrives days later as "the Play button does
  nothing".
- **`z.coerce.boolean()` is `Boolean(value)`**, so the string `"false"` is
  `true`. Use `QueryBool` from the contract.
- **`serde(rename_all = "SCREAMING_SNAKE_CASE")` turns `A2S` into `A2_S`.** It
  splits between a digit and the letter after it, so the one protocol whose name
  contains a digit mid-word gets a wire name nothing uses. Every Source server
  then failed to deserialise — and because `query_servers` takes a
  `Vec<QueryRequest>`, one such row rejected the WHOLE batch, so a screenful of
  servers showed `TO` because one of them ran CS2. `QueryProtocol::A2S` carries
  an explicit `#[serde(rename = "A2S")]`, and
  `every_protocol_round_trips_under_its_wire_name` checks all eighteen against a
  hardcoded list. Reading the enum will not catch this: the variant names look
  identical to the wire names.
- **Tailwind 4 generates a utility only for a token that EXISTS.** `bg-surface-2`
  was written in seven places and produced no background at all, because the
  shared theme calls it `--surface-secondary` — a bug that looks exactly like a
  design choice, since the panel simply sits flat against its parent.
  `app.css`'s `@theme inline` block aliases `--color-surface-2` to it. Before
  inventing a colour name, check it against `@modcommunity/shared`'s
  `src/styles/theme.css`.
- **Unlayered CSS in `app.css` beats Tailwind's `@layer utilities`,** whatever
  the specificity — that is the cascade's layer rule, not a specificity contest.
  A `button { text-transform: inherit }` added to fix one header silently
  overrode `uppercase` on every button in the app. Resets in that file must be
  properties no utility sets.
- **`-webkit-app-region: drag` is a no-op in WebKitGTK and WKWebView.** It is a
  Chromium extension, so it appears to work on Windows and silently does nothing
  on the two platforms the custom titlebar most needs. Use
  `data-tauri-drag-region`.
- **The offline cache is an ALLOW-list, and the timestamps it restores are the
  original ones.** `lib/api/offline-cache.ts` stores `browse`, `content`,
  `facets` and `reviews` — public catalogue data — into `localStorage`, and puts
  them back at boot with the `dataUpdatedAt` they had. Restoring them as fresh
  would suppress the refetch and pin the app to whatever it last saw. It is an
  allow-list because the failure modes are not symmetrical: a new query holding
  account data that nobody remembered to exclude gets written to disk in
  cleartext, while a new public one that nobody remembered to include costs a
  spinner. `me`, `log` and `plugins` are all excluded by not being on it.
- **Every browse filter lives in the URL, never in component state.** A
  filtered browse has to survive a reload, a deep link and the back button, and
  a HashRouter over a static bundle has nowhere else durable to put it.
  `browse.tsx`'s `param`/`flag`/`num`/`idList` are the only place that encoding
  is decoded; multi-selects are comma lists.
- **`sort=players` is a deprecated alias of `curUsers`.** The app invented the
  name; the website has always called it `curUsers`. It stays in the contract's
  enum because an installed build is not redeployed with the server and would
  otherwise 400 on every server browse — but it is never offered in the sort
  dropdown.
- **The server browser defaults to the table, every other kind to the grid.**
  `?view=grid` / `?view=table` overrides it, and `view` is only honoured for
  `kind === 'server'` — nothing else has a table to switch to.
- **The server browser's defaults are the website's**, from
  `lib/user/settings/default.ts`: sort by player count descending, online only,
  table view. Sorting servers by "newest" instead means 2.6 million rows, nearly
  all of them freshly imported and never once seen online — the browser looked
  broken because every row read `TO`, and it was right to.
- **An app ref carries the game's FULL name, never `App.nameShort`.** The site
  abbreviates because its chrome is built around one chosen game; the app has no
  such chrome and shows a flat list of every game in the catalogue, where a
  `nameShort` that is unset — or set to `''`, which `??` happily returns — is a
  blank, selectable row in the filter's game picker. `mapApp` in website-city's
  `lib/app-api/content.ts` and the `/facets` handler both send `App.name`, and
  `lib/api/labels.ts`'s `appLabel` is the display-side guard for an older server
  or a genuinely empty name. Use it wherever a game is named on screen.
- **Ping is the FIRST column in the table**, as it is on the website. The table
  is wider than its pane and scrolls horizontally, so any column on the right can
  be off-screen — and the one the app exists to provide must never be the one
  that disappears.
- **A live `maxPlayers` is only believed when `>=` the live player count.**
  Several games report A2S `max_players` as something other than capacity — Rust
  answers ignoring the queue, which rendered as `144/51`. The measured PLAYER
  count always wins; only the slot count falls back to the API's.
- **Articles are opened in the system browser, not rendered.** `lib/external.ts`
  holds the list. Their bodies are laid out for the website's content column and
  the app's markdown subset strips exactly the parts carrying that layout.
- **The live-query context's accessors are stable; `version` is the render
  signal.** Anything that puts a context accessor in an effect dependency list
  re-runs that effect on its own result — which, for the server panel, was an
  unbounded query loop against a real game server.
- **Brand marks come from `react-icons/fa6`, never lucide.** lucide dropped its
  brand glyphs; `Github` / `Twitter` / `Facebook` survived only as deprecated
  aliases and are gone from ~0.475. `@modcommunity/shared` imported all three,
  which is why this package pinned `lucide-react` to `0.468.0` to build at all.
  Fixed in `shared@4.2.3`, so the pin is gone and lucide tracks `^0.542.0` with
  the other repos. lucide stays the house icon set for everything else.
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
- **An app rule that is not compiled into the binary supports nothing, quietly.**
  `plugins/app/` is embedded by `core/build.rs` rather than bundled as a Tauri
  resource, because the portable Windows exe and a relocated AppImage carry no
  resources. With an empty rule set the scan finds no games, nothing launches,
  no sandbox has a strategy and the config editor lists nothing — and not one of
  those paths reports an error, because an empty map is a valid answer to all of
  them. `the_compiled_in_rules_match_the_repository` is what makes the omission
  loud.
- **A `.jar` is a zip, and a Minecraft resource pack is a `.zip`.** Neither may
  be unpacked on import, and both would be by any rule that keys off "is this an
  archive". The game's own `manage_*` rules decide, through
  `AppPlugins::mod_extensions` — a file whose extension a rule accepts is copied
  verbatim. Inventing a second extension list here is how a resource pack
  becomes a folder of loose textures the loader ignores.
- **An archive that already holds `mods/` is a slice of the game folder.**
  Nesting it under the game's mod target again produces `mods/mods/foo.jar`,
  which deploys, reports no error and does nothing. `store::suggest_rel_path`
  tests the payload's top level against the game's declared `modTargets` before
  it nests anything.
- **`store::relayout` cannot rename in place.** Moving a payload from the game's
  root into `mods` is a rename of a directory into itself, which every platform
  refuses — and the root-to-subdirectory case is the FIRST one a user hits,
  because it is what an unrecognised archive gets. It goes via a sibling
  holding directory, and puts the files back if the second rename fails.
- **`serde(rename_all)` does not rename variant fields.** Enum variants in the
  plugin manifest need `rename_all_fields = "camelCase"` too, or a manifest
  would have to write `max_bytes`.

## Not built yet

Honest list, so nothing here reads as finished when it is not:

- **Self-updating.** The app CHECKS — `/api/app/v1/version` against two
  `SiteSetting` rows, once per launch when `autoUpdateCheck` is on, with a
  banner offering the download page in the user's browser. It does not install
  anything. A real updater needs `tauri-plugin-updater`, a signing key held by
  whoever cuts releases, and a manifest endpoint; shipping the client half
  against none of those would be a feature naming a capability it does not have,
  with a silently-installed binary as the consequence.
- **Writes.** The app is read-only against the API for publishing — no
  commenting or uploading. Reviews, review votes, reports, subscriptions,
  sandboxes and play-time reports DO write.
- **Native builds to actually install.** The whole path exists on both sides —
  `AppNativeBuild`, `/apps/:id/build`, the installer, the updater and the
  Library view — and nothing has published a row yet, because
  `dot-server-deploy/export_presets.cfg` has only a `Web` preset. Until a
  desktop preset exists and `scripts/publish-native-build.ts` has been run
  against its output, the TMC Games tab correctly says there is nothing
  published for this machine.
- **A measurable virtual launch.** A USVFS launch is recorded as a
  `SessionKind::Handoff` — not because nothing of ours started it, but because
  `inject::launch` does not return the process handle, so there is no exit to
  observe and no duration to measure. Fixing that means a change inside the one
  part of the tree that has never been run against a game.
- **A schema-driven config form.** The config editor is a text box, which
  round-trips every format exactly — see "Editing a game's own settings" for
  why that is the version that is never wrong. A typed form per config dialect
  would be nicer where it worked, and would need one implementation per dialect
  and a story for what happens when a file does not match the one it assumed.
- **USVFS injection, proven.** The strategy is implemented end to end and gated
  behind `usvfs-hooks`, off by default. The tree and the blob are tested on
  every platform; the two hundred lines that patch an import table in somebody
  else's game process are type-checked against the Windows target and have never
  been run against a game. Turning the feature on is a decision to find out.
- **Importing from a mod manager's API.** Every shipped manager descriptor
  reads local storage, which is where the answer actually is — see "Reading
  another mod manager" for why CurseForge, Nexus and Thunderstore cannot answer
  "what has this user installed" over HTTP. If one ever does, the descriptor
  format gains a field and the built-in list gains a row; the import path,
  which takes "a name and a folder", does not change.
- **Mod Organizer 2.** Its instances are named freely and the game is recorded
  inside `ModOrganizer.ini`, so no declarative name match is reliable. A
  descriptor that guessed would find the wrong game's mods. Fixing it means the
  descriptor format learning to read a game id out of a per-instance file, which
  is a real feature rather than another row in `plugins/manager/`.
- **A shipped manager descriptor verified against a real install.** The layouts
  are from each manager's documented defaults and the parsing is tested against
  fixtures, but no descriptor here has been run against an actual Vortex or
  CurseForge installation. A wrong folder name fails closed — the scan finds
  nothing — and a corrected descriptor dropped in `plugins/manager/` is the
  whole fix, which is why this is data.
- **Plugin distribution.** Plugins install from a local folder and there is no
  registry to fetch them from. Signature *checking* is implemented and
  `requireSignedPlugins` enforces it, but nothing is published to be checked
  yet. Signing is `examples/plugin-sign`; what is missing is somewhere to
  publish the result and a key distribution story better than "paste this hex
  string".
- **Proof that a human chose a jail anchor.** `anchor::validate_root` decides
  whether a *directory* is an acceptable jail anchor, which is the enforceable
  half. The other half — that the path came from a real click rather than from
  script — died with the native dialog and cannot be recovered while the picker
  is drawn by the app. Restoring it needs an OS-level confirmation the webview
  cannot forge.
- **Protocols left on `TCP_ONLY`.** `DISCORD`, `GTA_NETWORK`, `GTA_RAGE`,
  `SCUM` — and unlike the rest of this list, they are not waiting to be
  written. Each is one the scanner speaks by asking a third party rather than
  the server, which the section above explains cannot be done from a user's
  device. `commands/servers.rs`'s `UNIMPLEMENTED` test constant names one of
  them, so a native implementation fails two tests on purpose.
- **No component or end-to-end tests on the frontend.** `vitest` covers the
  pure logic (see above); nothing renders a component or drives the real app.
  The webview has never been exercised by anything but a person.
- **No end-to-end test against a real game server.** The protocol parsers are
  covered by golden-reply and fuzz-shaped unit tests; the socket paths above
  them have been exercised only against the bounds checks, not a live box. The
  same is true of RCON: the codecs are tested, the sockets are not.
- **No end-to-end test of a real deploy against a real game.** The deployment
  engine is tested against temporary directories — including the hard-link and
  symlink paths where the platform supports them — but nothing has been linked
  into an actual Steam folder and launched.
- **The download queue's own network path IS tested**, against a loopback HTTP
  server: resume, the ignored-range trap, checksum rejection, a dropped
  connection retrying, cancel and pause. That is the exception, not the rule.

---

## Prior art

This app is not the first mod manager, and the comments throughout it name the
others on purpose: Mod Organizer's virtual filesystem, Vortex's hard-link
deployment, r2modman's and CurseForge's per-profile instances, Gale's config
form. Where one of them settled a question well, the reasoning is credited to
them at the point it applies, so the next person to read that code learns why
the shape is what it is rather than assuming it was arbitrary.

**Credited, not copied.** Every line in this repository is written for it.
What is taken from other projects is the same thing any engineer takes from
published work — the knowledge that a problem exists and roughly how it tends
to be solved — and that is exactly what the comments record. Three specific
categories are worth separating, because they are treated differently:

  * **Protocol and file-format behaviour** is fact about somebody else's wire
    format or on-disk layout, and the comments name the authority that settled
    it — `spy`'s scanner for query ports, `go-a2s` for split-reply framing,
    each game's own modding documentation for where its mods go. A format is
    not authorship, and getting it wrong is a bug in this app.
  * **Folder layouts in `plugins/manager/`** are the documented defaults each
    manager publishes for its own users. They are data in this repository
    because they belong to those projects and change on their schedule — a
    corrected descriptor is a JSON edit, not a patch to Rust.
  * **Nothing is vendored.** No third-party source is copied into this tree, and
    nothing here is a derivative work of another manager. Dependencies are
    declared in `package.json` and the `Cargo.toml`s, under their own licences.

If a comparison in a comment ever reads as a claim on somebody else's work
rather than a note about why this code looks the way it does, rewrite it. The
projects named here are the reason a lot of this was tractable, and the
attribution is meant to say so.
