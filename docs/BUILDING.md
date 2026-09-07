# Building and shipping TMC

Five targets, one codebase — but not one machine. This is what can be produced
from where, and how.

| Artifact | Built on | Command |
| --- | --- | --- |
| `TMC.exe` (portable), `*-setup.exe`, `*.msi` | **Linux or Windows** | `npm run build:windows` |
| `*.AppImage`, `*.deb` | Linux | `npm run build:linux` |
| `*.dmg`, `TMC.app` | **macOS only** | `npx tauri build --target universal-apple-darwin` |
| `*.apk` / `*.aab` | Linux, macOS or Windows | `npm run android:build` |
| `*.ipa` | macOS only | `npm run ios:build` |

The interesting row is the first one: **the whole Windows build comes out of the
Linux box**, installers included, so a developer without a Windows machine — or
without access to `@modcommunity/shared` on the one they have — can still hand
somebody an exe.

## Windows, from Linux

```bash
sudo apt install clang lld llvm nsis wixl
sudo ln -s "$(command -v clang)" /usr/local/bin/clang-cl
rustup target add x86_64-pc-windows-msvc
cargo install cargo-xwin --locked

npm run build:windows
```

Three separate problems, three answers:

- **Compiling.** `cargo-xwin` downloads the MSVC CRT and the Windows SDK
  headers and libraries, and points the build at `clang-cl` and `lld-link` in
  place of `cl.exe` and `link.exe`. That is also what compiles bundled SQLite
  for Windows — the thing `CLAUDE.md` names as the reason `tmc-core` could not
  be cross-checked the way `tmc-usvfs` is. It can now, at the cost of a
  toolchain download.
- **The .exe setup.** Tauri's NSIS bundler already runs anywhere; it wants
  `makensis` on `PATH` and fetches the rest itself.
- **The .msi.** Tauri builds this by shelling out to WiX 3's `candle.exe` and
  `light.exe`, so `--bundles msi` refuses on a non-Windows host.
  `scripts/make-msi.sh` compiles `packaging/windows/tmc.wxs` with `wixl` from
  msitools instead — a native reimplementation of enough of WiX to do this
  particular job.

Out of `npm run build:windows`:

```
…/x86_64-pc-windows-msvc/release/bundle/portable/TMC.exe          run it, no install
…/x86_64-pc-windows-msvc/release/bundle/nsis/TMC_x.y.z_x64-setup.exe
…/x86_64-pc-windows-msvc/release/bundle/msi/TMC_x.y.z_x64_en-US.msi
```

`npm run build:windows -- --no-installers` stops after the exe, which is the
fast loop when all you want is something to run.

### What cross-building does not give you

- **A signature.** Tauri skips signing on a non-Windows host, so SmartScreen
  will warn on first run. Signing needs the certificate and either a Windows
  runner or a `bundler > windows > sign_command` pointed at `osslsigncode`.
- **A WebView2 bootstrapper in the MSI.** The .exe setup installs the runtime
  if it is missing; `wixl` supports no launch conditions at all, so the MSI can
  neither install it nor warn about it. Windows 11 ships it and Windows 10 gets
  it with Edge, so the gap is narrow — but it is why **the .exe setup is the
  recommended download** and the MSI is there for people who deploy by policy.
- **ARM64.** `--target aarch64-pc-windows-msvc` compiles, but nothing here has
  been run on an ARM Windows machine.

### What the MSI does

`packaging/windows/tmc.wxs`, per-machine, into `Program Files\TMC`:

- `TMC.exe`
- a Start Menu shortcut under *The Modding Community*
- the `tmc://` scheme in `HKLM\Software\Classes` — the app cannot register this
  for itself in a per-machine install, and without it every deep link in the app
  is dead
- an `Upgrade` row, so installing a newer version replaces the old one rather
  than sitting beside it

Inspect one without Windows: `msiinfo tables x.msi`, `msiinfo export x.msi File`.

## Linux

```bash
npm run build:linux                       # AppImage + deb
npm run build:linux -- --bundles deb,rpm
```

Needs the webkit2gtk stack (see the README) or the private sysroot from
`npm run sysroot`, which the script sources automatically when the system
headers are absent.

Two things that bite on a current distribution, both handled by the script and
both reported by Tauri as the unhelpful `failed to run linuxdeploy`:

- **`librsvg2-dev`** — linuxdeploy's GTK plugin asks pkg-config for it and
  exits when nothing answers.
- **libfuse2** — linuxdeploy and appimagetool are themselves AppImages, and
  Debian 13 and Ubuntu 24.04 no longer ship the FUSE 2 library they mount
  themselves with. `APPIMAGE_EXTRACT_AND_RUN=1` is the escape hatch they both
  honour, and the script sets it when the library is missing.

## macOS

**Not from here, and not worth faking.** A cross-build would need the Apple SDK
(whose licence does not permit redistribution), and what came out would be
unsigned and un-notarised — which on any current macOS means Gatekeeper refuses
to open it at all, not that it shows a warning. `osxcross` gets as far as a
binary and no further.

So macOS builds come from the `macos-latest` runner in
`.github/workflows/release.yml`, as a universal binary. Signing and notarisation
are still to be wired up there: it needs an Apple Developer ID certificate in
the repository secrets, at which point `tauri-action`'s existing environment
variables (`APPLE_CERTIFICATE`, `APPLE_ID`, `APPLE_TEAM_ID`, …) do the rest.

## Cutting a release

```bash
git tag v0.2.0 && git push origin v0.2.0
```

`.github/workflows/release.yml` builds on all three desktop platforms, collects
the bundles, writes `checksums.txt` and publishes them to the GitHub release.

`checksums.txt` is not decoration: `scripts/install.sh` **refuses to install**
a release that does not have one, so a release published by hand without it is
one the one-line installer will not touch.

CI installs `@modcommunity/shared` from GitHub Packages with `GITHUB_TOKEN`,
falling back to a `PACKAGES_TOKEN` secret (a PAT with `read:packages`) where
the package does not grant this repository access. That is also the answer to
"I cannot `npm install` on my own Windows machine": you do not have to.

## Installing

**Linux**

```bash
curl -fsSL https://raw.githubusercontent.com/modcommunity/tmc-app/main/scripts/install.sh | sh
```

Downloads the AppImage for the running architecture, verifies it against the
release's `checksums.txt`, and registers a launcher entry, an icon and the
`tmc://` scheme. No root — everything lands under `~/.local` unless it is run as
root, in which case `/usr/local`. It does not call `sudo` for you.

```bash
… | sh -s -- --uninstall           # removes exactly what it wrote
… | sh -s -- --version v0.1.0      # a specific release
TMC_APPIMAGE=./TMC_0.1.0_amd64.AppImage sh scripts/install.sh   # a build you made
```

Where libfuse2 is absent the AppImage is unpacked into `~/.local/lib/tmc` and
`~/.local/bin/tmc` becomes a symlink to its `AppRun` — same files, no mount.

**Windows** — the `-setup.exe`, or the `.msi` for deployment. Both register
`tmc://`; the MSI is per-machine and needs elevation.
