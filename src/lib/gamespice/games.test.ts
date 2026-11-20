import { describe, expect, it } from 'vitest'

import { canonicalSlug, gameHeadingStyle, gameLook, isPlatformSlug } from './games'

describe('platform games', () => {
    it('knows current and former urls', () => {
        expect(isPlatformSlug('playlot')).toBe(true)
        expect(isPlatformSlug('Playground')).toBe(true)
        expect(isPlatformSlug('g2gfast')).toBe(true)
        expect(isPlatformSlug('rust')).toBe(false)
        expect(isPlatformSlug(null)).toBe(false)
        // the site may store an app url with its leading slash
        expect(isPlatformSlug('/fatal-contact')).toBe(true)
        expect(isPlatformSlug('/Playlot/')).toBe(true)
    })

    it('maps a former url to the current one', () => {
        expect(canonicalSlug('arena')).toBe('fatal-contact')
        expect(canonicalSlug('/g2gfast')).toBe('airdrift')
        expect(canonicalSlug('unknown-game')).toBe('unknown-game')
    })

    it('gives known games their own look and others a stable hue', () => {
        expect(gameLook('arena').font).toBe('display')
        expect(gameLook('hungario').hue).toBe(gameLook('hungario').hue)
        expect(gameLook('hungario').hue).toBeGreaterThanOrEqual(0)
        expect(gameLook('hungario').hue).toBeLessThan(360)
    })

    it('styles a heading from the look', () => {
        const s = gameHeadingStyle(gameLook('airdrift'))

        expect(s.textTransform).toBe('uppercase')
        expect(s.color).toBe('#06b6d4')
    })
})
