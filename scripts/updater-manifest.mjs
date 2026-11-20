#!/usr/bin/env node
/**
 * Write the Tauri updater's static manifest for one release.
 *
 *   node scripts/updater-manifest.mjs \
 *     --dir artifacts --version 0.3.0 \
 *     --base-url https://cdn.example.com/downloads/gamespice-app/0.3.0 \
 *     --out latest.json [--notes "…"]
 *
 * Reads every `<file>.sig` in `--dir` beside its `<file>`, maps the pair to the
 * updater's platform keys through `release-targets.mjs` (the same table the
 * site publish uses), and refuses — non-zero, nothing written — when nothing
 * signed maps, because an empty manifest would tell every installed app that
 * there is no update rather than that the release is broken.
 */
import fs from 'node:fs'
import path from 'node:path'

import { buildUpdaterManifest } from './release-targets.mjs'

const argv = process.argv.slice(2)
const flag = (name, fallback = '') => {
    const i = argv.indexOf(`--${name}`)
    return i >= 0 && argv[i + 1] ? argv[i + 1] : fallback
}

const dir = flag('dir', 'artifacts')
const out = flag('out', 'latest.json')

try {
    const files = fs.readdirSync(dir)
    const artifacts = files
        .filter((f) => f.endsWith('.sig') && files.includes(f.slice(0, -4)))
        .map((sig) => ({
            name: sig.slice(0, -4),
            signature: fs.readFileSync(path.join(dir, sig), 'utf8'),
        }))

    const manifest = buildUpdaterManifest({
        version: flag('version').replace(/^v/, ''),
        baseUrl: flag('base-url'),
        notes: flag('notes'),
        artifacts,
    })

    fs.writeFileSync(out, `${JSON.stringify(manifest, null, 2)}\n`)
    console.log(`wrote ${out}: ${Object.keys(manifest.platforms).join(', ')}`)
} catch (error) {
    console.error(`\n${error instanceof Error ? error.message : String(error)}\n`)
    process.exit(1)
}
