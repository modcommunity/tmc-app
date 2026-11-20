# Building and shipping the GameSpice App

Five targets, one codebase — but not one machine. This is what can be produced from where, and how.

| Artifact | Built on | Command |
| --- | --- | --- |
| `GameSpice App.exe` (portable), `*-setup.exe`, `*.msi` | **Linux or Windows** | `npm run build:windows` |
| `*.AppImage`, `*.deb` | Linux | `npm run build:linux` |
| `*.dmg`, `GameSpice App.app` | **macOS only** | `npx tauri build --target universal-apple-darwin` |
| `GameSpice_x.y.z.apk` / `.aab` | Linux, macOS or Windows | `npm run android:build` (CI: the `android` job) |
| `GameSpice_x.y.z.ipa` | macOS only | `npm run ios:build` (CI: the `ios` job) |

The interesting row is the first one: **the whole Windows build comes out of the Linux box**, installers included, so a developer without a Windows machine — or without access to `@modcommunity/shared` on the one they have — can still hand somebody an exe.

## Windows, from Linux

```bash
sudo apt install clang lld llvm nsis wixl
sudo ln -s "$(command -v clang)" /usr/local/bin/clang-cl
rustup target add x86_64-pc-windows-msvc
cargo install cargo-xwin --locked

npm run build:windows
```

Three separate problems, three answers:

- **Compiling.** `cargo-xwin` downloads the MSVC CRT and the Windows SDK headers and libraries, and points the build at `clang-cl` and `lld-link` in place of `cl.exe` and `link.exe`. That is also what compiles bundled SQLite for Windows — the thing [architecture.md](architecture.md) names as the reason `tmc-core` could not be cross-checked the way `tmc-usvfs` is. It can now, at the cost of a toolchain download.
- **The .exe setup.** Tauri's NSIS bundler already runs anywhere; it wants `makensis` on `PATH` and fetches the rest itself.
- **The .msi.** Tauri builds this by shelling out to WiX 3's `candle.exe` and `light.exe`, so `--bundles msi` refuses on a non-Windows host. `scripts/make-msi.sh` compiles `packaging/windows/tmc.wxs` with `wixl` from msitools instead — a native reimplementation of enough of WiX to do this particular job.

Out of `npm run build:windows`:

```
…/x86_64-pc-windows-msvc/release/bundle/portable/GameSpice App.exe run it, no install
…/x86_64-pc-windows-msvc/release/bundle/nsis/GameSpice App_x.y.z_x64-setup.exe
…/x86_64-pc-windows-msvc/release/bundle/msi/GameSpice App_x.y.z_x64_en-US.msi
```

`npm run build:windows -- --no-installers` stops after the exe, which is the fast loop when all you want is something to run.

### What cross-building does not give you

- **A signature.** Tauri skips signing on a non-Windows host, so SmartScreen will warn on first run. Signing needs the certificate and either a Windows runner or a `bundler > windows > sign_command` pointed at `osslsigncode`.
- **A WebView2 bootstrapper in the MSI.** The .exe setup installs the runtime if it is missing; `wixl` supports no launch conditions at all, so the MSI can neither install it nor warn about it. Windows 11 ships it and Windows 10 gets it with Edge, so the gap is narrow — but it is why **the .exe setup is the recommended download** and the MSI is there for people who deploy by policy.
- **ARM64.** `--target aarch64-pc-windows-msvc` compiles, but nothing here has been run on an ARM Windows machine.

### What the MSI does

`packaging/windows/tmc.wxs`, per-machine, into `Program Files\GameSpice App`:

- `GameSpice App.exe`
- a Start Menu shortcut under *The Modding Community*
- the `gs://` and `tmc://` schemes in `HKLM\Software\Classes` — the app cannot register this for itself in a per-machine install, and without it every deep link in the app is dead
- an `Upgrade` row, so installing a newer version replaces the old one rather than sitting beside it

Inspect one without Windows: `msiinfo tables x.msi`, `msiinfo export x.msi File`.

## Linux

```bash
npm run build:linux                       # AppImage + deb
npm run build:linux -- --bundles deb,rpm
```

Needs the webkit2gtk stack (see the README) or the private sysroot from `npm run sysroot`, which the script sources automatically when the system headers are absent.

Two things that bite on a current distribution, both handled by the script and both reported by Tauri as the unhelpful `failed to run linuxdeploy`:

- **`librsvg2-dev`** — linuxdeploy's GTK plugin asks pkg-config for it and exits when nothing answers.
- **libfuse2** — linuxdeploy and appimagetool are themselves AppImages, and Debian 13 and Ubuntu 24.04 no longer ship the FUSE 2 library they mount themselves with. `APPIMAGE_EXTRACT_AND_RUN=1` is the escape hatch they both honour, and the script sets it when the library is missing.

## macOS

**Not from here, and not worth faking.** A cross-build would need the Apple SDK (whose licence does not permit redistribution), and what came out would be unsigned and un-notarised — which on any current macOS means Gatekeeper refuses to open it at all, not that it shows a warning. `osxcross` gets as far as a binary and no further.

So macOS builds come from the `macos-latest` runner in `.github/workflows/release.yml`, as a universal binary. Signing and notarisation are still to be wired up there: it needs an Apple Developer ID certificate in the repository secrets, at which point `tauri-action`'s existing environment variables (`APPLE_CERTIFICATE`, `APPLE_ID`, `APPLE_TEAM_ID`, …) do the rest.

## Android and iOS

`src-tauri/gen/` (the Gradle and Xcode projects) is **not committed**. `release.yml` generates it on every build, so a Tauri upgrade never leaves a stale template behind and nothing generated can drift from `tauri.conf.json`:

1. `tauri android init --ci` / `tauri ios init --ci`.
2. The icons: `src-tauri/icons/android/` into `app/src/main/res/`, `src-tauri/icons/ios/` into the `AppIcon.appiconset`.
3. `scripts/mobile-patch.mjs android|ios`, which adds the `gs://` and `tmc://` intent filters / `CFBundleURLTypes` if the generated project lacks them. The schemes are read from `plugins.deep-link.mobile` in `tauri.conf.json`, the same list the desktop build registers. After the build the same script **checks** the built manifest (Android) or the `Info.plist` inside the `.ipa` (iOS) and fails the job when a scheme is missing: a build without them would have dead `gs://` links.
4. `tauri android build --apk --aab --target aarch64 armv7 x86_64` (one universal APK and one AAB with all three ABIs), or `tauri ios build --export-method <method>`.

The Android version code is derived by Tauri from `version` (`major*1000000 + minor*1000 + patch`), so a higher tag is always a higher code.

Locally: `npm run android:init`, then `node scripts/mobile-patch.mjs android src-tauri/gen/android/app/src/main/AndroidManifest.xml`, then `npm run android:build`. Needs JDK 17, the Android SDK with an NDK (`NDK_HOME`), and the rust targets `aarch64-linux-android armv7-linux-androideabi x86_64-linux-android`. iOS needs a Mac with Xcode and `xcodegen`.

**Signing happens after the build**, outside Gradle (`apksigner` for the APK, `jarsigner` for the AAB), so the generated build files are never patched. Without the `ANDROID_KEYSTORE_*` secrets the APK is signed with a throwaway debug key and no AAB is published; the job summary says so. An iOS build needs the owner's Apple Developer account and is skipped, with a notice, without it. Secrets and how to create them: [RELEASING.md](RELEASING.md#android-and-ios-secrets).

Mobile files have no `.sig` and are not updater artifacts: they never enter `latest.json` or the site publish. People update from the store or by installing the newer file.

## The app icon

The icon is the GameSpice website's: the GS potion mark on the ember tile, from website-gs `public/brand/icon-1024.png`. `src-tauri/icons/app-icon.png` is the 1024 master: that tile with rounded corners. Every size and format in `src-tauri/icons/` comes from it:

```bash
npx tauri icon src-tauri/icons/app-icon.png -o src-tauri/icons
cp src-tauri/icons/icon.png public/icon.png      # the webview favicon
```

The one exception is `ios/`. App Store icons must be opaque, and iOS rounds them itself, so that folder comes from the square, unrounded `icon-1024.png` (`npx tauri icon <website>/public/brand/icon-1024.png -o <tmp>`, then copy its `ios/`). Inside the UI (the sidebar rail, the title bar beside the wordmark, Settings → App → About) the mark is drawn inline with no tile, by `src/components/gs-logo.tsx`; keep its paths in step with the website's `gamespice-mark.svg`.

## Cutting a release

```bash
git tag v0.2.0 && git push origin v0.2.0
```

`.github/workflows/release.yml` builds on all three desktop platforms plus Android and iOS, signs the bundles, writes `checksums.txt`, uploads everything plus the updater's `latest.json` to the S3 downloads bucket, publishes the GitHub release and — with the release token set — tells the website too. Installed copies of the app update themselves from the bucket's `latest.json`. There is no manual step; the secrets it needs are listed in [`RELEASING.md`](RELEASING.md).

`checksums.txt` is not decoration: `scripts/install.sh` **refuses to install** a release that does not have one, so a release published by hand without it is one the one-line installer will not touch.

CI installs `@modcommunity/shared` from GitHub Packages with `GITHUB_TOKEN`, falling back to a `PACKAGES_TOKEN` secret (a PAT with `read:packages`) where the package does not grant this repository access. That is also the answer to "I cannot `npm install` on my own Windows machine": you do not have to.

## The update signing key, the downloads bucket and auto-update

Moved to [`RELEASING.md`](RELEASING.md): the signing keypair (the public half is `src-tauri/updater.pub`, compiled into every build), the GitHub secrets and variables the release workflow reads, the S3 downloads bucket and its `latest.json`, and the in-app updater settings.

## Installing

**Linux**

```bash
curl -fsSL https://raw.githubusercontent.com/gamespice-platform/app/main/scripts/install.sh | sh
```

Downloads the AppImage for the running architecture, verifies it against the release's `checksums.txt`, and registers a launcher entry, an icon and the `gs://` scheme. No root — everything lands under `~/.local` unless it is run as root, in which case `/usr/local`. It does not call `sudo` for you.

```bash
… | sh -s -- --uninstall           # removes exactly what it wrote
… | sh -s -- --version v0.1.0      # a specific release
GS_APPIMAGE=./GameSpice-App_0.1.0_amd64.AppImage sh scripts/install.sh   # a build you made
```

Where libfuse2 is absent the AppImage is unpacked into `~/.local/lib/gamespice-app` and `~/.local/bin/gamespice-app` becomes a symlink to its `AppRun` — same files, no mount.

**Windows** — the `-setup.exe`, or the `.msi` for deployment. Both register `gs://`; the MSI is per-machine and needs elevation.
