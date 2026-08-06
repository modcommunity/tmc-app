#!/usr/bin/env bash
#
# Build the GTK/WebKit development headers into a LOCAL sysroot, for machines
# where you cannot `sudo apt install` them.
#
# Tauri's Linux target needs webkit2gtk-4.1, libsoup-3.0 and friends at compile
# time. On a normal workstation `apt install` is the answer and this script is
# unnecessary. On a shared dev box, a CI image or a container without root, it
# is the difference between "cargo build works" and "cargo build cannot run at
# all" — `apt-get download` needs no privileges, and `dpkg -x` unpacks into any
# directory you can write.
#
#   ./scripts/linux-sysroot.sh        # download + extract
#   source ./scripts/linux-env.sh     # point the toolchain at it
#   npm run desktop
#
# Everything the sysroot contains is headers and .pc files. The RUNTIME
# libraries still come from the system, which is correct: you want to link
# against the ones the machine will actually load.
set -euo pipefail

SYSROOT="${TMC_SYSROOT:-$HOME/.local/tmc-sysroot}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

ROOTS=(
    libwebkit2gtk-4.1-dev
    libjavascriptcoregtk-4.1-dev
    libsoup-3.0-dev
    libgtk-3-dev
    libdbus-1-dev
    libkrb5-dev
    # `libkrb5-dev` ships `krb5-gssapi.pc` as a symlink into `mit-krb5/`, and
    # the real files are in THIS package. Without it libsoup-3.0 fails to
    # resolve and nothing downstream of it builds — an error that names
    # Kerberos and gives no hint that WebKit is what you were building.
    krb5-multidev
)

echo "→ resolving the -dev closure"

# `apt-cache depends` one level at a time. Six rounds is comfortably deeper than
# this tree goes; the `seen` set makes it terminate regardless.
seen=""
queue="${ROOTS[*]}"

for _ in 1 2 3 4 5 6; do
    next=""

    for pkg in $queue; do
        case " $seen " in *" $pkg "*) continue ;; esac
        seen="$seen $pkg"

        deps="$(apt-cache depends --no-recommends --no-suggests --no-conflicts \
            --no-breaks --no-replaces --no-enhances "$pkg" 2>/dev/null |
            awk '/Depends:/ {print $2}' | grep -E -- '-dev$' | tr -d '<>' || true)"

        next="$next $deps"
    done

    queue="$next"
    [ -z "${queue// /}" ] && break
done

printf '%s\n' $seen | sort -u > "$WORK/pkgs.txt"

echo "→ downloading $(wc -l < "$WORK/pkgs.txt") packages"
(cd "$WORK" && xargs -a pkgs.txt apt-get download >/dev/null)

# The `-dev` packages ship only the `libfoo.so` link symlinks; the `.so.N` files
# they point at come from the runtime packages. Anything the system already has
# is re-pointed at the system copy below, but these three are the ones a machine
# without the app installed genuinely will not have.
RUNTIME=(
    libwebkit2gtk-4.1-0
    libjavascriptcoregtk-4.1-0
    libsoup-3.0-0
    libkrb5-3
    libgssapi-krb5-2
    libk5crypto3
    libkrb5support0
    libpcre2-16-0
    libpcre2-32-0
    libpcre2-posix3
    libharfbuzz-icu0
    libharfbuzz-cairo0
)

echo "→ downloading runtime libraries"
(cd "$WORK" && apt-get download "${RUNTIME[@]}" >/dev/null 2>&1 || true)

echo "→ extracting into $SYSROOT"
mkdir -p "$SYSROOT"

for deb in "$WORK"/*.deb; do
    dpkg -x "$deb" "$SYSROOT"
done

# Repoint every dangling `libfoo.so` at the system's runtime copy where one
# exists. Preferring the system library is deliberate: the binary should link
# against what the machine will actually load, and the sysroot is only here to
# supply headers and the link stubs the -dev packages carry.
echo "→ fixing link symlinks"

SYSLIB=/usr/lib/x86_64-linux-gnu
fixed=0
dangling=0

while IFS= read -r link; do
    target="$(readlink "$link")"
    dir="$(dirname "$link")"

    [ -e "$dir/$target" ] && continue

    if [ -e "$SYSLIB/$target" ]; then
        ln -sf "$SYSLIB/$target" "$link"
        fixed=$((fixed + 1))
    else
        dangling=$((dangling + 1))
    fi
done < <(find "$SYSROOT" -name '*.so' -type l)

echo "  repointed $fixed, still dangling $dangling (unused by the link line)"

cat <<EOF

✓ sysroot ready at $SYSROOT

  source ./scripts/linux-env.sh
  npm run desktop
EOF
