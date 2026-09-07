# TMC App

The official [The Modding Community](https://moddingcommunity.com) app, built
with [Tauri 2](https://tauri.app/), Rust and React.

One codebase, five targets: **Windows, macOS, Linux, Android and iOS**.

## What it does

- **Install mods for a lot of games** — one click, into the right folder for
  that game, with an uninstall that removes exactly what it added and nothing
  else. The [supported games](#supported-games) are at the bottom
- **Sandboxes** — named mod profiles per game, with their own load order and
  their own deployment method. Switching between them does not re-download
  anything, and your game folder is recoverable either way
- Browse and view assets, mods, servers, server maps, articles, communities,
  collections and members
- **No account password, anywhere.** Signing in happens on the real website in
  your own browser; the app is handed a token and never a password. There is
  nothing to type into this app and nothing for it to store
- Real latency pings measured from your device, not from a scanner in another
  hemisphere
- Live server queries over each game's own protocol
- A download queue that pauses, resumes and obeys a bandwidth limit
- RCON for servers you run — passwords encrypted on the device, never uploaded
- Settings split between this device and your account

## Installing

**Linux** — one line, no root, no package manager:

```bash
curl -fsSL https://raw.githubusercontent.com/modcommunity/tmc-app/main/scripts/install.sh | sh
```

It verifies the download against the release's checksums, installs into
`~/.local`, and registers the launcher entry and the `tmc://` scheme. Undo it
with `… | sh -s -- --uninstall`.

**Windows** — the `-setup.exe` from the [releases
page](https://github.com/modcommunity/tmc-app/releases), or the `.msi` if you
deploy by policy.

**macOS** — the `.dmg`.

## Getting started

```bash
npm install
npm run desktop      # or: android / ios
```

Linux desktop builds need `webkit2gtk-4.1`, `libsoup-3.0` and `pkg-config` —
or `npm run sysroot`, which builds a private one without needing root.

## Building something to ship

```bash
npm run build:linux      # AppImage + deb
npm run build:windows    # portable exe, .exe setup and .msi — FROM LINUX
```

The Windows build cross-compiles: `cargo-xwin` for the compile, `makensis` for
the setup, `wixl` for the MSI. macOS is the one target that genuinely needs its
own machine, and comes from CI. [`docs/BUILDING.md`](docs/BUILDING.md) has the
prerequisites, the caveats (unsigned binaries, WebView2 in the MSI) and the
release process.

## Contributing plugins

A plugin is a folder with a `plugin.json` in it. See `examples/plugins/` for a
working installer, server-query and theme plugin, and `CLAUDE.md` for the format
and the sandbox rules.

Per-game rules — where a game's mods go, how it launches, how its sandboxes
behave — live in `plugins/app/<slug>/` and are compiled into the binary.
Adding a game is four small JSON files and no Rust;
[`scripts/gen-app-rules.py`](scripts/gen-app-rules.py) writes the first draft.

Plugins are **data, not code** — they describe steps the app carries out inside
a jail the user grants, and every action is written to the activity log.

## Supported games

Mod install management, per game: where its mods go, how it launches, which
deployment methods suit it, and how to find it on your machine.

**Tested** means somebody has actually installed a mod for that game with this
app and watched it land in the right place. Every row is currently **No** —
the rules are written from each game's own modding documentation and from
Vortex's and r2modman's published game data, which is not the same thing as
having been run once. They will flip to Yes one at a time.

| Game | Mods land in | Tested |
| --- | --- | :-: |
| 7 Days to Die | `Mods/` | **No** |
| Baldur's Gate 3 | `Mods/` | **No** |
| Cyberpunk 2077 | `archive/pc/mod/` | **No** |
| Fallout 3 | `Data/` | **No** |
| Fallout 4 | `Data/` | **No** |
| Fallout: New Vegas | `Data/` | **No** |
| Garry's Mod | `garrysmod/addons/` | **No** |
| Grand Theft Auto III | `modloader/`, `CLEO/` | **No** |
| Grand Theft Auto IV | game root, `scripts/` | **No** |
| Grand Theft Auto V | game root, `scripts/` | **No** |
| Grand Theft Auto: San Andreas | `modloader/`, `CLEO/` | **No** |
| Grand Theft Auto: Vice City | `modloader/`, `CLEO/` | **No** |
| Kerbal Space Program | `GameData/` | **No** |
| Minecraft | `mods/`, `resourcepacks/`, `shaderpacks/` | **No** |
| Mount & Blade II: Bannerlord | `Modules/` | **No** |
| Palworld | `Pal/Binaries/Win64/Mods/` | **No** |
| RimWorld | `Mods/` | **No** |
| Rust | `oxide/plugins/` | **No** |
| Sons of the Forest | `BepInEx/plugins/` | **No** |
| Stardew Valley | `Mods/` | **No** |
| Starfield | `Data/` | **No** |
| Team Fortress 2 | `tf/custom/` | **No** |
| The Elder Scrolls III: Morrowind | `Data Files/` | **No** |
| The Elder Scrolls IV: Oblivion | `Data/` | **No** |
| The Elder Scrolls V: Skyrim | `Data/` | **No** |
| The Elder Scrolls V: Skyrim Special Edition | `Data/` | **No** |
| The Elder Scrolls V: Skyrim VR | `Data/` | **No** |
| The Sims 3 | `Mods/Packages/` | **No** |
| The Sims 4 | `Mods/` | **No** |
| The Witcher 3: Wild Hunt | `Mods/` | **No** |
| V Rising | `BepInEx/plugins/` | **No** |
| Valheim | `BepInEx/plugins/` | **No** |

Five of those need a word of warning before you point a sandbox at a folder:

- **The Sims 3, The Sims 4 and Baldur's Gate 3** read their mods out of your
  documents folder, not out of the install. Point those sandboxes at
  `Documents/Electronic Arts/The Sims 4` (or the Larian folder for BG3), not at
  the folder Steam knows about — the install folder holds no mods at all.
- **Rust is server-side only.** EasyAntiCheat runs in the kernel, there is no
  client-side Rust modding, and this app will not invent one. Point a Rust
  sandbox at a dedicated server folder.
- **Minecraft is the instance model**, the way CurseForge and Prism do it. Give
  a sandbox its own empty folder and it gets its own `mods`, `config`, `saves`
  and `options.txt`, with your vanilla `.minecraft` left alone.

A few games refuse to load mods at all while their anti-cheat is on — Valheim
and 7 Days to Die both want it switched off in their launcher first. The app
cannot do that for you and says so rather than deploying into a folder the game
will ignore.

Your game is not here? It is four JSON files under `plugins/app/<slug>/` and no
code — see **Adding a game** in `CLAUDE.md`.

## Security

Found something? **Please do not open a public issue** —
[`SECURITY.md`](SECURITY.md) has the private reporting route, the threat model
the app is built around, and the list of things that look like holes and are
deliberate (RCON reaching private addresses is the usual one).

## Licence

[GPL-3.0-only](LICENSE). The game names, trademarks and mod-manager names used
throughout are their respective owners'; they appear here to say which game a
rule is for and which manager a folder layout belongs to, and nothing in this
repository is a derivative work of any of them — see **Prior art** in
`CLAUDE.md`.
