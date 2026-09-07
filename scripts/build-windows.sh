#!/usr/bin/env bash
#
# Build the Windows app — the exe, the .exe setup and the .msi — FROM LINUX.
#
#   scripts/build-windows.sh [--no-installers] [--target <triple>]
#
# Tauri's own advice is to build each platform on that platform, and for macOS
# that is still the only answer (see `docs/BUILDING.md`). Windows is the
# exception: the MSVC target has no proprietary toolchain requirement that
# cannot be satisfied here.
#
#   - `cargo-xwin` fetches the MSVC CRT and Windows SDK headers/libs and points
#     the build at them, so `clang-cl` and `lld-link` stand in for cl.exe and
#     link.exe. That is what compiles bundled SQLite for Windows — the thing
#     CLAUDE.md names as the reason `tmc-core` could not be cross-checked.
#   - Tauri's NSIS bundler already runs anywhere; it needs `makensis` on PATH.
#   - The MSI does not — Tauri shells out to WiX, which is Windows-only — so
#     `make-msi.sh` compiles `packaging/windows/tmc.wxs` with `wixl` instead.
#
# What cross-building does NOT give you: a signed binary. Tauri skips signing
# on a non-Windows host, so what comes out is unsigned and SmartScreen will say
# so on first run. Signing needs the certificate and a Windows runner (or a
# `sign_command` pointed at osslsigncode) — CI's job, not this script's.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/.." && pwd)"

target="x86_64-pc-windows-msvc"
installers=1
while [ $# -gt 0 ]; do
    case "$1" in
        --no-installers) installers=0 ;;
        --target) target="${2:?--target needs a triple}"; shift ;;
        -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
    shift
done

missing=()
need() { command -v "$1" >/dev/null 2>&1 || missing+=("$2"); }

need clang-cl   "clang-cl (apt install clang; then: ln -s \$(command -v clang) /usr/local/bin/clang-cl)"
need lld-link   "lld-link (apt install lld)"
need llvm-rc    "llvm-rc (apt install llvm)"
[ "$installers" -eq 1 ] && need makensis "makensis (apt install nsis)"
[ "$installers" -eq 1 ] && need wixl     "wixl (apt install wixl)"
cargo xwin --version >/dev/null 2>&1 || missing+=("cargo-xwin (cargo install cargo-xwin --locked)")
rustup target list --installed | grep -qx "$target" || missing+=("rust target $target (rustup target add $target)")

if [ ${#missing[@]} -gt 0 ]; then
    echo "Missing cross-build prerequisites:" >&2
    printf '  - %s\n' "${missing[@]}" >&2
    exit 1
fi

# `--no-bundle` rather than `--bundles none`: "none" is not one of the values
# the CLI accepts, and the error it gives lists the Linux bundle formats, which
# reads like the target being wrong.
bundle_args=(--bundles nsis)
[ "$installers" -eq 0 ] && bundle_args=(--no-bundle)

# `--runner cargo-xwin` replaces `cargo` for the compile step only; the bundler
# still runs from this machine, which is why NSIS works and WiX does not.
XWIN_ACCEPT_LICENSE="${XWIN_ACCEPT_LICENSE:-1}" \
    npx --no-install tauri build --runner cargo-xwin --target "$target" "${bundle_args[@]}"

outdir="$root/src-tauri/target/$target/release"
exe="$(ls "$outdir"/*.exe 2>/dev/null | head -1 || true)"
[ -n "$exe" ] || { echo "no .exe under $outdir" >&2; exit 1; }

# The cargo binary is named after the crate; what people are handed is named
# after the product. A Tauri app is one file — the webview comes from Windows —
# so this copy is genuinely portable, and it is what to send somebody who wants
# to run it without installing anything.
product="$(node -e "process.stdout.write(require('$root/src-tauri/tauri.conf.json').productName)")"
mkdir -p "$outdir/bundle/portable"
cp "$exe" "$outdir/bundle/portable/$product.exe"

if [ "$installers" -eq 1 ]; then
    "$here/make-msi.sh" "$exe" >/dev/null
fi

echo
echo "Windows artifacts:"
echo "  $outdir/bundle/portable/$product.exe"

# Only what THIS run produced. A previous run's installers are still on disk,
# and listing them after `--no-installers` claims output that was not rebuilt.
if [ "$installers" -eq 1 ]; then
    find "$outdir/bundle/nsis" "$outdir/bundle/msi" -type f \
        \( -name '*.exe' -o -name '*.msi' \) -printf '  %p\n' 2>/dev/null
fi

exit 0
