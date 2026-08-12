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
| `../spy` | The Go scanner that populates every server row. **The authority on query ports and protocol quirks** — when this app and the site disagree about how to reach a server, `spy` is what settles it. |
| `../tmc-global` | `@modcommunity/shared` — design tokens and a few UI primitives, consumed from GitHub Packages. `npm run shared:local` swaps in the sibling checkout. |
| `../website-processing` | Astro landing site. No relationship to this app beyond sharing the design system. |

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
| `api.rs` | HTTP. `api_base()` — compile-time, or `TMC_API_BASE` in a debug build. Never a setting |
| `auth.rs` | PKCE, device-grant state, in-memory access token |
| `secure.rs` | Keychain / Credential Manager / Secret Service, file fallback on mobile. Keyed per API base |
| `settings.rs` | App-local settings (`settings.json`), clamped on read |
| `anchor.rs` | **What a sandbox root may be.** Guards `gameDirs` / `downloadDir` |
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
| `commands/fs.rs` | Directory listing for the app's folder picker. Names only, never contents |
| `state.rs` | `AppState`, assembled once — shared locks and caches depend on that |
| `paths.rs` | Every path, from Tauri's resolver — never `$HOME` |

**`src/`**

| Path | Owns |
| --- | --- |
| `lib/ipc/` | `call()` + schemas + `ipc.*`. The only place `invoke` is imported |
| `lib/api/contract.ts` | **Mirror** of website-city's contract. `npm run contract:sync` |
| `lib/api/client.ts` | `api.*`, every response zod-parsed |
| `lib/api/labels.ts` | How a game is named on screen — always its full name |
| `lib/api/env.ts` | Which site this build talks to, for display and for the site's own links |
| `lib/auth/provider.tsx` | Login state and the poll loop |
| `lib/settings/provider.tsx` | App settings + account settings, kept apart |
| `lib/hooks/use-breakpoint.ts` | Layout decisions, keyed on the window |
| `lib/hooks/use-platform.ts` | The few decisions that genuinely are per-OS, not per-window |
| `lib/hooks/use-live-query.tsx` | The live-server registry: one timer, one batch |
| `lib/external.ts` | Which content kinds are handed to the system browser |
| `components/shell.tsx` | Sidebar ≥768px, bottom tabs below |
| `components/titlebar.tsx` | The app's own window frame — see "Cross-platform" |
| `components/folder-picker.tsx` | The in-app folder chooser, over `commands/fs.rs` |
| `components/markdown.tsx` | The safe renderer for untrusted bodies |
| `components/latency-graph.tsx` | Sparkline + full chart + the latency ladder, hand-rolled SVG |
| `components/server-live.tsx` | The live strip on a server card |
| `components/server-table.tsx` | The server browser's default view: table, expandable rows |
| `components/browse-filters.tsx` | The filter panel: collapsible groups, kind-aware, URL-backed |
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
**SCUM → game + 2**. `resolve_port` carries both with the Go file named.

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

### Two of them are not settings at all

`gameDirs` and `downloadDir` anchor the plugin jail — `plugins::sandbox`
resolves every `PathRef` beneath them, so an installer holding
`{gameDir, "", write}` can write anywhere below. They are therefore the only
fields with their own commands:

- **`settings_patch` refuses them** (`SANDBOX_ROOT_FIELDS`), loudly rather than
  by dropping the key. The refusal is in `SettingsStore::patch`, so it holds for
  every caller and not just the one command.
- **`settings_set_game_dir` / `settings_set_download_dir`** run
  `anchor::validate_root`, which rejects a drive root, a system directory, and
  anything that *contains* the app's data, logs, cache, plugins or the user's
  home. A jail anchored above the app's own files would enclose `settings.json`
  and the plugin registry — a plugin that can rewrite the registry can grant
  itself permissions.
- The **canonical** path is what gets stored and what gets audited, so the value
  in `settings.json` is the one the sandbox will resolve to later.
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

### Pointing a build at the dev site

```bash
TMC_API_BASE=https://tmcdev.net:3002 npm run desktop   # debug: read at RUN time
TMC_API_BASE=https://tmcdev.net:3002 npm run android   # baked in at BUILD time
```

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
- **Unlayered CSS in `app.css` beats Tailwind's `@layer utilities`,** whatever
  the specificity — that is the cascade's layer rule, not a specificity contest.
  A `button { text-transform: inherit }` added to fix one header silently
  overrode `uppercase` on every button in the app. Resets in that file must be
  properties no utility sets.
- **`-webkit-app-region: drag` is a no-op in WebKitGTK and WKWebView.** It is a
  Chromium extension, so it appears to work on Windows and silently does nothing
  on the two platforms the custom titlebar most needs. Use
  `data-tauri-drag-region`.
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
- **Proof that a human chose a sandbox root.** `anchor::validate_root` decides
  whether a *directory* is an acceptable jail anchor, which is the enforceable
  half. The other half — that the path came from a real click rather than from
  script — died with the native dialog and cannot be recovered while the picker
  is drawn by the app. Restoring it needs an OS-level confirmation the webview
  cannot forge.
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
