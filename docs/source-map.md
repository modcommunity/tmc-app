# Source map

What each file owns, per crate. The shape and the rule behind it are in [architecture.md](architecture.md).

**`src-tauri/core/src/` — `tmc-core`**

| File | Owns |
| --- | --- |
| `api.rs` | HTTP. `api_base()` — compile-time, or `TMC_API_BASE` in a debug build. Never a setting |
| `auth.rs` | PKCE, device-grant state, in-memory access token |
| `secure.rs` | Keychain / Credential Manager / Secret Service, file fallback on mobile. Keyed per API base |
| `settings.rs` | App-local settings (`settings.json`), clamped on read |
| `updater.rs` | **Where app updates come from and which key they need.** Override → compiled → fallback |
| `anchor.rs` | **What a jail anchor may be.** Guards `gameDirs` /`downloadDir` |
| `canon.rs` | Resolving a path, in the spelling everything else uses. The one door to `canonicalize` |
| `crypto.rs` | The device key, and what it does and does not buy |
| `deeplink.rs` | The closed list of what a `tmc://` link may ask for |
| `logging.rs` | Append-only JSONL audit log + `audit!` macro |
| `net/addr.rs` | **The public-address guard.** Resolve once, connect to that |
| `net/transport.rs` | Bounded UDP/TCP exchanges — every read has a deadline and a cap |
| `net/reader.rs` | The non-panicking byte cursor every parser is built on |
| `net/ping.rs` | TCP connect timing, for games with no protocol |
| `net/latency.rs` | Rolling per-server history, bounded on both axes |
| `net/query/` | The game protocols — [live-queries.md](live-queries.md) |
| `plugins/manifest.rs` | The manifest format and its validation |
| `plugins/jail.rs` | **The path jail.** Called a jail, not a sandbox — [sandboxes.md](sandboxes.md#sandboxes-and-how-mods-reach-the-game) |
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
| `detect/vdf.rs` | VDF KeyValues, bounded on every axis |
| `rcon/source.rs` | Source RCON, which many other servers also speak |
| `rcon/frostbite.rs` | Frostbite R-CON, with `login.hashed` |
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
| `components/shell.tsx` | Sidebar ≥768px, bottom tabs below; draws the tree in `lib/nav.ts` |
| `lib/nav.ts` | The primary navigation as data: Play, Discover (Apps, Mods, Assets, Servers > RCON, More), Library (Sandboxes, Downloads, Stores), Friends, Console, Settings. Feeds the rail, the bottom bar and the landing cards |
| `components/section-cards.tsx` | The card grid on the Discover and Library landing pages |
| `components/titlebar.tsx` | The app's own window frame — [frontend.md](frontend.md#the-window-frame-is-the-apps-not-the-oss) |
| `components/update-banner.tsx` | "There is a newer version", with an Install button when this build can verify one |
| `components/updater-settings.tsx` | Settings → App → Updates: channel, check now, and the endpoint/key overrides shown read-only |
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
| `routes/game.tsx` | **One game** (`/apps/:id`): play, its busiest servers, leaderboards and the member's own figures |
| `routes/friends.tsx` | Friends, requests, presence, party invites and the current party — the routes a Dot game's `dot-friends`/`dot-party` speak |
| `lib/api/social.ts` | UI names + helpers (`presenceLabel`, `formatStat`, `isStatKey`) for the friends/party/presence/stats routes; their schemas are the contract's "Social" section |
| `lib/gamespice/store.ts` | Store helpers: `itemState` (buy, short, owned, checkout on the website, closed...), `filterItems`, `slotFor`/`slotLabel`, `equippedSlot`, `newIdemKey` for points purchases |
| `lib/gamespice/stats.ts` | Stats helpers: streaks, `playShare`, `completion`, `sparkPath`, day and last-played labels |
| `components/gs-charts.tsx` | The GS-theme charts the Stats screens draw: activity bars, share bar, completion ring, sparkline |
| `routes/store.tsx` | Store directory, per-game store (search, kind/currency/show filters, sort), item dialog (points buy with confirm, credits open the website externally, equip), loadout panel, inventory tab |
| `routes/stats.tsx` | Stats overview (totals, 30-day activity, play share, per-game cards) and per-game drill-down (my stats, achievements, leaderboards) |
| `components/add-friend-button.tsx` | "Add friend" on a member's page |
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
