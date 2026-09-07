#!/usr/bin/env bash
#
# `cargo <args>` against the workspace, with the local GTK sysroot applied when
# one is needed and present.
#
# On a machine with the -dev packages properly installed this is a plain cargo
# invocation. On one without them (and without root) it picks up the sysroot
# built by `linux-sysroot.sh`, so the same command works in both places and CI
# does not need a second recipe.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [ "$(uname -s)" = "Linux" ] && ! pkg-config --exists webkit2gtk-4.1 2>/dev/null; then
    # shellcheck source=/dev/null
    . "$here/linux-env.sh"
fi

# `cd` rather than `--manifest-path`: the flag would have to be injected before
# any `--` separator, and callers legitimately pass one (`clippy … -- -D
# warnings`). Changing directory keeps the argument list untouched.
cd "$here/../src-tauri"

exec cargo "$@"
