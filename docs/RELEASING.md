# Releasing the GameSpice App, and how it updates itself

```bash
git tag v0.3.0 && git push origin v0.3.0        # a release
git tag v0.4.0-beta.1 && git push origin v0.4.0-beta.1   # a beta
```

Bump `version` in `src-tauri/tauri.conf.json` (and `package.json`, `src-tauri/Cargo.toml`) to match first and tag that commit: the workflow refuses a tag that differs, because a manifest newer than the binary it points at is an update every installed app takes again on every check. Beyond that, this is the whole ceremony once the secrets below exist. `.github/workflows/release.yml` then:

1. **Builds and signs** on Linux (AppImage, deb), Windows (NSIS setup, MSI, portable exe) and macOS (universal `.app` + `.dmg`). With `TAURI_SIGNING_PRIVATE_KEY` set it turns on `createUpdaterArtifacts`, so every updatable bundle gets a `.sig`, and then runs `scripts/verify-updater-signature.mjs`, which **fails the job** if those signatures do not verify against the public key the app is compiled with.
2. **Uploads to the downloads bucket**: every file under `downloads/gamespice-app/<version>/` (immutable cache headers), plus that version's `latest.json`; then — last, because they are the switch — `downloads/gamespice-app/latest-beta.json` and, for a non-pre-release tag, `downloads/gamespice-app/latest.json` (`no-cache`). The same two switches are also written under `downloads/tmc-app/`, the endpoint compiled into builds from before the rename (see "The repository moved").
3. **Publishes the GitHub release** with the same files plus `latest.json` and `latest-beta.json` (the bucket's manifest when there is a bucket; otherwise one written by `scripts/updater-manifest.mjs` whose URLs are the release's own downloads, so a release with no bucket is still an update), every artifact named `GameSpice-App_*` (Tauri's `GameSpice App_*` with the space replaced), (pre-release tags are marked as such), which `scripts/install.sh` and the README links use.
4. **Tells the site** (`POST /api/app/v1/releases`) when `TMC_RELEASE_TOKEN` is set, with the bucket's URLs. Optional — see below.

Each optional step that is not configured is a **warning, not a failure**: an unsigned release is still downloadable, and a release with no bucket still lands on GitHub.

## Where the app looks for updates

`tmc_core::updater::resolve` decides, on every check, from three places:

| Input | `settings.json` override | Compiled into the build | Last resort |
| --- | --- | --- | --- |
| endpoint | `updaterEndpoint` in the app's `settings.json` (hand-edited) | `TMC_UPDATER_ENDPOINT` — the variable when set, else `<S3_PUBLIC_URL>/downloads/gamespice-app/latest.json` with a bucket, else `https://github.com/<this repo>/releases/latest/download/latest.json` | the site's `/api/app/v1/update/{{target}}/{{arch}}/{{current_version}}` |
| public key | `updaterPubkey` in the same file | `src-tauri/updater.pub` (or `TMC_UPDATER_PUBKEY` in the build environment) | **none: the app cannot install, and says so** |

The **beta** channel (Settings → App → Updates → Channel) reads `latest-beta.json` instead of `latest.json` (on the GitHub route that is the newest *stable* release's copy, because `releases/latest` never names a pre-release: betas reach testers only through the bucket); an endpoint containing `{{channel}}` gets `stable`/`beta` substituted instead.

`update_check` asks the updater's endpoint first (fetching the manifest only), and falls back to the site's `/version` when there is no key or the endpoint cannot be reached — so the "newer version" banner works on every build, and its button installs whenever a key is configured.

### The public key is in the repository

`src-tauri/updater.pub` is the public half of the release key and `src-tauri/build.rs` compiles it into **every** build, a developer's included. It is public by definition; the private half exists only in GitHub secrets and on the machine of whoever generated it. A fork that signs its own releases sets the `TMC_UPDATER_PUBKEY` repository variable (or replaces the file) — the environment wins over the file.

`scripts/fixtures/signed.txt.sig` was signed with the private key, and `scripts/verify-updater-signature.test.mjs` checks it against `updater.pub` on every `npm run check`, so the two halves provably match.

### The overrides, and what they cost

An administrator can set `updaterEndpoint` (an HTTPS manifest URL) and `updaterPubkey` (the text `tauri signer generate` printed) in the app's `settings.json`, **with the app closed** (a running app rewrites the file from memory). They exist so a staging bucket, a fork or a rotated key can be tested without a rebuild. The key decides what the app will install over itself, so:

- **nothing in the app can set them.** `settings_patch` refuses both and there is no command for them — the webview is assumed hostile, and code that can write `settings.json` already runs as the user;
- they are validated when the file is read — HTTPS only, no credentials in the URL, and a key that decodes as a minisign public key — and an invalid value is dropped, falling back to the built-in source;
- while either is in force it is written to the audit log at **Security** level at every start, which cannot be switched off, and it survives *Reset settings*;
- Settings → App → Updates → **Update source (administrators)** shows them read-only, expanded and marked *overridden*. Deleting the keys from the file returns to the built-in source.

With no key compiled in and none set, the app does not crash and does not install: Settings says "No update signing key is configured" and the banner offers the download page.

## One-time setup

### 1. The signing keypair

Already generated for this repository (2026-09-27) into `.secrets/`, which is gitignored:

| File | What it is | Where it goes |
| --- | --- | --- |
| `.secrets/tmc-updater.key.pub` | public key | already committed as `src-tauri/updater.pub` |
| `.secrets/tmc-updater.key` | **private key** | GitHub secret `TAURI_SIGNING_PRIVATE_KEY` (paste the file's contents) |
| `.secrets/tmc-updater.key.password` | its password | GitHub secret `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` |

**Back up the private key and its password somewhere other than this machine** (a password manager). It cannot be recovered, and losing it means every installed copy stops accepting updates until its user reinstalls by hand — a new key cannot sign for the old one. After it is in GitHub and backed up, the `.secrets/` copy can be deleted.

To start over with a new pair (only before the first signed release ships, or as a deliberate rotation that strands existing installs):

```bash
npx tauri signer generate --ci -p "<password>" -w .secrets/tmc-updater.key
cp .secrets/tmc-updater.key.pub src-tauri/updater.pub
npx tauri signer sign -f .secrets/tmc-updater.key -p "<password>" scripts/fixtures/signed.txt
```

### 2. GitHub secrets and variables

GitHub → this repository → Settings → Secrets and variables → Actions.

| Name | Kind | Value | Needed for |
| --- | --- | --- | --- |
| `PACKAGES_TOKEN` | Secret, optional | a token with `read:packages` on `@modcommunity/shared` | `npm ci`, when this repository's own `GITHUB_TOKEN` cannot read that package (it is another organisation's) |
| `TAURI_SIGNING_PRIVATE_KEY` | Secret | contents of `.secrets/tmc-updater.key` | signed, auto-installable releases |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Secret | contents of `.secrets/tmc-updater.key.password` | the same |
| `S3_BUCKET` | Secret | website-city's `S3_BUCKET` (the uploads bucket) | the downloads bucket |
| `S3_REGION` | Secret | website-city's `S3_REGION` | the same |
| `S3_ACCESS_KEY` | Secret | website-city's `S3_ACCESS_KEY` (or a key scoped to `downloads/gamespice-app/*` and `downloads/tmc-app/*`) | the same |
| `S3_ACCESS_SECRET` | Secret | website-city's `S3_ACCESS_SECRET` | the same |
| `S3_ENDPOINT` | Secret, optional | website-city's `S3_ENDPOINT`, for a non-AWS store (R2, B2, MinIO) | the same |
| `S3_FORCE_PATH_STYLE` | Variable, optional | `true` when website-city sets it (MinIO) | the same |
| `S3_PUBLIC_URL` | **Variable** | the public base the bucket is served from — website-city's `NEXT_PUBLIC_CDN_URL`, e.g. `https://cdn.moddingcommunity.com` | the updater endpoint compiled into the app, and the URLs in `latest.json` |
| `TMC_RELEASE_TOKEN` | Secret, optional | the same value as `APP_RELEASE_TOKEN` on the website | announcing the release to the site's `/version` and `/update` routes |
| `TMC_SITE_URL` | Variable, optional | a staging site | the same |
| `TMC_UPDATER_PUBKEY` | Variable, optional | a different public key | a fork signing its own releases |
| `TMC_UPDATER_ENDPOINT` | Variable, optional | a different default manifest URL | overriding the bucket or GitHub default |

The minimum for a release that installed apps update to is the two `TAURI_SIGNING_*` secrets (plus `PACKAGES_TOKEN` if `npm ci` cannot read `@modcommunity/shared`); everything else is optional. Settings → Actions → General must also give workflows **read and write** permission, since the publish job creates the release with the job's own token.

The secret names match website-city's `.env` (`S3_BUCKET`, `S3_REGION`, `S3_ACCESS_KEY`, `S3_ACCESS_SECRET`, `S3_ENDPOINT`) so one set of values serves both. `S3_PUBLIC_URL` is a variable because it is not secret and it is compiled into the app; a secret of that name is accepted too.

The bucket must serve `downloads/gamespice-app/*` publicly at `S3_PUBLIC_URL` (website-city's uploads are already served that way through the CDN). The workflow does not set ACLs, since several S3-compatible stores reject them.

## Doing it by hand

```bash
# the manifest for a directory of signed artifacts
node scripts/updater-manifest.mjs --dir artifacts --version 0.3.0 \
  --base-url https://cdn.example.com/downloads/gamespice-app/0.3.0 --out latest.json

# check signatures against the compiled-in key
node scripts/verify-updater-signature.mjs artifacts

# tell the site (prints what it would send)
TMC_RELEASE_TOKEN=… node scripts/publish-release.mjs --dir artifacts \
  --version 0.3.0 --base-url https://cdn.example.com/downloads/gamespice-app/0.3.0 \
  --promote --dry-run
```

`scripts/release-targets.mjs` is the one table mapping bundle filenames to platforms, shared by the manifest and the site publish: macOS ships the `.app.tar.gz` (a `.dmg` is mounted, not unpacked over a running app), and Windows prefers the NSIS `-setup.exe` over the `.msi`, which does not bootstrap WebView2. The manifest writes `{os}-{arch}` for the preferred file and `{os}-{arch}-{bundle}` for each, so an MSI install updates from the MSI.

**A rollback** is re-uploading the previous version's `latest.json` over `downloads/gamespice-app/latest.json` (it is kept in that version's directory), and, if the site is used, re-publishing it with `--promote`. That stops the bad version reaching anybody else; it does not take back those who already installed it, since the updater never offers an older version. Undoing it for them is a new, higher version.

## The Rust toolchain is pinned

Both workflows install `RUST_TOOLCHAIN` (currently `1.98`) instead of floating `stable`. A new stable brings new clippy lints and `-D warnings` makes each an error, which once failed a release tag on code that had passed review. Bump it in `ci.yml` and `release.yml` together, on a branch, so a new lint fails a pull request rather than a tag.

## The repository moved

The source repo is now `gamespice-platform/app` and the product is the GameSpice App. **The updater public key (`src-tauri/updater.pub`) is unchanged**, so a bundle the new workflow signs verifies in every old install. What has to be bridged is how an old install *finds* it, which depends on the endpoint it was built with:

| Old install's endpoint | How it reaches the new release | Who does it |
| --- | --- | --- |
| `<bucket>/downloads/tmc-app/latest.json` (the compiled default) | `release.yml` writes the manifests there as well as under `downloads/gamespice-app/` | automatic, every release |
| the site's `/api/app/v1/update` route | the site serves whatever `publish-release.mjs` sent it | automatic when `TMC_RELEASE_TOKEN` is set |
| `github.com/modcommunity/tmc-app/releases/latest/download/latest.json` | a bridge release on the old repo whose only asset is a `latest.json` naming the new repo's artifacts | **by hand**: `scripts/bridge-release.sh <tag>` |

`scripts/bridge-release.sh <tag>` takes `latest.json` from the `gamespice-platform/app` release (or, for a release from before the workflow attached one, builds it from the `.sig` assets pointed at that release's download URLs), refuses unless the manifest's version is the tag's and every URL is the new release or the `downloads/gamespice-app/` bucket path, then creates the same tag on `modcommunity/tmc-app` and uploads the manifest. `--dry-run` prints the manifest and publishes nothing. Run it once per release for as long as old installs matter; it needs `gh` with write access to the old repository. New installs never read the old repository.

**After the update the app is the GameSpice App.** On first start it moves its data directory from `…/com.moddingcommunity.app` to `…/gamespice-app` and leaves a link at the old path, and moves the stored sign-in to the `gamespice-app` keychain service (`tmc_core::rename`; [settings-and-logging.md](settings-and-logging.md#where-files-live)). The bundle identifier stays `com.moddingcommunity.app`, so the OS and the updater treat it as the same program.

**Windows: the product name changed** (`TMC` → `GameSpice App`), and the NSIS installer's directory and Start-menu entry follow it. The updater runs the new setup in place; check on the first bridged release that it replaces the old entry rather than adding a second one, and uninstall the old entry by hand if it does not.
