The GameSpice App's releases, newest first. A version here is the one in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`; the release workflow refuses a tag that disagrees with `tauri.conf.json`.

## Unreleased

- Polish: Apps catalogue and Settings pages show skeleton placeholders while loading; content cards lift and show a focus ring on hover/keyboard focus; rail links show a focus ring.
- Server browser, round 2: the Favourites view sorts locally by the column headers and the sort dropdown; a sort without a direction uses its natural one (names and maps A to Z, numbers most-first); the latency chart has axis labels and a hover read-out; the server page has a Favourite button.
- Accessibility and sandboxes: one shared dialog behaviour (focus moves in, Tab stays inside, Escape closes, focus returns to the opener) now backs the store item dialog, the play dialog, the launch confirmation, quick play, report and folder picker. Sandboxes: failed Update, Check again, Add missing and Delete actions now show the error instead of failing silently; Delete shows progress; a failed list load has Retry; the selected sandbox is marked for screen readers, and a stale sandbox link says so.

- Server browser polish: the table header sticks while scrolling, narrow windows drop columns instead of scrolling sideways, rows are keyboard-focusable (Enter or Space expands) with a player fill bar, a skeleton table replaces the card skeleton while loading, a failed load shows a Retry banner instead of "Nothing matched", the search box follows the URL when you switch tabs or go back, and an invalid `?sort=` falls back to the default.
- Hardening: the desktop app now runs a single instance (a second launch, or a `gs://` link while it is open, focuses the running window instead of starting a second process on the same library database). A panic hook writes every panic to `crash.log` in the logs folder (size-capped, never sent anywhere) as well as the console.
- Polish: a friendly "Something went wrong" crash screen with Reload and Copy details; a real "Page not found" screen for unknown routes (it used to bounce to Play); an offline banner with Retry when GameSpice cannot be reached; routes fade in instead of snapping; skeleton placeholders replace "Loading…" text on Friends, game pages and item pages.
- Fixes (routes review 2): the view page Back button goes to Play when the page was opened directly (it did nothing). Apps search waits 250ms after typing before querying. A friend's avatar retries when its URL changes. Store, per-game Stats and Plugins no longer hang or mislabel failures: a bad game id shows "does not exist", load errors show with a retry (log, plugins, game stats), and the store's website checkout no longer raises an unhandled error. Sandbox update, dependency, make-default and delete actions no longer raise unhandled errors when they fail. Plugin enable toggle likewise; plugin and key forms announce errors.
- Fixes (website links): article, user, mod, server and map links opened from the app no longer land on "Page Not Found" (articles go to /blog/<slug>, users to /u/<name>, mods/servers/maps under their game); older website deployments are corrected in the app.
- **Server Monitor windows**: a Monitor button beside Join (server rows, cards and the server page) opens a resizable window per server, so several can be open at once. Each has a fluid latency chart (gradient, grid, hover crosshair, timeouts marked), a synced player-count chart, min/avg/max/p95/jitter/loss cards, a 1m/5m/15m/1h/session range and pause, a live player list, server details and rules, and an RCON console (graph, split or RCON only; remembered) that reuses your saved RCON login or offers to set one up. Not yet exercised in a running build.
- Fixes (routes review): the Settings download speed limit and simultaneous-downloads controls now keep the value you choose (they snapped back). Installs: make default, delete and create now refresh the list, and failures show on the card. A malformed port in a join link and unknown Apps type/sort values in the URL are ignored. Copying the sign-in code, clearing the log, removing a plugin or trusted key and the test notification no longer raise unhandled errors when they fail.
- Fixes: left-click Play no longer hangs on "Starting..." when a game has no launch mode (the normal dialog now opens), and waits for the platform answer before picking the default mode; the play dialog is announced as a dialog. Signing out clears the previous account's cached settings and social data. A plugin theme chosen and then replaced no longer paints the stale theme's colours. Favourites no longer crash when local storage is blocked. Nav highlights match on path-segment boundaries. Media-query hooks no longer resubscribe on every render.
- Security: the play hand-off link now opens only gs://, tmc://, https:// or a known game-client scheme (it was a blocklist that let schemes like ms-msdt: through), and links with control characters are refused.
- **Workshop browse redesign**: pack cards with banner image (lazy, kind-coloured gradient fallback), kind/game badges, author, subscriber count, version/size/updated chips and a runs-code badge; state-aware Subscribe button with optimistic feedback; debounced search with icon, active-filter chips, skeleton grid, infinite scroll and clear empty/error states. The Subscribed and My packs tabs use the same cards.

## Unreleased

- Library > Stores is now a real page. A store is a named folder for downloads and installs, permanent or temporary. Temporary stores are emptied when the app closes and again at the next launch. Create, rename, set as default and delete stores, with used and free space shown on each card. Deleting a permanent store only forgets it and never touches its files. The default store is the download folder, so existing behaviour is unchanged. Choosing a store per download is not wired yet.

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
