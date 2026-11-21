import { describe, expect, it } from 'vitest'

import type { StoreFrontItemT, StoreFrontResponseT } from '~/lib/api/contract'

import {
    DEFAULT_STORE_FILTER,
    equippedSlot,
    filterItems,
    itemState,
    kindLabel,
    kindsOf,
    newIdemKey,
    saleOff,
    slotFor,
    slotLabel,
} from './store'

function item(over: Partial<StoreFrontItemT> = {}): StoreFrontItemT {
    return {
        id: 1,
        key: 'hat',
        name: 'Hat',
        description: 'A hat',
        imageUrl: null,
        images: [],
        kind: 'COSMETIC',
        refId: null,
        currency: 'POINTS',
        price: 100,
        listPrice: 100,
        onSale: false,
        saleEndsAt: null,
        billing: 'ONE_TIME',
        periodDays: null,
        category: null,
        stock: null,
        stockLeft: null,
        maxPerPlayer: 1,
        availableFrom: null,
        availableUntil: null,
        window: 'open',
        bundle: [],
        ...over,
    } as StoreFrontItemT
}

function front(over: Partial<StoreFrontResponseT> = {}): StoreFrontResponseT {
    return {
        owner: { kind: 'app', id: 7, appId: 7, name: 'Playlot' },
        storeUrl: '/dashboard/store/app/7',
        availability: {
            open: true,
            reason: null,
            points: true,
            credits: true,
            appPoints: false,
        },
        labels: { points: 'Points', credits: 'Credits', appPoints: 'Coins' },
        items: [],
        wallet: {
            points: 150,
            pointsApp: 50,
            pointsSite: 100,
            credits: 0,
            appPoints: null,
        },
        owned: [],
        held: {},
        subscribed: [],
        ...over,
    }
}

describe('itemState', () => {
    it('says buy when affordable and short when not', () => {
        expect(itemState(item({ price: 100 }), front())).toBe('buy')
        expect(itemState(item({ price: 151 }), front())).toBe('short')
    })

    it('sends credits to the website checkout, never a points buy', () => {
        expect(itemState(item({ currency: 'CREDITS' }), front())).toBe('checkout')
    })

    it('puts ownership before every other state', () => {
        const f = front({
            owned: [1],
            availability: {
                open: false,
                reason: 'x',
                points: true,
                credits: true,
                appPoints: false,
            },
        })

        expect(itemState(item(), f)).toBe('owned')
        expect(
            itemState(
                item(),
                front({
                    subscribed: [
                        {
                            itemId: 1,
                            cancelAtPeriodEnd: false,
                            currentPeriodEnd: '2026-12-01T00:00:00Z',
                        },
                    ],
                })
            )
        ).toBe('subscribed')
    })

    it('handles closed, soon, sold out and signed out', () => {
        expect(
            itemState(
                item(),
                front({
                    availability: {
                        open: false,
                        reason: null,
                        points: true,
                        credits: true,
                        appPoints: false,
                    },
                })
            )
        ).toBe('closed')
        expect(itemState(item({ window: 'soon' }), front())).toBe('soon')
        expect(itemState(item({ stockLeft: 0 }), front())).toBe('soldout')
        expect(itemState(item(), front({ wallet: null }))).toBe('signedout')
    })
})

describe('filterItems', () => {
    const items = [
        item({ id: 1, name: 'Bravo', price: 50, kind: 'COSMETIC' }),
        item({
            id: 2,
            name: 'Alpha',
            price: 500,
            kind: 'EQUIPMENT',
            onSale: true,
            listPrice: 1000,
        }),
        item({
            id: 3,
            name: 'Charlie',
            price: 10,
            kind: 'COSMETIC',
            currency: 'CREDITS',
        }),
    ]

    it('searches, filters by kind, currency and show', () => {
        const f = front({ owned: [3] })

        expect(
            filterItems(
                items,
                f,
                { ...DEFAULT_STORE_FILTER, q: 'alp' },
                'name'
            ).map((i) => i.id)
        ).toEqual([2])
        expect(
            filterItems(
                items,
                f,
                { ...DEFAULT_STORE_FILTER, kind: 'COSMETIC' },
                'name'
            ).map((i) => i.id)
        ).toEqual([1, 3])
        expect(
            filterItems(
                items,
                f,
                { ...DEFAULT_STORE_FILTER, currency: 'CREDITS' },
                'name'
            ).map((i) => i.id)
        ).toEqual([3])
        expect(
            filterItems(
                items,
                f,
                { ...DEFAULT_STORE_FILTER, show: 'owned' },
                'name'
            ).map((i) => i.id)
        ).toEqual([3])
        expect(
            filterItems(
                items,
                f,
                { ...DEFAULT_STORE_FILTER, show: 'affordable' },
                'name'
            ).map((i) => i.id)
        ).toEqual([1])
        expect(
            filterItems(
                items,
                f,
                { ...DEFAULT_STORE_FILTER, show: 'sale' },
                'name'
            ).map((i) => i.id)
        ).toEqual([2])
    })

    it('sorts', () => {
        const f = front()

        expect(
            filterItems(items, f, DEFAULT_STORE_FILTER, 'price-asc').map(
                (i) => i.id
            )
        ).toEqual([3, 1, 2])
        expect(
            filterItems(items, f, DEFAULT_STORE_FILTER, 'price-desc').map(
                (i) => i.id
            )
        ).toEqual([2, 1, 3])
        expect(
            filterItems(items, f, DEFAULT_STORE_FILTER, 'newest').map((i) => i.id)
        ).toEqual([3, 2, 1])
        expect(filterItems(items, f, DEFAULT_STORE_FILTER, 'featured')[0]?.id).toBe(
            2
        )
    })
})

describe('helpers', () => {
    it('computes sale percent', () => {
        expect(saleOff({ onSale: true, price: 75, listPrice: 100 })).toBe(25)
        expect(saleOff({ onSale: false, price: 75, listPrice: 100 })).toBe(0)
        expect(saleOff({ onSale: true, price: 100, listPrice: 100 })).toBe(0)
    })

    it('labels kinds and orders them stably', () => {
        expect(kindLabel('AVATAR_ACTION')).toBe('Emote')
        expect(kindLabel('SPRAY_PAINT')).toBe('Spray paint')
        expect(
            kindsOf([
                item({ kind: 'OTHER' }),
                item({ kind: 'AVATAR' }),
                item({ kind: 'ZZZ' }),
            ])
        ).toEqual(['AVATAR', 'OTHER', 'ZZZ'])
    })

    it('maps items to server-shaped slots, wearables to none', () => {
        expect(slotFor({ kind: 'EQUIPMENT', category: 'Primary Weapon' })).toBe(
            'primary-weapon'
        )
        expect(slotFor({ kind: 'COSMETIC', category: null })).toBe('cosmetic')
        expect(slotFor({ kind: 'AVATAR_WEARABLE', category: 'hat' })).toBeNull()
        expect(slotFor({ kind: 'OTHER', category: '!!!' })).toBe('other')
        expect(slotFor({ kind: 'OTHER', category: 'x'.repeat(100) })?.length).toBe(
            64
        )
    })

    it('labels slots for people', () => {
        expect(slotLabel('spray-paint')).toBe('Spray paint')
        expect(slotLabel('primary_weapon')).toBe('Primary weapon')
        expect(slotLabel('--')).toBe('--')
    })

    it('finds the equipped slot only for this game store', () => {
        const lo = {
            appId: 7,
            maxSlots: 8,
            loadout: [
                { slot: 'hat', store: 'app:7', key: 'hat' },
                { slot: 'x', store: 'app:8', key: 'gun' },
            ],
        }

        expect(equippedSlot(lo, 7, 'hat')).toBe('hat')
        expect(equippedSlot(lo, 7, 'gun')).toBeNull()
        expect(equippedSlot(undefined, 7, 'hat')).toBeNull()
    })

    it('makes idempotency keys the server accepts', () => {
        const k = newIdemKey()

        expect(k).toMatch(/^[A-Za-z0-9_-]{8,64}$/)
        expect(newIdemKey(() => 0)).toBe(`app-${'A'.repeat(24)}`)
        expect(newIdemKey(() => 0.99999)).toMatch(/9$/)
    })
})
