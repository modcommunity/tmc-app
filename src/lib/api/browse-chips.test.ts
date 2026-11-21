import { describe, expect, it } from 'vitest'

import { activeChips, removeChip } from './browse-chips'

const p = (s: string) => new URLSearchParams(s)

describe('activeChips', () => {
    it('is empty for a fresh browse', () => {
        expect(activeChips(p(''))).toEqual([])
    })

    it('names filters, resolving game and country names', () => {
        const chips = activeChips(
            p('q=x&app=3&countries=5,6&password=1&minUsers=2&online=0&ages=ADULT_18'),
            { apps: new Map([['3', 'Quake']]), countries: new Map([['5', 'France']]) }
        )

        expect(chips.map((c) => c.label)).toEqual([
            '"x"',
            'Quake',
            'France',
            'Country 6',
            'Password',
            'Including offline',
            '18+',
            'Players: 2+',
        ])
    })

    it('inverts the online chip for favourites', () => {
        expect(activeChips(p('online=0'), {}, { favorites: true })).toEqual([])
        expect(activeChips(p('online=1'), {}, { favorites: true })).toHaveLength(1)
    })
})

describe('removeChip', () => {
    it('removes one value from a list', () => {
        const next = removeChip(p('countries=5,6'), { keys: ['countries:5'], label: '' })

        expect(next.get('countries')).toBe('6')
    })

    it('deletes the key when the list empties and clears ranges', () => {
        expect(removeChip(p('countries=5'), { keys: ['countries:5'], label: '' }).has('countries')).toBe(false)

        const next = removeChip(p('minUsers=1&maxUsers=9&sort=name'), {
            keys: ['minUsers', 'maxUsers'],
            label: '',
        })

        expect(next.toString()).toBe('sort=name')
    })
})
