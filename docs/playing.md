# Playing a game

There are **four** ways a game starts from the app, and which of them is possible is a fact about THIS machine that no server can answer. That is the whole reason the app's launcher beats the website's: a browser has exactly one way to start a game, so the site can offer one button.

| Mode | What it is | Who resolves it |
| --- | --- | --- |
| **native** | A build TMC published and this app unpacked, run as a real process | Entirely local — `games::plan` |
| **sandbox** | The copy installed here, with a profile's mods deployed and its load order applied | Entirely local — `launch::plan` |
| **web** | The app's uploaded JavaScript loader, in a window of its own | `/play/launch` |
| **connect** | The server's own `connectUrl`, handed to the installed game client | The API supplies it; Rust opens it |

`components/play-dialog.tsx` puts the four side by side with the facts that decide between them — which sandboxes exist, whether a TMC build is installed, whether a game folder is set, and the latency measured from this device rather than from a scanner in another hemisphere.

**`native` is offered first when it exists**, because it is the only mode where the app knows the exact build on disk and can therefore say that it will start. It is also the only one that does not consult `directPlay`: that flag answers "does pressing Play with no server start something worth starting", which is a question about a launch the SITE performs. A binary on this disk opening to its own menu is the game's business.

**A server's owner can close a platform**, and the dialog follows: `server.platforms` (`{ web, desktop }` on `ServerInfo`, from the dot-server's `sv_allow_web` /`sv_allow_desktop` query rules) drops the web card when `web` is false and native + connect when `desktop` is false, with a line saying why. Only an explicit off closes anything — the contract defaults both to true, so an older site or a non-dot server keeps every button. The server refuses at the door either way; this is so the app does not offer a launch that will bounce.

**The web player can open full screen**, and the choice is made before the window exists — `fullscreen` on `play_open_web`, set on the window BUILDER. The window is a remote page with no IPC, so it cannot ask for this itself; and calling `set_fullscreen` on a window that is already showing produces a visible resize plus a WebGL context that has already sized itself to the first geometry. Godot's canvas reads its size once at boot, so the game would render small inside a full-screen window with black bars it has no idea about.

### The web player runs on the SITE's origin, and that is the isolation

A game loader is third-party JavaScript. Running it in the app's own webview would hand it `invoke`, and the window it would reach is the one holding the sandbox engine, the RCON store and the settings that anchor the plugin jail.

So the player window opens at `<site>/app-player` — a chrome-free page in website-city with its own root layout, no providers and no session read. Two properties follow, and both are enforced by something other than care:

- **The app's CSP forbids the loader here anyway.** `tauri.conf.json` sets `script-src 'self'`, so a loader on a CDN does not execute in the app's webview at all. Widening that would widen it for the whole UI.
- **A remote page has no IPC.** Tauri exposes commands to the app's own asset origin only; a window pointed at `https://` has no `invoke`, no event channel and no plugin access — by the runtime's construction rather than by a permission list somebody has to keep correct. A hostile loader is a web page with a web page's powers.

The boot descriptor reaches it through `initialization_script`, not a query string, for the reason the site's own player gives: a server address on a URL ends up in every cache entry and history record that URL touches.

That window is **decorated**, unlike the main one. Drawing our own frame needs `core:window:allow-start-dragging`, and this window is remote precisely so that it has no capabilities at all. A native frame is the price of the isolation.

**`/app-player` is excluded from website-city's locale middleware**, alongside `/file/…` and for the same reason: it lives outside `[locale]` deliberately, and rewritten under one it 404s.

### The webview names ids, never URLs

`play_open_web` takes an app id and an optional server id and asks `/play/launch` itself. A loader URL from the frontend would be a `<script src>` injected into a page on the site's origin; a connect link from the frontend would be a request to hand a string to whatever program claimed a scheme. Both are resolved in Rust, and a connect link is checked against `plugins::apps::LAUNCH_SCHEMES` — the same closed list a plugin's launch rule is held to.

This is the rule `download_release` already followed, restated: there is no `play_open(loaderUrl)`, and nothing an injected script could point at a script of its choosing.

## Games TMC publishes

Everything above manages somebody else's game: `detect` finds where Steam put it, `deploy` puts mods into it, `launch` starts the copy that is already there. `tmc-core/games/` is the one place the app is the **distributor** — it downloads a build, unpacks it under the app's own data directory, keeps it current and starts it as a real process.

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

**Nothing here is new machinery, and that is the design.** An install is a download, an extraction, a row and a launch, and the app already had one of each with the bounds and failure modes worked out. A second downloader would have been a second place for a bandwidth limit to be ignored; a second unpacker would have been a second zip-slip guard, and the one that was wrong would be the one nobody was reading. What the module contributes is the sequencing and the refusals.

### Where the builds come from

`AppNativeBuild` on the website — one row per (app, platform), carrying an `https` URL, a **required** SHA-256, a size, an archive format, the entry path inside it and the launch arguments. `scripts/publish-native-build.ts` writes one and computes the checksum from the bytes it uploads, so the row and the artifact cannot disagree.

**The catalogue says a build EXISTS; `/apps/:id/build` says where it is**, one platform at a time. A device browsing the whole app list must not come away holding download addresses for every game on every machine it is not — the same rule `download_release` follows, applied to a listing.

**Architecture is part of the platform value.** `AppPlatform` — the descriptive "where is this game available" enum the website draws on an app page — says `PLATFORM_WINDOWS`, and that is a fine answer to that question. It is not an answer to this one: an x64 archive handed to an ARM machine either refuses to start or runs emulated at a cost nobody chose. `games::platform::current()` is compile-time, from this binary's own target triple, because the binary that is asking IS the evidence: a build running under Rosetta should ask for the architecture whose emulation is already working on this machine.

**Mobile is refused, and that is a statement about the OS.** Android and iOS both decline to execute code an app wrote into its own container; an `.apk` is installed by the package installer and an `.ipa` by the store. The two targets are in the enum because the site may publish them and the app should be able to say "there is a build, get it from the store" — which is true and useful, and is not an install.

### A game runs on its engine's build

**TMC publishes one native build, under the engine app, and every game on that engine runs in it.** A TMC game is a signed pack the shell downloads (the same way a browser tab gets it), and a server can change which game it runs while its players stay connected, so `dot-server-deploy`'s `./server export-native` publishes the shell once, under the engine, rather than a copy per game. On the site that means `/apps/<game>/build` answers `null` for every game and `/apps/<engine>/build` answers the shell; the catalogue's `install` is null for the games and lists the engine's targets.

So the native mode looks for the game's own install first and then, **for a TMC pack game only**, its engine's (`app.engine.id` in the catalogue). A third-party game made with the same engine names the same engine, and the TMC shell would start it at the shell's own menu or connect to a server it cannot play, so the fallback needs proof the app is a TMC pack game. The site is the one that knows: `/play/launch` names a signed pack in `boot.solo` only for an app that has one, and the catalogue row carries no such flag. An official app (`isOfficial` in the catalogue) is a TMC game without asking, so a game with `directPlay` off (no `boot.solo`) can still join a server through the shell. Otherwise Rust asks in `native_target` (games.rs), anonymously, because a signed-in `/play/launch` counts a play and may mint a handoff, and the Play dialog asks before anybody has pressed anything. Both the dialog (`game_native_target`, for the native card and the "Install it" hint) and `game_launch` go through that one function, so the dialog never offers a launch Rust then refuses. A catalogue that cannot be reached is its own error ("Couldn't reach the catalogue..."), not "That game is not installed", which would send somebody who has the build to install it again.

The game's own id is what the launch is for: `boot.solo` is asked for the game, so Play on a game starts the shell straight into that game's pack, and a server join hands it that server. The session is recorded under the install's app id, because that is the id the update, uninstall and "already running" refusals ask by, and it carries the game's slug and name, so play time and history read as the game. One shell runs at a time: a second would share the download cache with the first.

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

The last two are the ones worth keeping. An archive that unpacked perfectly and does not contain its own entry is an install that **reports success and then cannot be started**, with nothing on screen to explain it — `games::entry_path` resolves the declared entry through `join_relative` and checks that it is a file, because neither the extraction nor the checksum can notice a publisher naming the wrong path. And writing a new build over files a live process has mapped fails outright on Windows and succeeds on Unix, leaving a process running code that is no longer on disk.

**The previous version is not removed until the new one is in place.** A build lands in `<data>/games/<slug>/<version>/` and the row is only repointed once the entry is verified, so a failed update leaves a working game. Pruning the old version directories happens LAST, after the row already points somewhere else.

**Uninstalling removes the files first and the row last**, so a failure halfway leaves a row naming a partly-removed install — which the app can see and offer to finish — rather than a directory nothing knows about. The delete refuses any path that is not genuinely under the app's own games root: `dir` came out of a SQLite file on the user's disk, and a hand-edited row naming `/` must not be able to turn "uninstall a game" into a recursive delete.

### Why a game is not a subscription

`subscription` is a MIRROR the full sync rewrites: anything the server did not send is deleted, because that is how an unsubscribe on another device reaches this one. A game on this disk is a fact about this disk — the account knows which games exist and cannot know which of them somebody unpacked onto a laptop — so `native_game` is its own table for the same reason `local_mod` is. A sync must not be able to forget an install, and signing out must not either.

### Keeping them current

**Play updates a stale build first.** `game_native_target` reports `update` when the site publishes a newer build than the installed one, and the dialog runs `game_install` before `game_launch` (the card says so): a shell older than the site and its servers cannot read what they publish now, and the sync-pass auto-update below needs `auto_update` on and a signed-in sync.

**Play updates a stale build first.** `game_native_target` reports `update` when the site publishes a newer build than the installed one, and the dialog runs `game_install` before `game_launch` (the card says so): a shell older than the site and its servers cannot read what they publish now, and the sync-pass auto-update below is off by default and needs a signed-in sync.

`games::updates::run` on the library's sync pass, which is the device's own "am I online now?" heartbeat — the same place outstanding play time is drained. Four rules, all of them written down rather than implied: the install must have `auto_update` on, the game must not be running, only an install that already exists is updated, and **a version that cannot be ordered is not an update**.

`games_check_updates` reports and installs nothing, even for a game with auto-update on. It is the button somebody presses, and a button that silently downloaded four gigabytes because a row had a flag set is a button nobody presses twice.

### The launch arguments are a vector, never a command line

`args` is one element per argument and stays that way. A command string would have to be split by somebody, and there is no shell in this app to split it — so a server name containing a space would become two arguments and one containing a quote would become something nobody can predict. Substitution happens INSIDE an element and can never create a new one, and a value carrying a control character drops the argument rather than being escaped.

**An argument whose value is missing is dropped, with the flag before it.** `["--connect", "{host}:{port}"]` with no server must not become `["--connect", ":"]` — the game would take that as an address, fail to reach it, and report a connection error for a connection nobody asked for. Dropping the pair starts the game at its own menu, which is what "launch with no server" means.

**A Godot export reads its game's arguments only after a bare `--`.** Its `OS.get_cmdline_user_args()` is everything after the separator, and the engine silently ignores an argument it does not recognise — so `tmc.x86_64 --connect host:port` starts, draws its menu and joins nothing. The template `dot-server-deploy`'s `./server export-native` publishes is `["--", "--connect", "{host}:{port}", "--udp", "{host}:{udpPort}"]`, and `build::is_separator` is why the separator is never popped as if it were the flag a missing value belonged to. `{udpPort}` is a dual-stack server's ENet port, which `game_launch` reads as `udpPort` off the site's server summary (spy takes it from the server's A2S `udp:<port>` keyword); a server with none loses the `--udp` pair, and the shell still tries ENet at the `--connect` host and port for its `udp_connect_timeout_sec` (2.5 s) before joining over WebSocket there. The `gameUseUdp` setting (Settings → Games → Connect over UDP), off, appends `--transport ws` after the separator (`build::transport_args`), and the shell then joins over WebSocket only: browser and desktop players share a match either way, because every server listens on both. `games::tests::real_godot_export_receives_the_server_through_the_plan` runs a real export through `plan` (ignored by default; set `TMC_GODOT_BUILD_DIR` to an unpacked `tmc-linux-x64.zip`) and checks the address reached the game.

**Solo play** (no server): `game_launch` with no `serverId` asks `POST /play/launch` (`mode: web`) for the game's `boot.solo`, best effort, and appends `--solo <owner>/<repo>` and `--solo-version <v>` after the separator (`build::solo_args`, again only for a template with the bare `--`). A `?solo=` from a `tmc://play/app/` link is the fallback when the site gives none; a launch into a server never sends either. The shell reads these in `tmc_solo.gd`. A pack that moved to a new id is the site's business first (`boot.solo` names the new id and its version) and the shell's second: a link naming the old id with no version reaches `latest.json`, whose `moved_to` the shell follows one hop. A solo launch also gets `--content-url <boot.game.origin>/content` (`build::content_url_from_boot`, `https` and a bare origin only): a joined shell is told where packs live by its server and the web player uses its page's own `/content`, so a native shell with neither failed at its first `latest.json` with a relative URL and said the game had no published version. The other shell flags the app emits are `--connect`, `--udp`, `--transport`, `--browser-backbone-url` and the cache pair; `--verbose` is the shell's and is not set by the app. UDP and WebSocket players share a server because dot-server listens on both at once.


**A release Godot export block-buffers stdout into a pipe**, so a game stopped by a signal — including `Sessions::stop`'s SIGKILL — loses whatever it printed since the last 4 KB. That is the game's setting to change (`application/run/flush_stdout_on_print`), not something the launcher can fix from outside; the test above wraps the plan in `stdbuf -oL` for the same reason.

## Play sessions

The game outlives the launcher, but the launcher must not drop the `Child`: the handle is every fact about the launch — the play time the account is told, whether a game is already open, and what a game that died in two seconds printed.

`session::Sessions` keeps it on a supervisor thread. What that buys, and what it deliberately does not:

**`SessionKind` is the honest record of what can be known.**

| Kind | Duration | Exit code | Output |
| --- | --- | --- | --- |
| `Process` | measured | real | captured |
| `Handoff` | **none** | none | none |
| `Web` | measured | none | none |

A `steam://` launch is a `Handoff`: the OS opener started a launcher which started the game, and there is no process of ours. It reports **no** play time rather than the time between firing a URI and the app noticing — that number describes the app, and a launcher whose statistics are fiction is worse than one with none. The Library says "not measured" for one, not "0h": `Session::seconds` returns `None` for it and every caller decides what to draw — `0` is the wrong answer everywhere except the database column.

**A duration is never inferred from a later launch.** Other managers treat "the app was closed" as the end of a session, or clamp a long one to something plausible. Both produce a number nobody can distinguish from a right one.

**Output goes to one bounded file per session.** That is the whole crash-report story: a game that exits in two seconds has almost always printed why, and without a pipe that goes to a console nobody attached. Bounded because a game left running overnight with a chatty logger is the normal case.

**`stop` signals by pid.** The supervisor holds the `&mut Child` for its whole `wait`, so `Child::kill` is unreachable. Signalling by pid is normally a bug — a pid is reused — and is sound in exactly that position and nowhere else: a process inside `wait` has not been reaped, so its pid cannot yet have been handed to anybody else.

**A finished session is WRITTEN, not reported.** It lands in `game_session` marked unreported, and `sessions_flush` sends it when the device next has an API to talk to. A game is very often played offline — a laptop on a train is the case the feature is for — and play time that only counts when the network happened to be up is play time that silently goes missing. The flush runs on the library's sync pass, which is the device's own "am I online now?" heartbeat.

**A sandbox that is already running cannot be started twice.** The second copy opens files the first has mapped, and the play clock for that sandbox would otherwise be able to exceed wall-clock time.
