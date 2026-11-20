import { describe, expect, it } from 'vitest'

import {
    WorkshopCatalogResponse,
    WorkshopItemPatchRequest,
    WorkshopMineItemSchema,
    WorkshopOwnedItemSchema,
} from '~/lib/api/contract'
import { ownedStatus, packSize, toggleId, unavailableLabel } from './workshop'

const owned = (over: object) =>
    WorkshopOwnedItemSchema.parse({
        id: 1,
        name: 'n',
        summary: null,
        image: null,
        webUrl: 'https://tmcdev.net/a/1',
        compat: 'listed',
        games: [101],
        version: { version: '1', runsCode: false, approved: true, kinds: ['map'] },
        listable: true,
        canList: true,
        ...over,
    })

describe('workshop helpers', () => {
    it('explains every unavailable reason', () => {
        expect(unavailableLabel({ available: false, unavailable: 'awaiting_review' })).toMatch(/review/)
        expect(unavailableLabel({ available: false, unavailable: 'missing' })).toMatch(/deleted/)
        expect(unavailableLabel({ available: true, unavailable: null })).toBeNull()
        // An older site sends no reason.
        const old = WorkshopMineItemSchema.parse({
            subscriptionId: 's',
            assetId: 1,
            name: 'n',
            image: null,
            webUrl: 'x',
            scope: 'all',
            available: false,
        })
        expect(unavailableLabel(old)).toBe('Not loadable right now')
    })

    it('says where an owned pack stands', () => {
        expect(ownedStatus(owned({}))).toBe('Offered on 1 game(s)')
        expect(ownedStatus(owned({ compat: 'any' }))).toBe('Offered on every game')
        expect(ownedStatus(owned({ compat: 'none' }))).toBe('Not offered in the Workshop')
        expect(ownedStatus(owned({ canList: false }))).toMatch(/No version provides/)
        expect(
            ownedStatus(
                owned({
                    listable: false,
                    version: { version: '1', runsCode: true, approved: false, kinds: [] },
                })
            )
        ).toBe('Waiting on code review')
        expect(ownedStatus(owned({ hidden: true }))).toBe('Hidden')
    })

    it('toggles game ids and sizes packs', () => {
        expect(toggleId([1, 2], 2)).toEqual([1])
        expect(toggleId([1], 3)).toEqual([1, 3])
        expect(packSize(512)).toBe('512 B')
        expect(packSize(2 * 1024 * 1024)).toBe('2.0 MB')
    })

    it('mirrors the site rules for an owner edit', () => {
        expect(WorkshopItemPatchRequest.safeParse({ id: 1, games: [2] }).success).toBe(false)
        expect(
            WorkshopItemPatchRequest.safeParse({ id: 1, compat: 'listed', games: [2] }).success
        ).toBe(true)
        expect(WorkshopCatalogResponse.parse({
            items: [],
            total: 0,
            page: 1,
            per: 24,
            facets: { kinds: [], games: [] },
        }).subscribed).toEqual([])
    })
})
