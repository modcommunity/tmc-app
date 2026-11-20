#!/usr/bin/env node
/**
 * Makes the generated Android and iOS projects declare every deep-link scheme.
 *
 * `src-tauri/gen/` is NOT committed: release.yml runs `tauri android init` /
 * `tauri ios init` and then this script. The schemes come from
 * `plugins.deep-link.mobile` in tauri.conf.json (the same list the desktop
 * build registers), so there is one place to add one.
 *
 *   node scripts/mobile-patch.mjs android <AndroidManifest.xml>   add what is missing
 *   node scripts/mobile-patch.mjs ios <Info.plist>                add what is missing
 *   node scripts/mobile-patch.mjs check-android <AndroidManifest.xml>   exit 1 unless all present
 *   node scripts/mobile-patch.mjs check-ios <Info.plist>                exit 1 unless all present
 *
 * The deep-link plugin may also inject its own filters at build time; a
 * duplicate intent filter is legal, a missing one is a dead `gs://` link.
 */
import { readFileSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

export function schemesFromConf(conf) {
    const entries = conf?.plugins?.['deep-link']?.mobile ?? []
    return [...new Set(entries.flatMap((e) => (Array.isArray(e.scheme) ? e.scheme : [e.scheme])))]
}

const MARK = 'GameSpice deep links (scripts/mobile-patch.mjs)'

export function missingAndroid(xml, schemes) {
    return schemes.filter((s) => !new RegExp(`android:scheme="${s}"`).test(xml))
}

export function patchAndroid(xml, schemes) {
    const missing = missingAndroid(xml, schemes)
    if (missing.length === 0) return xml
    const data = missing.map((s) => `                <data android:scheme="${s}" />`).join('\n')
    const filter = `            <!-- ${MARK} -->
            <intent-filter>
                <action android:name="android.intent.action.VIEW" />
                <category android:name="android.intent.category.DEFAULT" />
                <category android:name="android.intent.category.BROWSABLE" />
${data}
            </intent-filter>
`
    // The launcher activity is the first <activity>; the filter goes at the
    // end of it.
    const m = /<activity\b[\s\S]*?<\/activity>/.exec(xml)
    if (!m) throw new Error('no <activity> in the manifest')
    const at = m.index + m[0].lastIndexOf('</activity>')
    return xml.slice(0, at) + filter + '        ' + xml.slice(at)
}

export function missingPlist(plist, schemes) {
    return schemes.filter((s) => !plist.includes(`<string>${s}</string>`))
}

export function patchPlist(plist, schemes) {
    const missing = missingPlist(plist, schemes)
    if (missing.length === 0) return plist
    const strings = missing.map((s) => `\t\t\t\t<string>${s}</string>\n`).join('')
    if (plist.includes('CFBundleURLTypes')) {
        // Add to the first existing CFBundleURLSchemes array.
        const re = /(<key>CFBundleURLSchemes<\/key>\s*<array>\s*)/
        if (!re.test(plist)) throw new Error('CFBundleURLTypes has no CFBundleURLSchemes')
        return plist.replace(re, `$1\n${strings}`)
    }
    const block = `\t<key>CFBundleURLTypes</key>
\t<array>
\t\t<dict>
\t\t\t<key>CFBundleURLName</key>
\t\t\t<string>com.moddingcommunity.app</string>
\t\t\t<key>CFBundleURLSchemes</key>
\t\t\t<array>
${strings}\t\t\t</array>
\t\t</dict>
\t</array>
`
    const at = plist.lastIndexOf('</dict>')
    if (at < 0) throw new Error('not a property list')
    return plist.slice(0, at) + block + plist.slice(at)
}

function main(argv) {
    const [cmd, file] = argv
    const conf = JSON.parse(
        readFileSync(fileURLToPath(new URL('../src-tauri/tauri.conf.json', import.meta.url)), 'utf8'),
    )
    const schemes = schemesFromConf(conf)
    if (schemes.length === 0) throw new Error('tauri.conf.json declares no mobile deep-link schemes')
    const text = readFileSync(file, 'utf8')
    switch (cmd) {
        case 'android':
            writeFileSync(file, patchAndroid(text, schemes))
            return
        case 'ios':
            writeFileSync(file, patchPlist(text, schemes))
            return
        case 'check-android':
        case 'check-ios': {
            const missing = (cmd === 'check-android' ? missingAndroid : missingPlist)(text, schemes)
            if (missing.length) {
                console.error(`${file} is missing the scheme(s): ${missing.join(', ')}`)
                process.exit(1)
            }
            console.log(`${file} declares: ${schemes.join(', ')}`)
            return
        }
        default:
            throw new Error('usage: mobile-patch.mjs android|ios|check-android|check-ios <file>')
    }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) main(process.argv.slice(2))
