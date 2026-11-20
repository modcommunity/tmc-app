# CLAUDE.md — tmc-app (product name: GameSpice App)

The official **The Modding Community** app: Rust + Tauri 2 + React 19, shipping to Windows, macOS, Linux, Android and iOS from one codebase. It browses what the website holds (assets, mods, servers, maps, articles, communities, collections, members) and adds what a browser tab cannot do: one-click installs through declarative plugins, sandboxes (mod profiles deployed by link, copy or a virtual filesystem), live server queries and real latency from the user's own device, a download queue, RCON, finding installed games, a catalogue with a Play button, and play sessions. It is not a wrapper around the website: no landing page, no marketing chrome; it opens on a browser and stays there.

**This file is the always-loaded summary. The detail is in [docs/](docs/README.md)** — open the file for what you are touching; do not work from this file alone.

**This repository is public and GPL-3.0-only.** A comment is published, and a vulnerability report arrives from a stranger. [SECURITY.md](SECURITY.md) states the threat model and lists what looks like a hole and is deliberate.

## Read before

| Read | Before |
| --- | --- |
| [docs/README.md](docs/README.md) | the index |
| [docs/architecture.md](docs/architecture.md) | anything that crosses the IPC bridge, or adds code to a crate |
| [docs/source-map.md](docs/source-map.md) | looking for where something lives |
| [docs/api.md](docs/api.md) | auth, the contract mirror, any `/api/app/v1` call, browse filters |
| [docs/settings-and-logging.md](docs/settings-and-logging.md) | adding a setting, touching `gameDirs`/`downloadDir`, writing to the audit log |
| [docs/frontend.md](docs/frontend.md) | layout, the window frame, game/item artwork, frontend tests |
| [docs/live-queries.md](docs/live-queries.md) | anything under `net/`, query ports, protocols, latency display |
| [docs/plugins.md](docs/plugins.md) | the plugin format, the jail, signatures, `plugins/app/` rules |
| [docs/sandboxes.md](docs/sandboxes.md) | the library database, sync, sandboxes, deployment strategies, USVFS, the ledger, share codes |
| [docs/local-mods.md](docs/local-mods.md) | imports, adoption, reading another mod manager, `tmc.json` |
| [docs/config-editor.md](docs/config-editor.md) | the game-settings editor and `config.json` |
| [docs/playing.md](docs/playing.md) | launching anything, the web player, TMC-published game builds, launch arguments, play sessions |
| [docs/updater.md](docs/updater.md) | the self-updater and its key |
| [docs/downloads.md](docs/downloads.md) | the download queue |
| [docs/detection.md](docs/detection.md) | finding installed games |
| [docs/rcon.md](docs/rcon.md) | RCON |
| [docs/deep-links.md](docs/deep-links.md) | `gs://` |
| [docs/development.md](docs/development.md) | commands, the dev site, Linux without root, CI, `audit:rust` |
| [docs/BUILDING.md](docs/BUILDING.md) | producing a platform artifact |
| [docs/RELEASING.md](docs/RELEASING.md) | cutting a release |
| [docs/extending.md](docs/extending.md) | adding a content kind, query protocol, game, mod manager, deployment strategy or step type |
| [docs/gotchas.md](docs/gotchas.md) | declaring anything done |
| [docs/history.md](docs/history.md) | a bug that "cannot be happening" |
| [docs/gaps.md](docs/gaps.md) | planning; what is deliberately not built yet ("Not built yet") |
| [docs/prior-art.md](docs/prior-art.md) | naming another mod manager in a comment |

## Sibling repositories

| Repo | Relationship |
| --- | --- |
| `../website-city` | The Next.js site. **Owns the API this app speaks** (`/api/app/v1`) and the auth flow; changes there usually need a matching change here. |
| `../spy` | The Go scanner behind every server row. **The authority on query ports and protocol quirks**: when this app and the site disagree about reaching a server, `spy` settles it. |
| `../tmc-global` | `@modcommunity/shared` — design tokens and UI primitives, from GitHub Packages. `npm run shared:local` swaps in the sibling checkout. |

## The one rule

> **Credentials and the filesystem live in Rust. The webview gets data.**

There is no `fetch` in the frontend and no `get_token` command. Every layer assumes an attacker already has script execution in the webview (a mod description is untrusted text next to `invoke`) and asks what they can reach. The webview names **ids and tokens, never URLs or paths**: `fs_list_dirs` (directory names only, for the folder picker) is the one command that takes a path. Details: [docs/architecture.md](docs/architecture.md).

## Layout

| Path | What |
| --- | --- |
| `src-tauri/core/` | `tmc-core`: everything privileged and testable — auth, API client, net and protocols, plugins and the jail, deploy, library, downloads, detect, RCON, launch, sessions. **No Tauri dependency** |
| `src-tauri/usvfs/` | `tmc-usvfs`: the virtual filesystem and its Windows-only injector; its own crate so it can be `cargo check`ed for Windows from Linux |
| `src-tauri/src/` | the Tauri shell: `commands/` (the whole IPC surface), `spawn.rs` (the only place a process starts), `state.rs`, `paths.rs` |
| `src/` | React: `lib/ipc/` (the only `invoke`), `lib/api/` (contract mirror and client), `routes/`, `components/` |
| `plugins/app/<slug>/` | per-game rules (`manage_*`, `launch.json`, `sandbox.json`, `config.json`), **compiled into the binary** by `core/build.rs` |
| `plugins/manager/` | descriptors for other mod managers' folders, compiled in the same way |
| `examples/plugins/` | example plugin bundles, validated by two Rust tests |
| `scripts/` | build, release, installer, sysroot and publishing scripts |
| `packaging/windows/tmc.wxs` | the MSI, written for the subset `wixl` implements |

## Running and testing

- `npm run dev` is vite only (UI work); `npm run desktop` is `tauri dev`; `npm run check` runs everything and is the definition of done. `npm run test:core` needs no GTK stack. Full list: [docs/development.md](docs/development.md).
- `npm run contract:sync` re-copies the API contract from website-city; never hand-edit `src/lib/api/contract.ts`.
- Against the dev site: `TMC_API_BASE=https://tmcdev.net` — port 443, not 3002. Read at run time in a debug build, baked in at build time otherwise.
- Without root on Linux: `npm run sysroot`, then `source ./scripts/linux-env.sh`.
- Windows builds come out of Linux (`npm run build:windows`); macOS only from a Mac or CI. [docs/BUILDING.md](docs/BUILDING.md).
- A release is a pushed `v*` tag matching the version in `tauri.conf.json`, `package.json` and `Cargo.toml`. [docs/RELEASING.md](docs/RELEASING.md).

## Rules that bite

Short forms; the reasoning is in the linked file.

**Security boundary**

- No command takes a URL or a path from the webview: `download_release`, `play_open_web`, `games_*` and the import commands take ids or opaque tokens and Rust resolves the rest ([architecture.md](docs/architecture.md), [local-mods.md](docs/local-mods.md#the-webview-still-never-names-a-path)).
- A new privileged capability is a new `#[tauri::command]` with the policy check inside it, not in the caller ([architecture.md](docs/architecture.md#rust)).
- `api_base()` is never a setting and never a webview argument; each base keeps its own stored session ([development.md](docs/development.md#pointing-a-build-at-the-dev-site)).
- `gameDirs`/`downloadDir` anchor the plugin jail: `settings_patch` refuses them; only their own commands set them, through `anchor::validate_root` ([settings-and-logging.md](docs/settings-and-logging.md#two-of-them-are-not-settings-at-all)).
- `updaterEndpoint`/`updaterPubkey` have no setter, and must not get one ([updater.md](docs/updater.md#the-signature-is-the-entire-security-model)).
- Security-level audit entries are written even with logging off; never put a token or credential in `audit!` data ([settings-and-logging.md](docs/settings-and-logging.md#logging)).
- A `gs://` link may SHOW something, never DO something; an unrecognised link does nothing ([deep-links.md](docs/deep-links.md)).
- The web player runs on the site's origin with no IPC; its window stays decorated ([playing.md](docs/playing.md#the-web-player-runs-on-the-sites-origin-and-that-is-the-isolation)).
- RCON is the one deliberate SSRF gap; everything else resolves through `net::addr::resolve_public` ([rcon.md](docs/rcon.md)).

**Plugins and deployment**

- A plugin is data, never code: no step may run a program, name an absolute path or read the environment ([plugins.md](docs/plugins.md#the-step-vocabulary)).
- Adding a game is four JSON files and no Rust ([extending.md](docs/extending.md#adding-a-game)); the rules are compiled in, so a new one needs a rebuild ([plugins.md](docs/plugins.md#where-they-come-from-and-why-they-are-compiled-in)).
- `plugins::jail` is the path jail; a sandbox is a user's mod profile; on the wire a sandbox is an `AppInstall` ([sandboxes.md](docs/sandboxes.md)).
- Nothing in a game folder is overwritten (moved to backup first) and nothing is removed unless the ledger proves it is still ours ([sandboxes.md](docs/sandboxes.md#three-properties-that-hold-across-every-strategy)).
- Falling back between link strategies is automatic; falling back to copying never is; USVFS never falls back at all ([sandboxes.md](docs/sandboxes.md#the-four-strategies)).
- `antiCheat: "kernel"` forces Direct and refuses injection ([sandboxes.md](docs/sandboxes.md#sandboxjson)).
- A server upsert never touches device-local columns; local mods and installed games are their own tables because a full sync deletes what the server did not send ([sandboxes.md](docs/sandboxes.md#the-library), [playing.md](docs/playing.md#why-a-game-is-not-a-subscription)).

**Network and protocols**

- Query port: explicit `queryPort`, else the GAME port. `portOffset` and `swapGamePort` are not inputs ([live-queries.md](docs/live-queries.md#which-port-gets-queried-and-why-not-the-other-two)).
- Parsers under `net/query/` never index or slice, bound every loop and length, and each protocol has a truncated-prefix test; a panic is a remote kill switch (`panic = "abort"`) ([live-queries.md](docs/live-queries.md#rules-for-anything-under-netquery)).
- `QueryProtocol` variants with a digit need an explicit serde `rename` ([gotchas.md](docs/gotchas.md#rust-and-tauri)).
- Downloads go through the queue with its own HTTP client (never the API client's bearer); the final filename appears only after the checksum ([downloads.md](docs/downloads.md)).

**Launching**

- Launch `args` are a vector: substitution never creates an element, and a missing value drops its flag; a Godot export needs the bare `--` ([playing.md](docs/playing.md#the-launch-arguments-are-a-vector-never-a-command-line)).
- A `steam://` launch is a `Handoff` with no play time; never invent one ([playing.md](docs/playing.md#play-sessions)).
- Non-`https` links go through Rust and `LAUNCH_SCHEMES`; `openUrl` refuses them silently ([gotchas.md](docs/gotchas.md#typescript-css-and-the-build)).
- Versions compare with `tmc_core::version`, never as strings ([gotchas.md](docs/gotchas.md#versions)).

**Frontend**

- `any` is banned; every IPC call and every API response is zod-parsed; only `src/lib/ipc` imports `invoke` ([architecture.md](docs/architecture.md#type-safety)).
- Browse filters live in the URL, decoded only in `lib/api/query-params.ts`; the contract coerces query strings per field ([gotchas.md](docs/gotchas.md#contract-query-strings-and-the-browse-screen)).
- Layout is keyed on the window width (768px), not the OS; the window frame is the app's own, so drag with `data-tauri-drag-region` ([frontend.md](docs/frontend.md)).
- No native `<select>`, no brand icons from lucide, no colour token that is not in the shared theme ([gotchas.md](docs/gotchas.md#typescript-css-and-the-build)).
- Name a game with its full name via `appLabel`, never `nameShort` ([gotchas.md](docs/gotchas.md#contract-query-strings-and-the-browse-screen)).
- The offline cache is an allow-list of public queries ([gotchas.md](docs/gotchas.md#contract-query-strings-and-the-browse-screen)).

**Crates and CI**

- Anything that does not need a window belongs in `tmc-core`; the CI `core` job must never need `apt-get` ([development.md](docs/development.md#what-ci-runs-and-why-it-is-split-that-way)).
- Keep `cargo check -p tmc-usvfs --target x86_64-pc-windows-gnu` working; it is the only check the injector gets ([sandboxes.md](docs/sandboxes.md#usvfs-the-one-that-deploys-nothing)).
- `audit:rust` is not part of `check`; re-check `src-tauri/.cargo/audit.toml` whenever Tauri is upgraded ([development.md](docs/development.md#supply-chain-npm-run-auditrust)).

## Writing docs here

One subject per file in `docs/`, listed in the table above and in [docs/README.md](docs/README.md). Markdown is never hard-wrapped. Describe current behaviour; a bug story goes in [docs/history.md](docs/history.md) and its rule goes where the code it governs is documented. Other mod managers are named only where the code reads their data or where [docs/prior-art.md](docs/prior-art.md) credits them.
