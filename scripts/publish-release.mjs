#!/usr/bin/env node
//
// Publish a built release to the TMC website, so the app can install it.
//
//   node scripts/publish-release.mjs --dir artifacts --version 0.2.0 \
//     --base-url https://github.com/<owner>/<repo>/releases/download/v0.2.0 \
//     [--site https://moddingcommunity.com] [--notes "…"] \
//     [--promote] [--dry-run]
//
// The token comes from TMC_RELEASE_TOKEN in the environment, never a flag: a
// flag lands in a process listing and in a CI log's command echo.
//
// WHY THIS IS A SCRIPT AND NOT TEN LINES OF YAML
// ---------------------------------------------
// The whole job is mapping bundle FILENAMES to update targets, and those names
// are this repository's business — `tmc_0.2.0_amd64.AppImage`,
// `TMC_0.2.0_x64-setup.exe`, `TMC.app.tar.gz`. Expressing that as shell globs
// inside a workflow means the one thing most likely to break quietly (a bundler
// renaming its output) breaks somewhere nobody reads, in a language with no way
// to test it. Here it is one table and a dry run.
//
// WHAT IT REFUSES
// ---------------
// A `.sig` with no artifact beside it, an artifact whose name maps to no
// target, and a release with no signatures at all. All three are "the build did
// not produce what this expects", and publishing a partial release is the one
// outcome worth avoiding — see the endpoint's own docblock for why promoting a
// version some machines have no artifact for is worse than not publishing.

import fs from 'node:fs'
import path from 'node:path'

const argv = process.argv.slice(2)
const flag = (name, fallback = '') => {
    const i = argv.indexOf(`--${name}`)
    return i >= 0 && argv[i + 1] ? argv[i + 1] : fallback
}

const DIR = flag('dir', 'artifacts')
const VERSION = flag('version')
const BASE_URL = flag('base-url').replace(/\/+$/, '')
const SITE = flag('site', 'https://moddingcommunity.com').replace(/\/+$/, '')
const NOTES = flag('notes')
const PROMOTE = argv.includes('--promote')
const DRY = argv.includes('--dry-run')

const TOKEN = (process.env.TMC_RELEASE_TOKEN ?? '').trim()

function die(message) {
    console.error(`\n${message}\n`)
    process.exit(1)
}

/**
 * Filename → update target.
 *
 * Ordered, and the order matters twice. `.app.tar.gz` is tested before the
 * generic archive rules because it IS a tarball; and NSIS is preferred over the
 * MSI for Windows because the `.exe` setup is the recommended download — the
 * MSI does not bootstrap the WebView2 runtime, since wixl has no launch
 * conditions and cannot even warn (see docs/BUILDING.md).
 *
 * The architecture tests are deliberately loose: Tauri spells the same
 * architecture `amd64`, `x86_64` and `x64` depending on which bundler produced
 * the file, and a rule that knew only one of those would silently skip a
 * platform.
 */
const RULES = [
    {
        target: 'DARWIN_UNIVERSAL',
        rank: 0,
        test: (n) => n.endsWith('.app.tar.gz'),
    },
    {
        target: 'LINUX_AARCH64',
        rank: 0,
        test: (n) => n.endsWith('.AppImage') && /aarch64|arm64/i.test(n),
    },
    {
        target: 'LINUX_X86_64',
        rank: 0,
        test: (n) => n.endsWith('.AppImage') && /amd64|x86_64|x64/i.test(n),
    },
    {
        target: 'WINDOWS_AARCH64',
        rank: 0,
        test: (n) => n.endsWith('-setup.exe') && /aarch64|arm64/i.test(n),
    },
    {
        target: 'WINDOWS_X86_64',
        rank: 0,
        test: (n) => n.endsWith('-setup.exe') && /x64|x86_64|amd64/i.test(n),
    },
    // The MSI is the fallback for each Windows target, never the first choice.
    {
        target: 'WINDOWS_AARCH64',
        rank: 1,
        test: (n) => n.endsWith('.msi') && /aarch64|arm64/i.test(n),
    },
    {
        target: 'WINDOWS_X86_64',
        rank: 1,
        test: (n) => n.endsWith('.msi') && /x64|x86_64|amd64/i.test(n),
    },
]

function classify(name) {
    return RULES.find((rule) => rule.test(name)) ?? null
}

async function main() {
    if (!VERSION) die('Say which release: --version <v>.')
    if (!BASE_URL) die('Say where the artifacts are served from: --base-url <url>.')
    if (!BASE_URL.startsWith('https://'))
        die('--base-url must be https. The app refuses anything else.')

    if (!fs.existsSync(DIR)) die(`No such directory: ${DIR}`)

    const files = fs.readdirSync(DIR)
    const sigs = files.filter((f) => f.endsWith('.sig'))

    if (sigs.length === 0)
        die(
            `No .sig files in ${DIR}.\n` +
                '\n' +
                'That means the build was not signed, which means this release\n' +
                'cannot be installed from inside the app. Nothing was published.\n' +
                'See "The update signing key" in docs/BUILDING.md.'
        )

    /** target → the best-ranked artifact found for it. */
    const chosen = new Map()

    for (const sig of sigs) {
        const artifact = sig.slice(0, -'.sig'.length)

        if (!files.includes(artifact))
            die(
                `${sig} has no ${artifact} beside it.\n` +
                    'A signature without its artifact is a build that did not\n' +
                    'finish collecting; nothing was published.'
            )

        const rule = classify(artifact)

        if (!rule) {
            // NOT fatal: Tauri signs bundles this updater does not use (a .deb,
            // say). Skipping one is normal; skipping ALL of them is what the
            // empty check below catches.
            console.log(`skip   ${artifact} (no update target)`)
            continue
        }

        const existing = chosen.get(rule.target)

        if (existing && existing.rank <= rule.rank) {
            console.log(`skip   ${artifact} (${rule.target} already has a better artifact)`)
            continue
        }

        chosen.set(rule.target, {
            rank: rule.rank,
            artifact,
            signature: fs.readFileSync(path.join(DIR, sig), 'utf8').trim(),
        })
    }

    if (chosen.size === 0)
        die(
            'No signed artifact mapped to an update target.\n' +
                'Either the bundlers renamed their output or this release built\n' +
                'nothing the updater can use. Nothing was published.'
        )

    const artifacts = [...chosen.entries()].map(([target, found]) => ({
        target,
        url: `${BASE_URL}/${encodeURIComponent(found.artifact)}`,
        signature: found.signature,
    }))

    console.log(`\ntmc-app ${VERSION}${PROMOTE ? '  (promoting)' : ''}`)

    for (const a of artifacts) console.log(`  ${a.target.padEnd(16)} ${a.url}`)

    const payload = {
        version: VERSION,
        notes: NOTES || undefined,
        artifacts,
        promote: PROMOTE,
    }

    if (DRY) {
        console.log('\n--dry-run: nothing sent.\n')
        return
    }

    if (!TOKEN)
        die(
            'TMC_RELEASE_TOKEN is not set, so there is nothing to authenticate with.\n' +
                'Set it as a GitHub Actions secret in this repository; the same value\n' +
                'is APP_RELEASE_TOKEN on the website.'
        )

    const res = await fetch(`${SITE}/api/app/v1/releases`, {
        method: 'POST',
        headers: {
            'content-type': 'application/json',
            authorization: `Bearer ${TOKEN}`,
        },
        body: JSON.stringify(payload),
    })

    const text = await res.text()

    if (!res.ok)
        die(
            `The site answered ${res.status}.\n${text}\n\n` +
                (res.status === 404
                    ? 'A 404 here usually means APP_RELEASE_TOKEN is unset on the site,\n' +
                      'or the token does not match. The route answers 404 rather than\n' +
                      '401 on purpose, so those two look the same from here.'
                    : '')
        )

    console.log(`\npublished. ${text}\n`)

    if (!PROMOTE)
        console.log(
            'NOT promoted: nothing is offered to anybody until app.version.latest\n' +
                'names this version. Re-run with --promote when you are ready.\n'
        )
}

await main()
