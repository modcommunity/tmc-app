#!/usr/bin/env bash
#
# Build the Linux app: the binary, an AppImage and a .deb.
#
#   scripts/build-linux.sh [--bundles appimage,deb,rpm]
#
# Native, because there is nothing to cross — this IS the Linux box. The only
# thing it adds over `npm run desktop:build` is the sysroot fallback that
# `scripts/cargo.sh` already applies to check and test, so a machine without the
# webkit2gtk -dev packages (and without root) builds with the same command as
# one that has them.
#
# The AppImage is the artifact `scripts/install.sh` downloads: one file, no
# package manager, and the same binary on every distribution.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/.." && pwd)"

bundles="appimage,deb"
while [ $# -gt 0 ]; do
    case "$1" in
        --bundles) bundles="${2:?--bundles needs a list}"; shift ;;
        -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
    shift
done

if ! pkg-config --exists webkit2gtk-4.1 2>/dev/null; then
    # shellcheck source=/dev/null
    . "$here/linux-env.sh"
fi

case ",$bundles," in
    *,appimage,*)
        # linuxdeploy's GTK plugin asks pkg-config where librsvg lives and
        # exits non-zero when nothing answers. Tauri reports that as the
        # uninformative `failed to run linuxdeploy`, so it is worth catching
        # here with the package name attached.
        pkg-config --exists librsvg-2.0 2>/dev/null ||
            { echo "librsvg-2.0 development files are missing — the AppImage's GTK plugin needs them (apt install librsvg2-dev)." >&2; exit 1; }
        command -v patchelf >/dev/null 2>&1 ||
            { echo "patchelf is missing (apt install patchelf)." >&2; exit 1; }

        # linuxdeploy and appimagetool are themselves AppImages, so they mount
        # themselves with libfuse2 — which Debian 13 and Ubuntu 24.04 no longer
        # ship. This is the escape hatch they both honour: unpack and run
        # instead of mounting.
        if ! /sbin/ldconfig -p 2>/dev/null | grep -q 'libfuse\.so\.2'; then
            export APPIMAGE_EXTRACT_AND_RUN=1
        fi
        ;;
esac

npx --no-install tauri build --bundles "$bundles"

outdir="$root/src-tauri/target/release"
echo
echo "Linux artifacts:"
find "$outdir/bundle" -maxdepth 2 -type f \
    \( -name '*.AppImage' -o -name '*.deb' -o -name '*.rpm' \) -printf '  %p\n' 2>/dev/null || true
