# Security

## Reporting a vulnerability

**Do not open a public issue.** Report privately through GitHub's
[Report a vulnerability](https://github.com/modcommunity/tmc-app/security/advisories/new)
form, or by email to `security@moddingcommunity.com`.

Please include what you did, what happened, and which build you were on
(Settings → App → About names the version and the site it talks to). A proof of
concept helps enormously, and a plugin manifest or a captured server reply is
usually the whole report.

You will get an acknowledgement within a few days. Nothing here is bug-bountied
— this is a small project — but every accepted report is credited in the release
notes unless you ask otherwise.

## What this app assumes an attacker can already do

The architecture is built around one assumption, and it is worth stating so
that reports can be aimed at it: **a script is already executing in the
webview.** Mod descriptions, item titles, server names and review bodies are
untrusted text from strangers, rendered in a window that can call `invoke`.

The defence is that there is nothing worth reaching on that side of the bridge:

- **No command returns a credential.** There is no `get_token`. The access
  token is memory-only in Rust, the refresh token is in the OS credential store,
  and the `Authorization` header is attached inside `tmc_core::api`.
- **No command takes a filesystem path from the webview**, with two documented
  exceptions that each state their own cost: `fs_list_dirs` (directory _names_
  only, never files, never a read or a write) and the config editor (bounded by
  the game's own declared locations _and_ the path jail, and removable with
  `allowConfigEditing`).
- **The webview names ids, never URLs.** `download_release`, `play_open_web`
  and the connect-link path all resolve in Rust, so there is no command an
  injected script could point at a file or a host of its choosing.
- **The API base is not a setting.** A "which server?" field is a phishing
  primitive; a release build has no runtime override at all.

So the reports that matter most are the ones that break one of those
sentences.

## Things that are deliberate, and are not vulnerabilities

Reported often enough to be worth listing:

- **RCON connects to private addresses.** `192.168.1.10` and `127.0.0.1` are
  the normal answers for "administer my server", so RCON is the one place
  `net::addr::resolve_public` is deliberately not used. It is bounded instead by
  a per-host connection cooldown, a session cap, and a Security-level audit
  entry on every connect. This is documented as a widening in
  `core/src/rcon/mod.rs`.
- **The app process has the user's filesystem permissions.** It writes into
  game folders — that is the product. The jail bounds what a _plugin_ and the
  _webview_ can name, not what the process could do if it were rewritten.
- **A signed plugin is confined exactly as an unsigned one is.** A signature
  says a holder of that key produced those permissions and those steps. It says
  nothing about whether they are safe.
- **`tmc://` links do nothing but show a page.** If you find one that performs
  an action, that _is_ a vulnerability — the closed list is in
  `core/src/deeplink.rs` and its test names the shapes that must never start
  working.
- **Unsigned cross-built binaries.** Windows and Linux artifacts are built on a
  Linux host and are not code-signed, so SmartScreen will warn. That is a fact
  about the build, not a compromise; verify against `checksums.txt` on the
  release, which is what `scripts/install.sh` does before it will install
  anything.

## Where the security-relevant code lives

| Area                                         | File                                      |
| -------------------------------------------- | ----------------------------------------- |
| The path jail                                | `src-tauri/core/src/plugins/jail.rs`      |
| What may anchor it                           | `src-tauri/core/src/anchor.rs`            |
| The public-address guard (SSRF)              | `src-tauri/core/src/net/addr.rs`          |
| Bounds-checked parsing for every wire format | `src-tauri/core/src/net/reader.rs`        |
| Token lifecycle                              | `src-tauri/core/src/auth.rs`, `secure.rs` |
| Encryption at rest                           | `src-tauri/core/src/crypto.rs`            |
| Plugin signatures and the trust store        | `src-tauri/core/src/plugins/signature.rs` |
| Extraction limits, zip-slip, tar links       | `src-tauri/core/src/plugins/steps.rs`     |
| What a `tmc://` link may mean                | `src-tauri/core/src/deeplink.rs`          |
| The IPC surface, and its two rules           | `src-tauri/src/commands/mod.rs`           |
| The audit trail                              | `src-tauri/core/src/logging.rs`           |

Parsers under `net/query/` read bytes from unauthenticated machines and the
release profile sets `panic = "abort"`, so **a panic in a parser is a remote
kill switch**. Each one has a test that feeds every prefix of a valid reply
through it; a parser without that test is not finished.

## Supported versions

Pre-1.0: only the latest release gets fixes.

## Dependencies

`npm run audit:rust` runs `cargo audit --deny warnings` against a reviewed
allow-list in `src-tauri/.cargo/audit.toml`. Ignores are per advisory id rather
than per crate, so a new advisory against an already-listed crate still fails
the run. The list is re-checked whenever Tauri is upgraded.
