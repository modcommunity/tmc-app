# Docs

The index. `CLAUDE.md` in the repository root is the always-loaded summary; this tree is the detail, one subject per file, meant to be opened when you touch that subject rather than read end to end.

## Working in this repository

| File | Read it when |
| --- | --- |
| [development.md](development.md) | running, checking or building anything; pointing a build at the dev site; building on Linux without root; what CI runs; `audit:rust` |
| [BUILDING.md](BUILDING.md) | producing an artifact for a platform, including the whole Windows build from Linux |
| [avatar.md](avatar.md) | the avatar studio: the shared editor, its IPC, the model-bytes command, the `blob:` CSP allowance |
| [RELEASING.md](RELEASING.md) | cutting a release: the tag, the secrets, the bucket, the updater keypair |
| [extending.md](extending.md) | adding a content kind, a query protocol, a game, a mod manager, a deployment strategy or a step type |
| [gotchas.md](gotchas.md) | before declaring anything done: the traps that do not error usefully, by area |
| [history.md](history.md) | a bug that "cannot be happening" — what this repository has already met, and how it looked |
| [gaps.md](gaps.md) | planning: what is deliberately not built yet, and why |

## Architecture

| File | Subject |
| --- | --- |
| [architecture.md](architecture.md) | the one rule (credentials and the filesystem live in Rust), the three crates and why they are split, type safety, Rust conventions |
| [source-map.md](source-map.md) | what each file owns, in `tmc-core`, `tmc-usvfs`, the Tauri shell and `src/` |
| [api.md](api.md) | the device grant with PKCE, the `/api/app/v1` contract mirror, `/apps`, `/play/launch`, the browse filters |
| [settings-and-logging.md](settings-and-logging.md) | app settings vs account settings, the two jail anchors that are not settings, the audit log |
| [frontend.md](frontend.md) | window-keyed layout, the app's own window frame, artwork components, what the frontend tests cover |

## Subsystems

| File | Subject |
| --- | --- |
| [live-queries.md](live-queries.md) | querying game servers from the device: which port, which protocols, parser rules, the registry, latency display and history |
| [plugins.md](plugins.md) | plugins as data: the four types, the step vocabulary, the defences, approval, signatures, app rules per game |
| [sandboxes.md](sandboxes.md) | the library database and sync, sandboxes, stage then deploy, the four strategies (USVFS included), the ledger, conflicts, `sandbox.json`, share codes |
| [local-mods.md](local-mods.md) | importing dropped files, adopting what is in a game folder, reading another mod manager, `tmc.json` |
| [config-editor.md](config-editor.md) | editing a game's own settings files: the two bounds, `config.json`, why it is a text box |
| [playing.md](playing.md) | the four launch modes, the web player's isolation, games TMC publishes, launch argument vectors, play sessions |
| [updater.md](updater.md) | how the app updates itself, and why the signature is the whole security model |
| [downloads.md](downloads.md) | the download queue: resume, checksums, its own HTTP client, bandwidth |
| [detection.md](detection.md) | finding installed games from launcher manifests, and the bounded folder walk |
| [rcon.md](rcon.md) | the two RCON dialects, the one deliberate SSRF gap, how passwords are kept |
| [deep-links.md](deep-links.md) | `gs://`: a link may SHOW, never DO |
| [prior-art.md](prior-art.md) | which mod managers this learned from, and how they are credited |
