#!/bin/sh
#
# TMC — one-line install for Linux.
#
#   curl -fsSL https://moddingcommunity.com/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/modcommunity/tmc-app/main/scripts/install.sh | sh
#
# Downloads the published AppImage, verifies it against the release's
# checksums.txt, and puts it somewhere the desktop can find it — a launcher
# entry, an icon, and the `tmc://` scheme so deep links work.
#
# Deliberate properties, because this is a script people pipe into a shell:
#
#   - POSIX sh, no bashisms. It runs under dash, which is /bin/sh on Debian.
#   - It never needs root. Without it, everything lands under ~/.local; run it
#     as root and it lands under /usr/local instead. It does not call sudo for
#     you — a script that escalates on your behalf is a script you cannot read
#     the consequences of.
#   - The checksum is REQUIRED. A download that cannot be verified is refused
#     rather than installed with a warning nobody reads. `--no-verify` exists
#     for the case where you know the release predates checksums.txt.
#   - It touches nothing outside the prefix and ~/.local/share/applications,
#     and `--uninstall` removes exactly what it wrote.
#
# Environment:
#   TMC_REPO      owner/name to download from        (default modcommunity/tmc-app)
#   TMC_VERSION   a release tag, e.g. v0.2.0         (default: latest)
#   TMC_PREFIX    install prefix                     (default ~/.local, /usr/local as root)
#   TMC_APPIMAGE  install this local file instead of downloading — which is how
#                 you install a build you made yourself
set -eu

REPO="${TMC_REPO:-modcommunity/tmc-app}"
VERSION="${TMC_VERSION:-}"
APPIMAGE="${TMC_APPIMAGE:-}"
verify=1
mode=install
extract=auto

if [ "$(id -u)" -eq 0 ]; then
    PREFIX="${TMC_PREFIX:-/usr/local}"
    DATA="$PREFIX/share"
else
    PREFIX="${TMC_PREFIX:-$HOME/.local}"
    DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
fi

while [ $# -gt 0 ]; do
    case "$1" in
        --uninstall) mode=uninstall ;;
        --no-verify) verify=0 ;;
        --extract) extract=yes ;;
        --version) VERSION="${2:?--version needs a tag}"; shift ;;
        --prefix) PREFIX="${2:?--prefix needs a path}"; DATA="$PREFIX/share"; shift ;;
        -h|--help) sed -n '2,32p' "$0" 2>/dev/null || echo "see scripts/install.sh"; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
    shift
done

BIN="$PREFIX/bin/tmc"
LIBDIR="$PREFIX/lib/tmc"
DESKTOP="$DATA/applications/tmc.desktop"
ICON="$DATA/icons/hicolor/256x256/apps/tmc.png"

say()  { printf '%s\n' "$*"; }
warn() { printf '%s\n' "$*" >&2; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }

# ------------------------------------------------------------------ uninstall

if [ "$mode" = uninstall ]; then
    rm -rf "$BIN" "$LIBDIR" "$DESKTOP" "$ICON"
    command -v update-desktop-database >/dev/null 2>&1 &&
        update-desktop-database "$DATA/applications" 2>/dev/null || true
    say "Removed TMC from $PREFIX."
    say "Your settings and library are untouched, in ${XDG_CONFIG_HOME:-$HOME/.config}/com.moddingcommunity.app."
    exit 0
fi

# ------------------------------------------------------------------- download

case "$(uname -s)" in
    Linux) ;;
    Darwin) die "macOS builds are .dmg downloads, not this script — see https://moddingcommunity.com/download" ;;
    *) die "$(uname -s) is not supported by this installer." ;;
esac

case "$(uname -m)" in
    x86_64|amd64) arch=amd64 ;;
    aarch64|arm64) arch=aarch64 ;;
    *) die "no AppImage is published for $(uname -m)." ;;
esac

fetch() {
    # $1 url, $2 destination ("-" for stdout)
    if command -v curl >/dev/null 2>&1; then
        if [ "$2" = "-" ]; then curl -fsSL "$1"; else curl -fsSL -o "$2" "$1"; fi
    elif command -v wget >/dev/null 2>&1; then
        if [ "$2" = "-" ]; then wget -qO- "$1"; else wget -qO "$2" "$1"; fi
    else
        die "neither curl nor wget is installed."
    fi
}

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

if [ -n "$APPIMAGE" ]; then
    [ -f "$APPIMAGE" ] || die "$APPIMAGE does not exist."
    cp "$APPIMAGE" "$tmp/tmc.AppImage"
    say "Installing $APPIMAGE"
else
    if [ -z "$VERSION" ]; then
        # The redirect on /releases/latest names the tag, which the API also
        # does — but the API is rate-limited per IP at 60 requests an hour and
        # a shared NAT burns that on somebody else's behalf.
        if command -v curl >/dev/null 2>&1; then
            resolved="$(curl -fsSLI -o /dev/null -w '%{url_effective}' \
                "https://github.com/$REPO/releases/latest" || true)"
        else
            resolved="$(wget -qS --max-redirect=10 -O /dev/null \
                "https://github.com/$REPO/releases/latest" 2>&1 |
                sed -n 's/^ *Location: *//p' | tail -1 || true)"
        fi
        VERSION="${resolved##*/}"
        case "$VERSION" in
            ''|latest|*/*) die "could not work out the latest release of $REPO — pass --version <tag>." ;;
        esac
    fi

    base="https://github.com/$REPO/releases/download/$VERSION"
    number="${VERSION#v}"
    asset="TMC_${number}_${arch}.AppImage"

    say "Downloading TMC $VERSION ($arch)…"
    fetch "$base/$asset" "$tmp/tmc.AppImage" ||
        die "no $asset in release $VERSION — that architecture may not be published."

    if [ "$verify" -eq 1 ]; then
        fetch "$base/checksums.txt" "$tmp/checksums.txt" ||
            die "release $VERSION publishes no checksums.txt; re-run with --no-verify only if you understand that."
        want="$(awk -v a="$asset" '$2 == a || $2 == "*" a { print $1 }' "$tmp/checksums.txt" | head -1)"
        [ -n "$want" ] || die "checksums.txt does not mention $asset."
        if command -v sha256sum >/dev/null 2>&1; then
            got="$(sha256sum "$tmp/tmc.AppImage" | cut -d' ' -f1)"
        elif command -v shasum >/dev/null 2>&1; then
            got="$(shasum -a 256 "$tmp/tmc.AppImage" | cut -d' ' -f1)"
        else
            die "no sha256sum or shasum to verify with."
        fi
        [ "$want" = "$got" ] || die "checksum mismatch — expected $want, got $got. Not installing."
        say "Checksum verified."
    fi
fi

chmod +x "$tmp/tmc.AppImage"

# --------------------------------------------------------------------- place

# An AppImage mounts itself with libfuse2, which several current distributions
# no longer ship (Debian 13, Ubuntu 24.04). Where it is absent the image is
# unpacked instead and `tmc` becomes a symlink to its AppRun — the same files,
# without the mount.
if [ "$extract" = auto ]; then
    # `ldconfig` lives in /sbin, which is not on a normal user's PATH.
    if PATH="/sbin:/usr/sbin:$PATH" ldconfig -p 2>/dev/null | grep -q 'libfuse\.so\.2'
    then extract=no; else extract=yes; fi
fi

mkdir -p "$PREFIX/bin" "$DATA/applications" "$(dirname "$ICON")"

# Unpack regardless: the icon and the desktop entry come out of the image, and
# in FUSE mode the unpacked copy is thrown away with $tmp.
( cd "$tmp" && ./tmc.AppImage --appimage-extract >/dev/null 2>&1 ) ||
    warn "could not unpack the AppImage; carrying on without its icon."

if [ -d "$tmp/squashfs-root" ]; then
    src="$(find "$tmp/squashfs-root" -name '*.png' -path '*256x256*' | head -1)"
    [ -n "$src" ] || src="$(find "$tmp/squashfs-root" -maxdepth 1 -name '*.png' | head -1)"
    [ -n "$src" ] && cp "$src" "$ICON"
fi

if [ "$extract" = yes ]; then
    [ -d "$tmp/squashfs-root" ] || die "this system has no libfuse2 and the AppImage could not be unpacked."
    rm -rf "$LIBDIR"
    mkdir -p "$(dirname "$LIBDIR")"
    mv "$tmp/squashfs-root" "$LIBDIR"
    rm -f "$BIN"
    ln -s "$LIBDIR/AppRun" "$BIN"
    say "No libfuse2 here, so the image was unpacked into $LIBDIR."
else
    rm -rf "$LIBDIR"
    install -m 755 "$tmp/tmc.AppImage" "$BIN"
fi

# ------------------------------------------------------- desktop registration

# Written here rather than taken from the image: this is where `tmc://` is
# claimed, and a link that opens the wrong copy of the app is worse than one
# that opens nothing. `%u` is what passes the link through to the running
# instance.
cat > "$DESKTOP" <<DESK
[Desktop Entry]
Type=Application
Name=TMC
GenericName=Mod Manager
Comment=Browse, install and manage mods, assets and servers.
Exec=$BIN %u
Icon=tmc
Terminal=false
Categories=Utility;Game;
MimeType=x-scheme-handler/tmc;
StartupWMClass=TMC
DESK

command -v update-desktop-database >/dev/null 2>&1 &&
    update-desktop-database "$DATA/applications" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null 2>&1 &&
    gtk-update-icon-cache -qtf "$DATA/icons/hicolor" 2>/dev/null || true
command -v xdg-mime >/dev/null 2>&1 &&
    xdg-mime default tmc.desktop x-scheme-handler/tmc 2>/dev/null || true

say ""
say "TMC is installed at $BIN"
case ":$PATH:" in
    *":$PREFIX/bin:"*) say "Run it with: tmc" ;;
    *) say "$PREFIX/bin is not on your PATH — run it with: $BIN"
       say "  (or add it: echo 'export PATH=\"$PREFIX/bin:\$PATH\"' >> ~/.profile)" ;;
esac
say "Uninstall with the same script: … | sh -s -- --uninstall"
