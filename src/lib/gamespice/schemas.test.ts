import { describe, expect, it } from 'vitest'

import { RAIL } from '../nav'
import { AvatarOutfitsResponse, PointsSummaryResponse, WorkshopMineResponse } from '~/lib/api/contract'

describe('gamespice schemas', () => {
    it('parses a workshop subscription list', () => {
        const r = WorkshopMineResponse.parse({
            items: [
                { subscriptionId: 's1', assetId: 4, name: 'Pack', image: null, webUrl: '/a/4', scope: 'all' },
            ],
        })

        expect(r.items[0]?.scope).toBe('all')
    })

    it('rejects an unknown scope', () => {
        expect(() =>
            WorkshopMineResponse.parse({
                items: [{ subscriptionId: 's', assetId: 1, name: 'x', image: null, webUrl: '/', scope: 'some' }],
            })
        ).toThrow()
    })

    it('parses points and outfits', () => {
        const p = PointsSummaryResponse.parse({
            site: { earned: 5, spent: 1, available: 4 },
            apps: [{ appId: 2, trusted: true, earned: 1, spent: 0, available: 1 }],
            total: 5,
        })

        expect(p.site.available).toBe(4)
        expect(AvatarOutfitsResponse.parse({ outfits: [{ id: 'a', name: 'Red', savedAt: '2026-11-20', doc: { version: 1, schema_id: 's', parts: {}, colours: {} } }] }).outfits).toHaveLength(1)
    })
})

describe('nav', () => {
    it('has the GameSpice routes and the avatar slot in the rail', () => {
        const to = RAIL.map((n) => n.to)

        expect(to).toEqual(expect.arrayContaining(['/games', '/avatar', '/me']))
    })
})
