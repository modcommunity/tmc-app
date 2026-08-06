# Point the toolchain at the local sysroot built by `linux-sysroot.sh`.
#
#   source ./scripts/linux-env.sh
#
# `source`, not execute — it only exports variables.
#
# The `.pc` files in a Debian package carry absolute `/usr/...` paths, so
# PKG_CONFIG_SYSROOT_DIR is what rewrites them to point inside the sysroot.
# Without it pkg-config reports include paths that do not exist and every build
# script fails with a header-not-found error that looks nothing like the real
# cause.

TMC_SYSROOT="${TMC_SYSROOT:-$HOME/.local/tmc-sysroot}"

if [ ! -d "$TMC_SYSROOT" ]; then
    echo "No sysroot at $TMC_SYSROOT — run ./scripts/linux-sysroot.sh first." >&2
else
    export PKG_CONFIG_SYSROOT_DIR="$TMC_SYSROOT"
    export PKG_CONFIG_PATH="$TMC_SYSROOT/usr/lib/x86_64-linux-gnu/pkgconfig:$TMC_SYSROOT/usr/share/pkgconfig:$TMC_SYSROOT/usr/lib/pkgconfig"

    # `--define-prefix` would fight PKG_CONFIG_SYSROOT_DIR; the sysroot var
    # already rewrites every path, and setting both double-prefixes them.
    export PKG_CONFIG_ALLOW_SYSTEM_CFLAGS=1
    export PKG_CONFIG_ALLOW_SYSTEM_LIBS=1

    # Link against the SYSTEM runtime libraries — the sysroot only supplies
    # headers and link stubs, and preferring its copies would produce a binary
    # bound to libraries the machine may not actually have at run time.
    export LIBRARY_PATH="$TMC_SYSROOT/usr/lib/x86_64-linux-gnu${LIBRARY_PATH:+:$LIBRARY_PATH}"

    echo "✓ toolchain pointed at $TMC_SYSROOT"
fi
