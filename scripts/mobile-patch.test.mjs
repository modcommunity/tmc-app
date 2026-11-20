import { expect, test } from 'vitest'
import { readFileSync } from 'node:fs'
import { missingAndroid, missingPlist, patchAndroid, patchPlist, schemesFromConf } from './mobile-patch.mjs'

const conf = JSON.parse(readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'))
const schemes = schemesFromConf(conf)

const MANIFEST = `<manifest xmlns:android="http://schemas.android.com/apk/res/android">
    <application>
        <activity android:name=".MainActivity">
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
            </intent-filter>
        </activity>
    </application>
</manifest>
`
const PLIST = `<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
\t<key>CFBundleName</key>
\t<string>x</string>
</dict>
</plist>
`

test('the schemes come from tauri.conf.json', () => {
    expect(schemes).toEqual(['gs', 'tmc'])
})

test('android: both schemes added once, and idempotently', () => {
    const out = patchAndroid(MANIFEST, schemes)
    expect(missingAndroid(out, schemes)).toEqual([])
    expect(patchAndroid(out, schemes)).toBe(out)
    expect(out.indexOf('android:scheme="gs"')).toBeLessThan(out.indexOf('</activity>'))
})

test('android: only the missing scheme is added', () => {
    const half = patchAndroid(MANIFEST, ['gs'])
    const out = patchAndroid(half, schemes)
    expect(out.match(/android:scheme="gs"/g)).toHaveLength(1)
    expect(missingAndroid(out, schemes)).toEqual([])
})

test('ios: URL types added, idempotent, and extended when present', () => {
    const out = patchPlist(PLIST, schemes)
    expect(missingPlist(out, schemes)).toEqual([])
    expect(patchPlist(out, schemes)).toBe(out)
    const ext = patchPlist(patchPlist(PLIST, ['gs']), schemes)
    expect(missingPlist(ext, schemes)).toEqual([])
    expect(ext.match(/CFBundleURLTypes/g)).toHaveLength(1)
})
