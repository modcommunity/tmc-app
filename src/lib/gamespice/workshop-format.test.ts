import { describe, expect, it } from 'vitest'

import {
    compactCount,
    filledStars,
    formatRating,
    hueOf,
    initials,
    kindGradient,
    relativeDate,
} from './workshop-format'

describe('workshop-format', () => {
    it('compacts counts', () => {
        expect(compactCount(0)).toBe('0')
        expect(compactCount(999)).toBe('999')
        expect(compactCount(1000)).toBe('1K')
        expect(compactCount(1234)).toBe('1.2K')
        expect(compactCount(15_400)).toBe('15K')
        expect(compactCount(2_500_000)).toBe('2.5M')
        expect(compactCount(-3)).toBe('0')
    })
    it('formats ratings and stars', () => {
        expect(formatRating(4.26)).toBe('4.3')
        expect(formatRating(null)).toBeNull()
        expect(formatRating(0)).toBeNull()
        expect(filledStars(4.6)).toBe(5)
        expect(filledStars(undefined)).toBe(0)
        expect(filledStars(9)).toBe(5)
    })
    it('makes initials', () => {
        expect(initials('WhoZ')).toBe('WH')
        expect(initials('Ada Lovelace')).toBe('AL')
        expect(initials(null)).toBe('?')
    })
    it('colours kinds stably', () => {
        expect(hueOf('map')).toBe(hueOf('map'))
        expect(hueOf('map')).toBeLessThan(360)
        expect(kindGradient('map')).toContain('linear-gradient')
    })
    it('words dates', () => {
        const now = Date.parse('2026-11-21T00:00:00Z')
        expect(relativeDate('2026-11-21T00:00:00Z', now)).toBe('today')
        expect(relativeDate('2026-11-18T00:00:00Z', now)).toBe('3d ago')
        expect(relativeDate('2026-08-01T00:00:00Z', now)).toBe('3mo ago')
        expect(relativeDate('2024-01-01T00:00:00Z', now)).toBe('2y ago')
        expect(relativeDate('nope', now)).toBe('')
    })
})
