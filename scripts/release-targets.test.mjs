import { describe, expect, it } from 'vitest'

import {
    buildUpdaterManifest,
    bundleType,
    classify,
    isPrerelease,
} from './release-targets.mjs'

const BASE = 'https://cdn.example.com/downloads/gamespice-app/0.3.0'

/** What `release.yml`'s Collect step gathers from a signed build. */
const ARTIFACTS = [
    'TMC_0.3.0_universal.app.tar.gz',
    'TMC_0.3.0_amd64.AppImage',
    'TMC_0.3.0_x64-setup.exe',
    'TMC_0.3.0_x64_en-US.msi',
].map((name) => ({ name, signature: `sig-of-${name}\n` }))

describe('the updater manifest', () => {
    const manifest = buildUpdaterManifest({
        version: '0.3.0',
        baseUrl: `${BASE}/`,
        artifacts: [
            ...ARTIFACTS,
            // Never an update: a disk image, a portable exe, a deb with no sig.
            { name: 'TMC_0.3.0_universal.dmg', signature: 'x' },
            { name: 'TMC_0.3.0_x64-portable.exe', signature: 'x' },
        ],
        pubDate: '2026-09-27T00:00:00.000Z',
    })

    it('covers every desktop platform, both generic and per-bundle', () => {
        expect(Object.keys(manifest.platforms)).toEqual([
            'darwin-aarch64',
            'darwin-aarch64-app',
            'darwin-x86_64',
            'darwin-x86_64-app',
            'linux-x86_64',
            'linux-x86_64-appimage',
            'windows-x86_64',
            'windows-x86_64-msi',
            'windows-x86_64-nsis',
        ])
    })

    it('hands a Windows machine of unknown bundle the setup exe, and an MSI install the MSI', () => {
        expect(manifest.platforms['windows-x86_64'].url).toBe(
            `${BASE}/TMC_0.3.0_x64-setup.exe`
        )
        expect(manifest.platforms['windows-x86_64-msi'].url).toBe(
            `${BASE}/TMC_0.3.0_x64_en-US.msi`
        )
    })

    it('carries the signature text, trimmed, and an encoded URL', () => {
        expect(manifest.platforms['linux-x86_64'].signature).toBe(
            'sig-of-TMC_0.3.0_amd64.AppImage'
        )
        expect(manifest.version).toBe('0.3.0')
        expect(manifest.pub_date).toBe('2026-09-27T00:00:00.000Z')
    })

    it('refuses plaintext hosting and a release with nothing signed', () => {
        expect(() =>
            buildUpdaterManifest({
                version: '1.0.0',
                baseUrl: 'http://x',
                artifacts: ARTIFACTS,
            })
        ).toThrow(/https/)
        expect(() =>
            buildUpdaterManifest({ version: '1.0.0', baseUrl: BASE, artifacts: [] })
        ).toThrow(/No signed artifact/)
    })
})

describe('classification', () => {
    it('matches what the site publish has always sent', () => {
        expect(classify('TMC_0.3.0_x64-setup.exe')?.target).toBe('WINDOWS_X86_64')
        expect(classify('TMC_0.3.0_universal.app.tar.gz')?.target).toBe(
            'DARWIN_UNIVERSAL'
        )
        expect(classify('TMC_0.3.0_x64-portable.exe')).toBeNull()
        expect(bundleType('TMC_0.3.0_universal.dmg')).toBeNull()
    })

    it('tells a pre-release from a release', () => {
        expect(isPrerelease('1.2.0-beta.1')).toBe(true)
        expect(isPrerelease('1.2.0')).toBe(false)
    })
})
