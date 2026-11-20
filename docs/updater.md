# Updating the app

`docs/RELEASING.md` is the operator's guide — the secrets, the bucket, the keypair. What belongs here is the shape and the rules.

| | `update_check` | `update_install` |
| --- | --- | --- |
| Asks | the updater's endpoint (manifest only), else `/api/app/v1/version` | the updater's endpoint |
| Answers | a version, and a download page when it came from the site | a signed artifact |
| Needs | nothing | a public key from somewhere, and desktop |
| Does | reports | replaces the running program |

**Both inputs are resolved per call by `tmc_core::updater::resolve`**: the `settings.json` override (hand-edited; no command sets it), else what was compiled in, else the fallback. The endpoint's compiled default is `TMC_UPDATER_ENDPOINT` — a static `latest.json` that needs nothing on the site: the one the release workflow uploads to the S3 downloads bucket when the bucket is configured, else the one it attaches to the GitHub release (`github.com/<repo>/releases/latest/download/latest.json`) — and its fallback is the site's `/update/:target/:arch/:current` route. The key's compiled default is `src-tauri/updater.pub`, which `build.rs` turns into `TMC_UPDATER_PUBKEY` unless the environment sets one; there is no fallback key. The beta channel reads `latest-beta.json`.

**The plugin is registered unconditionally on desktop** and handed the key with `updater_builder().pubkey(..)` on each call. `updater_builder()` panics without the plugin's managed state, so registering it only when a key was compiled in would make a runtime key impossible.

## The signature is the entire security model

`tauri-plugin-updater` verifies a minisign signature against the resolved key before it installs anything. TLS says who served the bytes; it says nothing about what they are. An unsigned self-updater converts a compromise of a bucket or the website's database into a compromise of every machine running the app.

So:

- **There is no placeholder key.** The key in `updater.pub` is a real one whose private half is a GitHub secret; with no key anywhere the app has no way to install and says so in Settings. It does not crash and it never installs unchecked.
- **The two halves are proven to match, twice.** `scripts/fixtures/signed.txt.sig` was signed with the private key and a vitest checks it against `updater.pub`; the release job runs `scripts/verify-updater-signature.mjs` over its own bundles and fails if they do not verify. A mismatched pair otherwise builds, signs and uploads perfectly and is refused by every user after the download.
- **`updaterEndpoint`/`updaterPubkey` are not settings a patch can touch.** Like the jail roots, `SettingsStore::patch` refuses them (`UPDATER_SOURCE_FIELDS`), and **no command sets them** — script in the webview must not be able to choose what installs next. They come only from a hand-edited `settings.json` (code that can write it already runs as the user): `sanitise` validates them on read (HTTPS, no credentials, a real minisign key) and drops what fails, `AppState::build` audits an override in force at Security level at every start, `reset` keeps them, and Settings shows them read-only. Do not add a setter back; SECURITY.md lists this.
- **`AppRelease.signature` is a required column** on the website, the publishing script refuses without one, and there is no "unsigned for now" branch — because that branch is the one somebody ships by accident.

## Cutting a release is pushing a tag

`release.yml` signs each bundle, verifies the signatures, uploads everything to `downloads/gamespice-app/<version>/` on the bucket, writes `latest-beta.json` (and, for a non-pre-release tag, `latest.json`) LAST because they are the switch, publishes the GitHub release with `latest.json` and `latest-beta.json` attached (written for the release's own URLs when there is no bucket), and — when `TMC_RELEASE_TOKEN` is set — posts the lot to `POST /api/app/v1/releases` in one call so the site's routes agree. Every step that is not configured warns and skips.

`scripts/release-targets.mjs` is the one filename → platform table, shared by the static manifest (`updater-manifest.mjs`) and the site publish (`publish-release.mjs`), and pinned by a vitest. macOS ships the `.app.tar.gz`, not the `.dmg`; Windows prefers the `-setup.exe` over the `.msi`, which does not bootstrap WebView2; the manifest carries `{os}-{arch}-{bundle}` keys too, so an MSI install updates from the MSI.

## Installs from before the rename

The updater public key is unchanged, so old installs verify new bundles; what they need is to *find* them. The workflow writes the manifests under the old `downloads/tmc-app/` prefix too, and `scripts/bridge-release.sh` publishes a `latest.json`-only release on `modcommunity/tmc-app` for installs that poll that repository. [RELEASING.md](RELEASING.md#the-repository-moved) has the table of who reaches the new release how.

## Three refusals worth keeping

- **A running game blocks an install.** Restarting the app out from under a supervisor thread holding a `Child` ends the play session with no duration and loses whatever the game had printed.
- **It does not relaunch.** Deciding for somebody that now is the moment to close their app is not the command's call, so the banner gains a third state — "installed, restart to finish" — and keeps showing it even after the notice was dismissed.
- **The site's route answers `204` for current.** The plugin treats anything else as a manifest, so an up-to-date client answered with a 200 would be cheerfully downgraded. The static manifest needs no such care: the plugin only installs a version newer than its own.
