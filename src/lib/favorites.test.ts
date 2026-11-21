import { describe, expect, it } from 'vitest'

import type { ContentSummaryT } from '~/lib/api/contract'
import {
    MAX_FAVORITES,
    filterFavorites,
    parseFavorites,
    refreshFavorites,
    toggleFavorite,
} from './favorites'

function srv(
    id: number,
    over: Partial<ContentSummaryT> = {},
    s: object = {}
): ContentSummaryT {
    return {
        id,
        name: `Server ${id}`,
        isOfficial: false,
        app: { id: 1, name: 'G' },
        server: { online: true, curUsers: 5, maxUsers: 10, ...s },
        ...over,
    } as unknown as ContentSummaryT
}

describe('toggleFavorite', () => {
    it('adds then removes without mutating', () => {
        const a = toggleFavorite({}, srv(1))

        expect(Object.keys(a)).toEqual(['1'])

        const b = toggleFavorite(a, srv(1))

        expect(b).toEqual({})
        expect(Object.keys(a)).toEqual(['1'])
    })

    it('caps at MAX_FAVORITES dropping the oldest', () => {
        let map = {}

        for (let i = 1; i <= MAX_FAVORITES + 2; i++)
            map = toggleFavorite(map, srv(i))

        expect(Object.keys(map)).toHaveLength(MAX_FAVORITES)
        expect('1' in map).toBe(false)
    })
})

describe('refreshFavorites', () => {
    it('updates only existing ids', () => {
        const map = toggleFavorite({}, srv(1))
        const next = refreshFavorites(map, [srv(1, { name: 'New' }), srv(2)])

        expect(next['1']?.name).toBe('New')
        expect('2' in next).toBe(false)
    })
})

describe('filterFavorites', () => {
    const list = [
        srv(1, { name: 'Alpha' }, { curUsers: 0 }),
        srv(2, { name: 'Beta' }, { curUsers: 10, maxUsers: 10 }),
        srv(3, { name: 'Gamma', app: { id: 2, name: 'H' } } as never, {
            online: false,
        }),
    ]

    it('searches by name', () => {
        expect(filterFavorites(list, { search: 'alp' }).map((x) => x.id)).toEqual([
            1,
        ])
    })

    it('hides empty, full and offline', () => {
        expect(filterFavorites(list, { hideEmpty: true }).map((x) => x.id)).toEqual(
            [2, 3]
        )
        expect(filterFavorites(list, { hideFull: true }).map((x) => x.id)).toEqual([
            1, 3,
        ])
        expect(
            filterFavorites(list, { onlineOnly: true }).map((x) => x.id)
        ).toEqual([1, 2])
    })

    it('filters by game', () => {
        expect(filterFavorites(list, { appId: 2 }).map((x) => x.id)).toEqual([3])
    })
})

describe('parseFavorites', () => {
    it('survives garbage', () => {
        expect(parseFavorites('nope')).toEqual({})
        expect(parseFavorites('[1]')).toEqual({})
        expect(parseFavorites(null)).toEqual({})
        expect(Object.keys(parseFavorites('{"1":{},"x":{}}'))).toEqual(['1'])
    })
})
