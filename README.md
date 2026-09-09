The **official** [TMC App](https://moddingcommunity.com/tmc-app), built with [Tauri 2](https://tauri.app/), [Rust](https://rust-lang.org/) and [React](https://react.dev/).

## What It Does
### Mod Management
- **One-click Install**: Simply click to install or uninstall mods that support installing via the app. The list of [supported games](#supported-games) are at the bottom of this page.
- **Sandboxes**: Named mod profiles per game, with their own load order and their own deployment method.
  - Switching between them does not re-download anything, and your game folder is recoverable either way!
### Server Management
- **Server Browser**: A full server browser like the one on the website, but with **real-time latency** and **server status** along with latency charts & graphs. You can also filter by game version, mod list, and more.
- **RCON**: Remote console access to your servers, with a full command history and logging. You can also send commands to multiple servers at once, and even schedule commands to be sent at a later time.
- **Connect**: One-click connect to servers that support it, with the ability to save your login credentials for future use.
### Gaming Platform
- **Integrated Game Launcher**: Launch external games from the app or launch integrated games right within the built-in Game Player.
- **Find fun games or servers**: Our gaming platform relies on third-party communities creating games and servers. Browse these games and servers right through our app and connect to them with one click!
- **Assets**: Automatically download and install game assets onto servers where the game supports it (integrated games only).

## Device Support
| Platform | Status |
| --- | --- |
| Linux | ✅ |
| Windows | ✅ |
| macOS | ✅ |
| Android | ✅ |
| iOS | ✅ |

## From Maintainer & WARNING
This application was built initially with **Claude Code** and will continue to be maintained and extended using it. This is because I (`gamemann`) cannot build the entire TMC platform alone (I wish I could lol).

**Please treat this as partially tested.** I've tested many things in the app, but there are still many things that need to be tested and verified. If you find any bugs or issues, please report them in [issues](https://github.com/modcommunity/tmc-app/issues).

I intend on reviewing code, testing, and editing documentation regularly. If you're interested in helping out, please let me know!

## Installing
### Linux
One line, no root, no package manager, just `curl` and `sh`:

```bash
curl -fsSL https://raw.githubusercontent.com/modcommunity/tmc-app/main/scripts/install.sh | sh
```

The script verifies the download against the release's checksums, installs into `~/.local`, and registers the launcher entry and the `tmc://` scheme. Undo it with `… | sh -s -- --uninstall`.

### Windows
You can install the app onto your computer using the `setup` executables (`setup.exe` or `setup.msi`) in the releases.

The `portable` executable is also available for those who want to run the app without installing it. The portable version **DOES NOT** register the launcher entry or the `tmc://` scheme.

### macOS
Install using the included `.dmg` file.

## Development
### Getting Started
```bash
npm install
npm run desktop      # or: android / ios
```

Linux desktop builds need `webkit2gtk-4.1`, `libsoup-3.0` and `pkg-config`, or `npm run sysroot`, which builds a private one without needing root.

### Targeting Platforms
```bash
npm run build:linux      # AppImage + deb
npm run build:windows    # portable exe, .exe setup and .msi — FROM LINUX
```

The Windows build cross-compiles: `cargo-xwin` for the compile, `makensis` for
the setup, `wixl` for the MSI. macOS is the one target that genuinely needs its
own machine, and comes from CI. [`docs/BUILDING.md`](docs/BUILDING.md) has the
prerequisites, the caveats (unsigned binaries, WebView2 in the MSI) and the
release process.

## Contributing Plugins
A plugin is a folder with a `plugin.json` in it. See `examples/plugins/` for a
working installer, server-query and theme plugin, and `CLAUDE.md` for the format
and the sandbox rules. We will be improving this system over time.

**Per-game rules**:
  - Where a game's mods go
  - How it launches
  - How its sandboxes behave

All of these live in `plugins/app/<slug>/` and are compiled into the binary.

Adding a game is four small JSON files and no Rust; [`scripts/gen-app-rules.py`](scripts/gen-app-rules.py) writes the first draft.

**Plugins are data, not code**: They describe steps the app carries out inside a jail the user grants, and every action is written to the activity log.

## Supported Games
Here's a full list of the games that the app currently *supports*. The **Mods land in** column is where the app will deploy mods for that game, and the "Tested" column indicates whether or not the deployment has been tested with that game.

All games right now **haven't been tested**. We will be testing and verifying the deployment of mods for each game in the future.

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

**WARNING**: A few games refuse to load mods at all while their anti-cheat is active. Valheim
and 7 Days to Die both want it switched off in their launcher first. The app
cannot do that for you and says so rather than deploying into a folder the game
will ignore.

Your game is not here? It is four JSON files under `plugins/app/<slug>/` and no
code. See **Adding a game** in `CLAUDE.md` for more details.

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
