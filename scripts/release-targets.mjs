/**
 * Which bundle file serves which platform, for BOTH ways the app learns about
 * a release:
 *
 *   * `publish-release.mjs` posts one artifact per `AppUpdateTarget` to the
 *     site (`POST /api/app/v1/releases`), which answers the app's
 *     `/update/:target/:arch/:current` route;
 *   * `updater-manifest.mjs` writes the static `latest.json` that the release
 *     workflow uploads to the downloads bucket, which is the updater's default
 *     endpoint and needs nothing on the site.
 *
 * One table, so the two cannot disagree about which file a Windows machine is
 * handed. Pure functions only; `release-targets.test.mjs` pins them.
 */

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
export const RULES = [
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

export function classify(name) {
    return RULES.find((rule) => rule.test(name)) ?? null
}

/**
 * `AppUpdateTarget` → the platform keys the Tauri updater looks up in a static
 * manifest. It tries `{os}-{arch}-{bundle}` first, then `{os}-{arch}`, so the
 * generic key is written for the PREFERRED artifact only (rank 0) and the
 * bundle-specific key for each — an MSI install then updates from the MSI and
 * an NSIS install from the setup exe, and anything unknown gets the setup exe.
 *
 * macOS ships one universal `.app.tar.gz`, so both architectures point at it.
 */
const TAURI_PLATFORMS = {
    DARWIN_UNIVERSAL: ['darwin-x86_64', 'darwin-aarch64'],
    LINUX_X86_64: ['linux-x86_64'],
    LINUX_AARCH64: ['linux-aarch64'],
    WINDOWS_X86_64: ['windows-x86_64'],
    WINDOWS_AARCH64: ['windows-aarch64'],
}

/** The bundle-type suffix Tauri's updater uses for a file, or null. */
export function bundleType(name) {
    if (name.endsWith('.app.tar.gz')) return 'app'
    if (name.endsWith('.AppImage')) return 'appimage'
    if (name.endsWith('-setup.exe')) return 'nsis'
    if (name.endsWith('.msi')) return 'msi'
    return null
}

/**
 * Build the static updater manifest (`latest.json`).
 *
 * @param {object} input
 * @param {string} input.version   Without the leading `v`.
 * @param {string} input.baseUrl   Where the artifacts are served, https, no trailing slash.
 * @param {{ name: string, signature: string }[]} input.artifacts  Signed files only.
 * @param {string} [input.notes]
 * @param {string} [input.pubDate] RFC 3339; defaults to now.
 */
export function buildUpdaterManifest({
    version,
    baseUrl,
    artifacts,
    notes,
    pubDate,
}) {
    if (!version) throw new Error('A manifest needs a version.')
    if (!/^https:\/\//.test(baseUrl ?? ''))
        throw new Error(
            'The artifacts must be served over https; the app refuses anything else.'
        )

    const base = baseUrl.replace(/\/+$/, '')
    const platforms = {}
    const generic = new Map()

    for (const { name, signature } of artifacts) {
        const rule = classify(name)
        const bundle = bundleType(name)

        if (!rule || !bundle || !signature) continue

        const entry = {
            signature: signature.trim(),
            url: `${base}/${encodeURIComponent(name)}`,
        }

        for (const key of TAURI_PLATFORMS[rule.target] ?? []) {
            platforms[`${key}-${bundle}`] = entry

            const held = generic.get(key)
            if (!held || rule.rank < held.rank)
                generic.set(key, { rank: rule.rank, entry })
        }
    }

    for (const [key, { entry }] of generic) platforms[key] = entry

    if (Object.keys(platforms).length === 0)
        throw new Error(
            'No signed artifact maps to an updater platform; nothing to publish.'
        )

    return {
        version,
        notes: notes || `GameSpice App ${version}`,
        pub_date: pubDate ?? new Date().toISOString(),
        platforms: Object.fromEntries(
            Object.entries(platforms).sort(([a], [b]) => a.localeCompare(b))
        ),
    }
}

/**
 * Whether a version is a pre-release (`1.2.0-beta.1`). A pre-release goes only
 * to the beta manifest; a stable release goes to both, so beta never trails.
 */
export function isPrerelease(version) {
    return /^\d+\.\d+\.\d+-/.test(version)
}
