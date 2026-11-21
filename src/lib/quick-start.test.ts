import { describe, expect, it } from 'vitest'

import { quickAction } from './quick-start'

describe('quickAction', () => {
    it('launches native when installed, even if web and install exist', () => {
        expect(
            quickAction({ canNative: true, canInstall: false, canWeb: true })
        ).toBe('native')
    })

    it('offers the chooser (install then run) when native is missing but installable', () => {
        expect(
            quickAction({ canNative: false, canInstall: true, canWeb: true })
        ).toBe('chooser')
    })

    it('falls back to web only where native is unavailable', () => {
        expect(
            quickAction({ canNative: false, canInstall: false, canWeb: true })
        ).toBe('web')
        expect(
            quickAction({ canNative: false, canInstall: false, canWeb: false })
        ).toBe('dialog')
    })
})
