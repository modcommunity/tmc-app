# Shared by the build/test helpers. Sources the local sysroot only when one
# exists, so a machine with the packages properly installed is unaffected.
if [ -d "${TMC_SYSROOT:-$HOME/.local/tmc-sysroot}" ] && ! pkg-config --exists webkit2gtk-4.1 2>/dev/null; then
    # shellcheck source=/dev/null
    . "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/linux-env.sh"
fi
