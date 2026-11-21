# Settings and logging

## Settings: the two halves

| | App settings | Account settings |
| --- | --- | --- |
| Stored | `settings.json`, this machine | `UserSettings` on the website |
| Written by | `settings.rs` | `PATCH /api/app/v1/me` |
| Read by | `useSettings().app` | `useSettings().user` |
| Contains | Theme, scale, game directories, logging, latency, download limits, plugin prompts | Notifications, locale, timezone |
| Can fail | No | Yes — network, auth |

The test for which side something belongs on: **would this be wrong to apply on a different machine?** A game directory would be. A notification preference would not.

### Two of them are not settings at all

`gameDirs` and `downloadDir` anchor the plugin jail — `plugins::jail` resolves every `PathRef` beneath them, so an installer holding `{gameDir, "", write}` can write anywhere below. They are therefore the only fields with their own commands:

- **`settings_patch` refuses them** (`JAIL_ROOT_FIELDS`), loudly rather than by dropping the key. The refusal is in `SettingsStore::patch`, so it holds for every caller and not just the one command.
- **`settings_set_game_dir` /`settings_set_download_dir`** run `anchor::validate_root`, which rejects a drive root, a system directory, and anything that *contains* the app's data, logs, cache, plugins or the user's home. A jail anchored above the app's own files would enclose `settings.json` and the plugin registry — a plugin that can rewrite the registry can grant itself permissions.
- The **canonical** path is what gets stored and what gets audited, so the value in `settings.json` is the one the jail will resolve to later.
- **A sandbox's own `gameDir` goes through the same validator**, on `sandbox_patch`. It gets no more trust for having arrived on a different command.
- Both are audited at **Security** level, so turning logging off cannot hide a change to where plugins may write.

What this does *not* establish is that a human chose the path. That came from the OS dialog and is gone with it; the anchor's **identity** is what is checked now, not its provenance. `anchor.rs`'s header says so plainly — don't let a later comment upgrade the claim.

## Logging

`Settings → Logging` reads an append-only JSONL audit trail. It is a **record of what the app did on the user's behalf**, not diagnostics — `tracing` handles those and goes to stderr.

- `Security` level entries are written **even when the user turns logging off**. A switch that lets a plugin ask the user to stop watching is not a control.
- Clearing the log writes the "log cleared" entry **after** the truncation.
- Every plugin step, every download URL, every jail refusal, every permission grant lands here.

Write with the `audit!` macro. Never put a token or a credential in `data`.

## Where files live

On desktop the data directory (`settings.json`, `library.sqlite3`, plugins, staging, backups) is `gamespice-app` under the platform's app-data root: `~/.local/share/gamespice-app` on Linux, `%APPDATA%\gamespice-app` on Windows, `~/Library/Application Support/gamespice-app` on macOS. The audit log and cache follow the same name. Tauri would name them after the bundle identifier, which stays `com.moddingcommunity.app` so updates install over the old app, so `AppPaths::resolve` renames them (`gamespice_dirs` in `src-tauri/src/paths.rs`, rules in `tmc_core::rename`):

- **An install from before the rename is moved once, and the old path becomes a link to the new one.** The library stores absolute paths into the data directory and symlink-strategy sandboxes link into staging, so the old path has to keep resolving.
- **Where the link cannot be made** (Windows without the right to create symlinks), the move is undone and the app keeps using `com.moddingcommunity.app`, with a warning in the log. A working install under the old name beats a renamed one with dangling paths.
- **Two directories are never merged.** If both exist, the new one is used and the old one is left alone.
- **The audit log** (`activity.jsonl`) moves with the data, or on its own where the log directory is elsewhere (macOS, Windows). The cache is scratch and is not moved.
- **The stored sign-in** moves from the `com.moddingcommunity.app` keychain service to `gamespice-app` the first time it is read; the old entry is deleted only after the new one was written (`secure::keyring_get`).

The WebView's own storage stays where Tauri puts it, keyed by the identifier, so nothing in it is lost. Mobile is unaffected: there everything is inside the app's container.

## Advanced: tray and notification cards

Both settings are machine-level, in `settings.json`, and OFF by default.

| Setting | Default | Effect |
|---|---|---|
| `minimiseToTray` | `false` | Closing the window hides it; a tray icon (Open / Quit) keeps the app running. No tray available → the close quits. |
| `trayNotifications` | `false` | Polls `/notifications` while signed in and shows each new item as a corner card. |
| `notificationPollSecs` | `60` | Poll interval, clamped to 30–3600. |

Code: `src-tauri/src/notifier.rs` (tray, poller, card windows), `src-tauri/core/src/notify.rs` (parsing, per-kind style, first-poll baseline, safe click URLs).
