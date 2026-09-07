#!/usr/bin/env bash
#
# Build the Windows MSI, on Linux, with `wixl` from msitools.
#
#   scripts/make-msi.sh <TMC.exe> [output.msi]
#
# Tauri bundles an MSI itself, by shelling out to WiX 3's candle.exe and
# light.exe — which is why `tauri build --bundles msi` refuses on anything but
# Windows. `wixl` is a native reimplementation of enough of WiX to compile a
# .wxs into an .msi from here, so the Windows installer comes out of the same
# machine as the Linux one.
#
# What it is NOT: WiX proper. `packaging/windows/tmc.wxs` sticks to the subset
# wixl covers and says so at the top. If that file grows a MajorUpgrade element
# or a WixUI dialog set, this stops working and the answer is CI on a Windows
# runner, not a workaround here.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/.." && pwd)"

exe="${1:-}"
if [ -z "$exe" ] || [ ! -f "$exe" ]; then
    echo "usage: $0 <path/to/TMC.exe> [output.msi]" >&2
    exit 2
fi
exe="$(cd "$(dirname "$exe")" && pwd)/$(basename "$exe")"

command -v wixl >/dev/null 2>&1 || {
    echo "wixl not found — install msitools (Debian/Ubuntu: apt install msitools)." >&2
    exit 1
}

conf="$root/src-tauri/tauri.conf.json"
read_conf() { node -e "process.stdout.write(String(require('$conf')$1))"; }

product="$(read_conf '.productName')"
version="$(read_conf '.version')"
publisher="$(read_conf '.bundle.publisher')"
description="$(read_conf '.bundle.shortDescription')"
homepage="https://moddingcommunity.com"
icon="$root/src-tauri/icons/icon.ico"

# The cargo binary is `tmc-app.exe`; what gets INSTALLED is named after the
# product, exactly as Tauri's own bundlers name it, so a machine that has had
# both installers on it does not end up with two differently-named exes and two
# `tmc://` registrations pointing at different files.
exename="$(read_conf ".bundle.windows && require('$conf').bundle.windows.mainBinaryName || require('$conf').mainBinaryName || require('$conf').productName")".exe

# MSI ProductVersion is major.minor.build and nothing else — the fourth field
# is ignored by the installer's version comparison, and a prerelease suffix is
# rejected outright. `0.2.0-beta.1` becomes `0.2.0`, which is the version the
# upgrade logic in the .wxs then reasons about.
msi_version="$(printf '%s' "$version" | sed 's/[-+].*$//' | awk -F. '{printf "%d.%d.%d", $1+0, $2+0, $3+0}')"

# Default alongside whatever bundles the same build produced, which for a
# cross-build is the windows target directory rather than the host's.
out="${2:-$(dirname "$exe")/bundle/msi/${product}_${version}_x64_en-US.msi}"
mkdir -p "$(dirname "$out")"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

sed -e "s|@PRODUCT@|$product|g" \
    -e "s|@VERSION@|$msi_version|g" \
    -e "s|@PUBLISHER@|$publisher|g" \
    -e "s|@DESCRIPTION@|$description|g" \
    -e "s|@HOMEPAGE@|$homepage|g" \
    -e "s|@EXENAME@|$exename|g" \
    -e "s|@EXEPATH@|$exe|g" \
    -e "s|@ICON@|$icon|g" \
    "$root/packaging/windows/tmc.wxs" > "$work/tmc.wxs"

wixl --arch x64 --output "$out" "$work/tmc.wxs"

echo "✓ $out"
