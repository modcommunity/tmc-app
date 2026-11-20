# Developing, checking and shipping

## Commands

```bash
npm install                # frontend deps (needs GitHub Packages auth for @modcommunity)
npm run dev                # vite only, no Rust — fastest loop for UI work
npm run desktop            # tauri dev
npm run desktop:build      # tauri build, for this machine
npm run build:linux        # AppImage + deb
npm run build:windows      # portable exe + .exe setup + .msi, cross-built

npm run android:init       # once, generates gen/android
npm run android            # tauri android dev
npm run ios:init           # once, macOS only
npm run ios                # tauri ios dev

npm run check              # everything. Run before declaring done
npm run check:web          # tsc + eslint + vitest
npm run check:rust         # clippy -D warnings, whole workspace
npm run test               # both sides
npm run test:web           # vitest
npm run test:rust          # cargo test, whole workspace
npm run test:core          # tmc-core only — no GTK stack needed, runs anywhere
npm run fmt                # prettier + cargo fmt

npm run contract:sync      # re-copy the API contract from website-city
npm run sysroot            # build the local GTK sysroot (see "Building on Linux without root")
npm run shared:local       # install @modcommunity/shared from ../tmc-global
npm run shared:build       # rebuild it in place
```

### Pointing a build at the dev site

```bash
TMC_API_BASE=https://tmcdev.net npm run desktop        # debug: read at RUN time
TMC_API_BASE=https://tmcdev.net npm run android        # baked in at BUILD time
TMC_API_BASE=https://tmcdev.net npm run build:windows  # so is a release build
```

**`https://tmcdev.net`, not `https://tmcdev.net:3002`.** Port 3002 is the Next dev server speaking plain HTTP; 443 is the one with TLS in front of it. Pointing `https://` at 3002 fails the handshake, which surfaces as "could not reach the site" and reads exactly like the box being down.

Two consequences of the dev site's certificate being signed by a private CA (`TMC Local Development CA`) rather than a public one, both of which cost an afternoon if you meet them without expecting them:

- **The machine running the build's OUTPUT has to trust that CA**, not the machine that built it. A cross-built exe handed to a Windows box which has never seen the dev CA fails every request with a TLS error.
- **`http://tmcdev.net:3002` is not a way around it.** `normalise_base` refuses `http` for anything but a loopback or LAN host, and a public-looking domain is neither however it resolves. A LAN address is: `http://<the box's LAN IP>:3002` is accepted, at the cost of `tauri-plugin-opener` being scoped to `https://*` — so sign-in cannot open the browser for you and the device screen prints the URL instead, which is exactly the case that behaviour exists for.

`tmc_core::api::api_base()` resolves the base once per process, from `TMC_API_BASE` in the environment (**debug builds only**) and otherwise from `TMC_API_BASE` at compile time, falling back to production. `core/build.rs` carries the `rerun-if-env-changed` that makes the compile-time half honest — without it a rebuild keeps the base the binary was FIRST built with.

Four properties, none of them incidental:

- **It is never a setting and never an argument from the webview.** There is no `api_set_base`; `api_env` is read-only. A "which server?" field is a phishing primitive — point the app at a look-alike and it sends that host a bearer token — and the same is true of a field a script in a mod description can reach.
- **A release build has no runtime path at all.** The environment of the process that launched the app is not a trust boundary: a `.desktop` file, a shortcut's "Start in", an installer's launch step all set one.
- **The value is validated, not trusted.** Scheme and host only — no path, no query, no credentials — reduced to an origin by `Url::origin`, and `http` is refused for anything but a loopback or LAN host. A refused override logs and falls back to the built-in base, because "my dev server saw no traffic" is a better failure than "my token went to a host I fat-fingered".
- **Each base gets its own stored session.** `secure.rs` keys the refresh token on `api_base_scope()`, so a dev run neither reads nor overwrites the production one. Sharing the entry meant the real refresh token was sent to the dev server on its first refresh — and a rejected refresh is terminal, so it also signed the developer out of the live site.

A non-production base is **shown** (title bar badge, Settings → App → Development, and the sign-in copy names the real host) and **audited** at Security level on launch. Nothing else on screen distinguishes staging from production, which is how a screenshot of dev data becomes a bug report about live data.

The OS hand-off (`tauri-plugin-opener`) stays scoped to `https://*`, so with a plain-`http` local base the app can browse and query but cannot open the login page or an article in the browser. The device screen prints the URL for exactly that case.

### Building on Linux without root

The Tauri target needs `webkit2gtk-4.1`, `libsoup-3.0`, `libgtk-3` and `libdbus-1` headers. Where you have root:

```bash
sudo apt install pkg-config build-essential \
  libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev \
  libsoup-3.0-dev libgtk-3-dev libdbus-1-dev
```

Where you do not — a shared dev box, a CI image, a container — `npm run sysroot` builds a private one instead. `apt-get download` needs no privileges and `dpkg -x` unpacks anywhere:

```bash
npm run sysroot                  # ~35 MB into ~/.local/tmc-sysroot
source ./scripts/linux-env.sh    # point pkg-config and the linker at it
npm run desktop
```

`scripts/cargo.sh` — which `check:rust` and `test:rust` go through — sources that automatically when the system headers are absent, so the same npm script works in both situations. The sysroot supplies headers and link stubs only; runtime libraries still come from the system where it has them.

## Shipping it

Four of the five targets can be built without owning the machine they run on. [BUILDING.md](BUILDING.md) is the full account and [RELEASING.md](RELEASING.md) the release process; what belongs here is the shape and the one thing that is easy to undo.

**Windows cross-builds from Linux, installers and all.** `scripts/build-windows.sh` runs `tauri build --runner cargo-xwin`, which is what makes bundled SQLite compile for Windows from here — the same gap that made `tmc-usvfs` its own crate, closed for the rest of the tree by a toolchain rather than by a split. NSIS already runs anywhere. **The MSI does not**: Tauri builds one by shelling out to WiX's `candle.exe`, so `scripts/make-msi.sh` compiles `packaging/windows/tmc.wxs` with `wixl` from msitools instead.

That file is written against **the subset wixl implements**, which is smaller than WiX's — no `MajorUpgrade`, no launch conditions, no `Platform` attribute, no WixUI. A change that reaches for one of those does not fail at review, it fails at `wixl` with a Vala assertion, and the fix is a Windows runner rather than a workaround. Its header says which elements are there and why.

The MSI registers `gs://` in `HKLM` itself, because a per-machine install is the one elevated moment there is and the app cannot claim the scheme for itself afterwards. It does **not** bootstrap the WebView2 runtime — wixl has no launch conditions, so it cannot even warn — which is why the `.exe` setup is the recommended download and the MSI is for policy deployment.

**macOS is the exception and stays one.** The SDK is not redistributable and an un-notarised `.app` is one Gatekeeper refuses outright rather than warns about, so a cross-build would produce something that cannot be run. It comes from the `macos-latest` runner in `.github/workflows/release.yml`.

**Nothing cross-built is signed.** Tauri skips signing on a foreign host and says so on every run. A build handed to somebody else gets a SmartScreen warning, and that is a fact about the build rather than about the machine it was made on.

### The one-line installer

`scripts/install.sh` is what `curl … | sh` runs, so it is written as a document somebody reads before piping it: POSIX `sh`, no `sudo`, `~/.local` unless run as root, and `--uninstall` removes exactly what it wrote.

**The checksum is required, not advisory.** It reads `checksums.txt` from the release and refuses to install without it — which makes the `publish` job's `sha256sum` step load-bearing: a release cut by hand without one is a release the installer will not touch.

It resolves the latest version from the **redirect** on `/releases/latest` rather than the GitHub API, which is rate-limited per IP at sixty an hour and would be spent by whoever shares the NAT.

Where **libfuse2** is missing — Debian 13, Ubuntu 24.04 — the AppImage is unpacked into `~/.local/lib/tmc` and `bin/tmc` becomes a symlink to its `AppRun`. The same problem hits the BUILD side, where linuxdeploy and appimagetool are themselves AppImages: `APPIMAGE_EXTRACT_AND_RUN=1` is what both honour, and Tauri reports its absence as the unhelpful `failed to run linuxdeploy` — as it does the missing `librsvg2-dev` the GTK plugin needs. `build-linux.sh` checks for both by name.

## Working here

### What CI runs, and why it is split that way

`.github/workflows/ci.yml` on every push and pull request, in four jobs rather than one:

| Job | Runs | Needs |
| --- | --- | --- |
| `web` | `check:web` — tsc, eslint, vitest | node |
| `core` | clippy, tests and `cargo fmt --check` for `tmc-core` + `tmc-usvfs` | **nothing else** |
| `shell` | `check:rust` and `test:rust` for the whole workspace | the GTK/WebKit stack |
| `windows-crosscheck` | `cargo check -p tmc-usvfs --target x86_64-pc-windows-gnu` | the Windows target |

**The `core` job is the one the workspace split exists for.** Every guard worth failing a pull request over — the path jail, the address guard, the protocol parsers, the token lifecycle — is in `tmc-core`, and that job proves them on a runner with no display stack, no WebKit and no dbus. If it ever starts needing `apt-get`, something has been added on the wrong side of the line.

`windows-crosscheck` is the only thing that looks at the injector at all: that code has never been run against a game, and type-checking it against the Windows target is the only check it gets.

`release.yml` is separate and builds the bundle on all three desktop platforms. It is not duplicated here — proving that a `.dmg` links on every push would cost twenty minutes to re-prove something that matters only when a release is cut.

`npm run audit:rust` is deliberately in neither: `cargo-audit` is a separate install, and wiring it into the definition of done would make a fresh clone fail its own check.

### Supply chain: `npm run audit:rust`

`cargo audit --deny warnings`, against `src-tauri/Cargo.lock`. Deliberately **not** part of `npm run check`: `cargo-audit` is a separate `cargo install` and is not in the toolchain `check` assumes, so wiring it in would make the definition of done depend on a tool a fresh clone does not have. Install it with `cargo install cargo-audit --locked`.

**Deny-on-warning, with a reviewed allow-list** in `src-tauri/.cargo/audit.toml`. An unfiltered run reports the same advisories every time, which trains everyone to stop reading it — and then a real one lands unnoticed. Filtered, the command is silent until something new appears. Ignores are per **advisory id**, not per crate, so a genuine new advisory against `gtk` still fails the run.

The `unmaintained`/`unsound` ones arrive through Tauri's own tree — the gtk-rs GTK3 bindings behind the Linux WebKitGTK webview (Linux-only; absent from the Windows, macOS, Android and iOS builds), and `urlpattern` inside `tauri-build`, which runs at compile time and is not linked into the app. `audit.toml` says which is which and why. The three `zip` 4.6.1 advisories (`tauri-plugin-updater`, which extracts only a signature-checked archive) and `unsafe-libyaml` (via `serde_yaml_ng`) are also listed; drop the `zip` ones when the plugin moves to zip 9.

**Re-check the list whenever Tauri is upgraded.** An entry that stopped appearing should be deleted from `audit.toml`, not left behind as a comment.
