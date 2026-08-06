#!/usr/bin/env bash
#
# Copy the API contract across from website-city, which owns it.
#
# The app parses every response through these schemas, so the two copies have to
# agree — and a hand-edited copy is exactly how they stop agreeing. Run this
# after any change to `website-city/src/types/app-api/contract.ts`, then
# `npm run check:web`: a field the app relies on and the server stopped sending
# becomes a type error rather than a runtime surprise.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
src="$here/../../website-city/src/types/app-api/contract.ts"
dst="$here/../src/lib/api/contract.ts"

[ -f "$src" ] || {
    echo "No contract at $src — is website-city checked out next door?" >&2
    exit 1
}

header='/**
 * The contract for `/api/app/v1` — the app'"'"'s whole view of TMC.
 *
 * **This file is a VERBATIM MIRROR of
 * `website-city/src/types/app-api/contract.ts`, which is the source of truth.**
 * Copy it across whenever the server side changes; do not edit it here
 * (`npm run contract:sync` does the copy). Every response is parsed through
 * these schemas before it reaches a component, so a drift shows up as a loud
 * validation error on the first request rather than as an `undefined` three
 * screens deep.
 *'

# Replace the source-of-truth header block with the mirror one. Everything from
# the first `/**` to the line before ` * Why a separate API` is the header.
awk -v header="$header" '
    NR == 1 { print; next }
    /^\/\*\*$/ && !seen { seen = 1; print header; skipping = 1; next }
    skipping && /^ \* Why a separate API/ { skipping = 0 }
    !skipping { print }
' "$src" > "$dst"

echo "✓ contract mirrored into src/lib/api/contract.ts"
