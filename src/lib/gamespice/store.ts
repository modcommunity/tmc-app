import type { StoreCurrencyT, StoreFrontItemT, StoreFrontResponseT, StoreLoadoutResponseT } from '~/lib/api/contract'

/**
 * The Store screen's view logic, kept out of the components so it can be
 * tested: what an item's buy button says, which items a filter keeps, and how
 * an item maps to a loadout slot. Mirrors website-gs `src/lib/app-store.ts`
 * (`StoreView`), which is the same question asked by the web store.
 */

export type StoreFilterT = {
    q: string
    kind: string
    currency: 'all' | StoreCurrencyT
    show: 'all' | 'owned' | 'affordable' | 'sale'
}

export type StoreSortT = 'featured' | 'price-asc' | 'price-desc' | 'name' | 'newest'

export const DEFAULT_STORE_FILTER: StoreFilterT = { q: '', kind: 'all', currency: 'all', show: 'all' }

/**
 * What a member can do with one item right now.
 *
 * - `owned`/`subscribed`: already theirs.
 * - `buy`: a points (or game points) buy they can afford.
 * - `short`: a points buy they cannot afford yet.
 * - `checkout`: paid with credits, which only ever happens on the website.
 * - `soon`/`soldout`/`closed`/`signedout`: nothing to press.
 */
export type ItemStateT =
    | 'owned'
    | 'subscribed'
    | 'buy'
    | 'short'
    | 'checkout'
    | 'soon'
    | 'soldout'
    | 'closed'
    | 'signedout'

const KIND_LABELS: Record<string, string> = {
    AVATAR: 'Avatar',
    AVATAR_ACTION: 'Emote',
    AVATAR_WEARABLE: 'Wearable',
    EQUIPMENT: 'Equipment',
    COSMETIC: 'Cosmetic',
    OTHER: 'Other',
}

export function kindLabel(kind: string): string {
    return KIND_LABELS[kind] ?? kind.charAt(0) + kind.slice(1).toLowerCase().replace(/_/g, ' ')
}

/** A loadout slot key as people read it: `primary-weapon` is "Primary weapon". */
export function slotLabel(slot: string): string {
    const t = slot.replace(/[-_]+/g, ' ').trim()

    return t ? t.charAt(0).toUpperCase() + t.slice(1) : slot
}

/** The store's own name for a currency ("Points", "Credits", the game's coin). */
export function currencyLabel(front: Pick<StoreFrontResponseT, 'labels'>, currency: StoreCurrencyT): string {
    if (currency === 'CREDITS') return front.labels.credits
    if (currency === 'APP_POINTS') return front.labels.appPoints

    return front.labels.points
}

/** The member's balance in a currency, or null when signed out or the pool is off. */
export function balanceIn(front: Pick<StoreFrontResponseT, 'wallet'>, currency: StoreCurrencyT): number | null {
    const w = front.wallet

    if (!w) return null
    if (currency === 'CREDITS') return w.credits
    if (currency === 'APP_POINTS') return w.appPoints

    return w.points
}

export function itemState(item: StoreFrontItemT, front: StoreFrontResponseT): ItemStateT {
    if (front.subscribed.some((s) => s.itemId === item.id)) return 'subscribed'
    if (front.owned.includes(item.id)) return 'owned'
    if (!front.availability.open) return 'closed'
    if (item.window === 'soon') return 'soon'
    if (item.stockLeft !== null && item.stockLeft <= 0) return 'soldout'
    if (item.currency === 'CREDITS') return 'checkout'
    if (!front.wallet) return 'signedout'

    const balance = balanceIn(front, item.currency)

    return balance !== null && balance >= item.price ? 'buy' : 'short'
}

/** Percent off while a sale runs, rounded; 0 when not on sale. */
export function saleOff(item: Pick<StoreFrontItemT, 'onSale' | 'price' | 'listPrice'>): number {
    if (!item.onSale || item.listPrice <= 0 || item.price >= item.listPrice) return 0

    return Math.round((1 - item.price / item.listPrice) * 100)
}

/** The kinds a store lists, in a stable order, for the kind filter. */
export function kindsOf(items: readonly StoreFrontItemT[]): string[] {
    const order = Object.keys(KIND_LABELS)

    return [...new Set(items.map((i) => i.kind))].sort(
        (a, b) => (order.indexOf(a) + 1 || 99) - (order.indexOf(b) + 1 || 99) || a.localeCompare(b)
    )
}

export function filterItems(
    items: readonly StoreFrontItemT[],
    front: StoreFrontResponseT,
    filter: StoreFilterT,
    sort: StoreSortT
): StoreFrontItemT[] {
    const needle = filter.q.trim().toLowerCase()
    const kept = items.filter((item) => {
        if (filter.kind !== 'all' && item.kind !== filter.kind) return false
        if (filter.currency !== 'all' && item.currency !== filter.currency) return false

        if (needle) {
            const hay = `${item.name} ${item.description} ${item.category ?? ''} ${kindLabel(item.kind)}`.toLowerCase()

            if (!hay.includes(needle)) return false
        }

        const state = itemState(item, front)

        if (filter.show === 'owned') return state === 'owned' || state === 'subscribed'
        if (filter.show === 'affordable') return state === 'buy'
        if (filter.show === 'sale') return item.onSale

        return true
    })

    const byName = (a: StoreFrontItemT, b: StoreFrontItemT) => a.name.localeCompare(b.name) || a.id - b.id

    switch (sort) {
        case 'price-asc':
            return kept.sort((a, b) => a.price - b.price || byName(a, b))
        case 'price-desc':
            return kept.sort((a, b) => b.price - a.price || byName(a, b))
        case 'name':
            return kept.sort(byName)
        case 'newest':
            return kept.sort((a, b) => b.id - a.id)
        default:
            // The store's own order, with sales first: what the owner featured, then what is cheap today.
            return kept.sort((a, b) => Number(b.onSale) - Number(a.onSale))
    }
}

/**
 * A fresh idempotency key for one confirmed buy (8-64 of `A-Za-z0-9_-`). Made
 * when the member confirms, reused if that same request is retried.
 */
export function newIdemKey(random: () => number = Math.random): string {
    const abc = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789'
    let out = 'app-'

    for (let i = 0; i < 24; i++) out += abc[Math.floor(random() * abc.length) % abc.length]

    return out
}

/**
 * The loadout slot an item goes in: its category when it has one, else its
 * kind, normalised to the server's slot shape (`a-z 0-9 _ . -`, max 64).
 * Null for wearables, which are worn on the avatar instead (store-review-11).
 */
export function slotFor(item: Pick<StoreFrontItemT, 'kind' | 'category'>): string | null {
    if (item.kind === 'AVATAR_WEARABLE') return null

    const raw = (item.category ?? item.kind).toLowerCase().replace(/[^a-z0-9_.-]+/g, '-').replace(/^[^a-z0-9]+/, '')

    return raw.slice(0, 64) || 'other'
}

/** Whether `key` from the game's own store is equipped, and in which slot. */
export function equippedSlot(loadout: StoreLoadoutResponseT | undefined, appId: number, key: string): string | null {
    const store = `app:${appId}`

    return loadout?.loadout.find((l) => l.key === key && l.store === store)?.slot ?? null
}

/** `1234` as `1,234`; the store's prices and balances are whole numbers. */
export function amount(n: number): string {
    return n.toLocaleString('en-US')
}
