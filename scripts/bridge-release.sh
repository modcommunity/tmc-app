#!/usr/bin/env bash
# Carry installs from before the rename onto a GameSpice App release.
#
#   scripts/bridge-release.sh <tag> [--from gamespice-platform/app]
#                                   [--to modcommunity/tmc-app] [--dry-run]
#
# Builds from before the repository moved can have the OLD repository's
# `releases/latest/download/latest.json` as their updater endpoint. Nothing is
# released there any more, so without this they never hear of an update. This
# publishes a release with the same tag on the old repository whose ONLY asset
# is a `latest.json` naming the NEW repository's artifacts:
#
#   1. take `latest.json` from the new repository's release (the release
#      workflow attaches one to every signed release), or, for a release from
#      before it did, build one from its
#      `.sig` assets with `updater-manifest.mjs`, pointed at that release's
#      download URLs;
#   2. refuse unless the manifest's version is the tag's (without its `v`)
#      and every platform URL is on the new repository or the
#      downloads bucket — a bridge that points somewhere else is the one thing
#      worse than no bridge;
#   3. create the release on the old repository (or reuse it) and upload the
#      manifest over any earlier one.
#
# Signatures are copied, never made: the old installs verify them against the
# updater public key they were built with, which is unchanged
# (`src-tauri/updater.pub`), so a bundle the new workflow signed installs.
# Installs whose endpoint is the downloads bucket do not need this — the
# release workflow writes `downloads/tmc-app/latest.json` as well — nor do ones
# on the site's route. docs/updater.md has the whole picture.
#
# Needs `gh` signed in with write access to the old repository, and node.
set -euo pipefail

TAG=""
FROM="gamespice-platform/app"
TO="modcommunity/tmc-app"
DRY=0

while [ $# -gt 0 ]; do
    case "$1" in
        --from) FROM="${2:?--from needs owner/name}"; shift ;;
        --to) TO="${2:?--to needs owner/name}"; shift ;;
        --dry-run) DRY=1 ;;
        -h|--help) awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$0"; exit 0 ;;
        -*) echo "unknown option $1" >&2; exit 2 ;;
        *) TAG="$1" ;;
    esac
    shift
done

[ -n "$TAG" ] || { echo "usage: $0 <tag> [--from owner/name] [--to owner/name] [--dry-run]" >&2; exit 2; }

root="$(cd "$(dirname "$0")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

base="https://github.com/$FROM/releases/download/$TAG"

if gh release download "$TAG" -R "$FROM" -p latest.json -D "$tmp" 2>/dev/null; then
    echo "Using latest.json from $FROM $TAG"
else
    echo "$FROM $TAG has no latest.json; building one from its signatures"
    mkdir -p "$tmp/sigs"
    gh release download "$TAG" -R "$FROM" -p '*.sig' -D "$tmp/sigs"
    # The manifest needs only each artifact's name beside its signature; an
    # empty stand-in spares downloading every installer.
    for sig in "$tmp"/sigs/*.sig; do : > "${sig%.sig}"; done
    node "$root/scripts/updater-manifest.mjs" --dir "$tmp/sigs" \
        --version "${TAG#v}" --base-url "$base" --out "$tmp/latest.json"
fi

# Every URL must be the new repository's release or the downloads bucket.
node -e '
const m = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"))
const from = process.argv[2]
const want = process.argv[3]
if (m.version !== want) { console.error(`refusing, the manifest names version ${m.version}, not ${want}`); process.exit(1) }
const bad = Object.entries(m.platforms ?? {}).filter(([, p]) =>
    !(p.url.startsWith(`https://github.com/${from}/releases/download/`) ||
      /^https:\/\/[^/]+\/downloads\/gamespice-app\//.test(p.url)) || !p.signature)
if (!Object.keys(m.platforms ?? {}).length) { console.error("no platforms in the manifest"); process.exit(1) }
if (bad.length) { console.error("refusing, these do not point at the GameSpice App release:", bad.map(([k]) => k).join(", ")); process.exit(1) }
console.log(`version ${m.version}: ${Object.keys(m.platforms).join(", ")}`)
' "$tmp/latest.json" "$FROM" "${TAG#v}"

if [ "$DRY" -eq 1 ]; then
    cat "$tmp/latest.json"
    echo "Dry run: nothing published to $TO."
    exit 0
fi

prerelease=()
case "$TAG" in *-*) prerelease=(--prerelease) ;; esac

notes="The app is now the GameSpice App, released from https://github.com/$FROM. This release carries only the updater manifest, so installs from before the move update to $TAG: https://github.com/$FROM/releases/tag/$TAG"

gh release view "$TAG" -R "$TO" >/dev/null 2>&1 ||
    gh release create "$TAG" -R "$TO" --title "GameSpice App $TAG" --notes "$notes" "${prerelease[@]}"
gh release upload "$TAG" -R "$TO" "$tmp/latest.json" --clobber
echo "Bridged: https://github.com/$TO/releases/download/$TAG/latest.json"
