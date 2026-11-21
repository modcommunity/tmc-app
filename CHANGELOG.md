The GameSpice App's releases, newest first. A version here is the one in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`; the release workflow refuses a tag that disagrees with `tauri.conf.json`.

## 0.4.1

- **Play buttons**: left-click on Quick play / Play / Play solo now starts the installed native build with no dialog, falling back to the web build with a visible warning if native is unavailable. Right-click opens the advanced options (native, install, web, sandbox, connect).

## 0.4.0

- **Close to the system tray** (Settings → Advanced, desktop, OFF by default): closing the window keeps GameSpice running in a tray icon (Open / Quit; left-click opens). If the OS has no tray, closing quits as before rather than leaving an unreachable app.
- **Desktop notifications** (Settings → Advanced, OFF by default): while the app is running — in the tray or with the window open — it checks your notifications on an interval you choose (30 s to 15 min) and shows each new one as a styled card in the bottom-right corner, themed per kind (friends, parties, messages, releases, credits…). Needs the site's `GET /api/app/v1/notifications`. The first check only records what already exists, so nothing old is announced; bursts collapse into one card; nothing is shown while the app is in front. Click a card to open its link (site links only); text from the server is only ever shown as text. Needs `libayatana-appindicator3-1` on Linux (added to the deb dependencies).

## 0.3.1

- **Release build fix**: the Tauri CLI rejected the empty updater key while signing, so the 0.3.0 tag produced no release. The workflow now passes the key from `src-tauri/updater.pub`. Same app as 0.3.0.

- **Android and iOS builds in the release pipeline**: tagged releases now also attach `GameSpice_<version>.apk`, `.aab` and `.ipa` (checksummed with the rest). Android is signed with the owner's keystore when `ANDROID_KEYSTORE_*` secrets exist, otherwise debug-signed and without an AAB; iOS needs the Apple Developer secrets and is skipped without them. Both `gs://` and `tmc://` are checked in the built app. See `docs/RELEASING.md`.

## 0.3.0

The first release as the GameSpice App, published from `gamespice-platform/app`.

- **Renamed to GameSpice App**: new crate and binary name, data folders and keyring entries migrated on first start, the `gs://` deep-link scheme, the GameSpice theme and icons, and a small "Powered by TMC" footer. Release files are named `GameSpice-App_*`.
- **Updates from the new home**: new installs poll `latest.json` on the `gamespice-platform/app` releases (or the downloads bucket when one is configured). Installs from before the rename keep updating through the bucket's `downloads/tmc-app/` manifests and the bridge release on `modcommunity/tmc-app` (`scripts/bridge-release.sh`). The updater key is unchanged.
- **GameSpice hub**: the games catalogue, a hub per game with its live servers, points and outfits, and pack discovery.
- **Avatar studio**: pick an avatar, wearables and emotes per game, against the site's library.
- **Store and Stats**: the store directory, a game's store, inventory and loadout; stats overview and drill-down with an activity chart.
- **Workshop in the app**: browse, subscribe, pause, auto-update, a per-game picker and owner edit and delete.
- **Safer workshop updates**: downloads are verified against the server's digest, an installed mod is kept when its update fails, interrupted updates recover, and a failed uninstall keeps the subscription and its journal.
- Fixed: a game just updated kept offering "Update to" the build it was already on.
- Fixed: the footer could overlap content in narrow windows.

## 0.2.4

- Solo launch refuses a shell build that predates `--solo` instead of opening its menu.
- An uninstall step that fails keeps the subscription row and its journal.
