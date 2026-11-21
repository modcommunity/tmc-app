import { describe, expect, it } from 'vitest'

import { filterGames, matchesChip, mergeGames, splitSections, type CatalogueRow } from './catalogue'

const row = (id: number, name: string, over: Partial<CatalogueRow> = {}): CatalogueRow => ({
    id,
    name,
    url: null,
    solo: false,
    serverCnt: 0,
    lobbyCnt: 0,
    playerCnt: 0,
    isEngine: false,
    ...over,
})

describe('mergeGames', () => {
    it('dedupes by id and drops engines and Godot', () => {
        const out = mergeGames([
            row(1, 'A'),
            row(1, 'A again'),
            row(2, 'Godot Engine'),
            row(3, 'Thing', { isEngine: true }),
            row(4, 'B', { url: '/godot-tools' }),
            row(5, 'C'),
        ])

        expect(out.map((r) => r.id)).toEqual([1, 5])
    })
})

describe('chips', () => {
    it('separates multiplayer, singleplayer and GameSpice', () => {
        const solo = row(1, 'Solo', { solo: true })
        const both = row(2, 'Both', { solo: true, serverCnt: 2 })
        const gs = row(3, 'Prop Hunt', { url: '/prop-hunt' })

        expect(matchesChip(solo, 'singleplayer')).toBe(true)
        expect(matchesChip(solo, 'multiplayer')).toBe(false)
        expect(matchesChip(both, 'multiplayer')).toBe(true)
        expect(matchesChip(both, 'singleplayer')).toBe(true)
        expect(matchesChip(gs, 'gamespice')).toBe(true)
        expect(matchesChip(solo, 'gamespice')).toBe(false)
    })

    it('searches by name, case-insensitively', () => {
        const rows = [row(1, 'Alpha'), row(2, 'Beta')]

        expect(filterGames(rows, 'all', ' ALP ').map((r) => r.id)).toEqual([1])
    })
})

describe('splitSections', () => {
    it('puts a platform game only in the platform section, busiest first', () => {
        const { platform, others } = splitSections([
            row(1, 'Other'),
            row(2, 'Playlot', { url: 'playlot', playerCnt: 1 }),
            row(3, 'Prop Hunt', { url: '/prop-hunt', playerCnt: 9 }),
        ])

        expect(platform.map((r) => r.id)).toEqual([3, 2])
        expect(others.map((r) => r.id)).toEqual([1])
    })
})
