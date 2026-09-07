#!/usr/bin/env bash
#
# Install @modcommunity/shared from the sibling ../tmc-global checkout instead
# of the registry, for iterating on the design system and the app together.
#
# Installs with --no-save, so package.json keeps its registry spec and a later
# `npm ci` / `npm install` restores the published package.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
shared="$(cd "$here/../../tmc-global/shared" && pwd)"

echo "→ building @modcommunity/shared"
(cd "$shared" && npm run build >/dev/null)

echo "→ packing"
tgz="$(cd "$shared" && npm pack 2>/dev/null | tail -n1)"

echo "→ installing $tgz (--no-save)"
npm install "$shared/$tgz" --no-save --no-audit --no-fund >/dev/null
rm -f "$shared/$tgz"

echo "✓ @modcommunity/shared installed from ../tmc-global/shared"
