# Architecture

The one rule everything follows, the three crates, and the conventions on each side of the IPC bridge. The file-by-file table is [source-map.md](source-map.md).

> **Credentials and the filesystem live in Rust. The webview gets data.**

There is no `fetch` in the frontend and no `get_token` command. A mod description is untrusted text rendered inside a webview that can call `invoke`; the defence is that there is nothing worth stealing on that side of the bridge. Every layer below assumes an attacker has already achieved script execution in the webview and asks what they can reach.

**One command takes a path from the webview**: `fs_list_dirs`, which backs the app's own folder picker and returns directory *names* only — no files, no reads, no writes. Its module header states exactly what that widens. Nothing else in `commands/` accepts a path.

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
  ├── deeplink.rs what a `gs://` link may mean, which is "show a page"
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

**`tmc-usvfs` is a third crate rather than a module** for one concrete reason: it is the only part of the tree that can be `cargo check`ed for Windows from a Linux box. `tmc-core` cannot without a C cross-compiler for bundled SQLite (the `cargo-xwin` toolchain in [BUILDING.md](BUILDING.md), which CI does not install) — so code that reaches into another process would otherwise be verified by reading it. The split is what makes `cargo check -p tmc-usvfs --target x86_64-pc-windows-gnu` possible — and it has found real bugs that reading had missed (see [history.md](history.md)).

**The workspace split is load-bearing.** `tmc-core` holds the plugin jail, the protocol parsers and the token lifecycle — the three things you most want to compile and fuzz on a CI runner with no display stack, no WebKit and no dbus. `cargo test -p tmc-core` needs none of them. Anything you add that does not genuinely need a window belongs on that side of the line.

## Type safety

- **`any` is banned** and enforced by eslint. `unknown` plus a parse is always available.
- **Every IPC call names a schema.** `invoke` returns whatever the caller claims; `src/lib/ipc/call()` makes that claim checked. Importing `invoke` anywhere else is an eslint error.
- **Every API response is parsed.** Same reason.
- `noUncheckedIndexedAccess` is on. A lot of this code indexes into plugin-supplied data.
- Floating promises are an error — a swallowed IPC rejection is an install that silently reported nothing.

## Rust

- `AppError` is the only error type crossing IPC. It serialises as `{ code, message }`; `code` is stable, `message` is for humans.
- `AppError::Internal`'s `Display` is a fixed sentence. Detail goes to `detail()` and the log, never to the webview.
- Prefer returning an `AppError` over panicking. A panic in a command is a `Result<_, String>` the frontend cannot classify.
- New privileged capability → new `#[tauri::command]` under `commands/`, with the policy check *in* the command, not in the caller.
