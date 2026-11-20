/**
 * Review fixtures for the Store and Stats screens: a signed-in member with a
 * wallet, purchases, a loadout, play days and achievements. The endpoints
 * behind them (`store/directory`, `store/purchases`, `me/stats`,
 * `stats/achievements`) are on a website-city branch and not on tmcdev.net
 * yet, so the harness answers them here when a flow sets
 * `globalThis.__reviewFixtures` (Playwright `addInitScript`). App ids are real
 * dev ids so the art comes from the proxied `/apps`.
 */

type Json = Record<string, unknown>

const DAY = 86_400_000
const today = new Date(Date.UTC(2026, 10, 20))
const iso = (d: Date) => d.toISOString().slice(0, 10)
const since = iso(new Date(today.getTime() - 29 * DAY))

/** A deterministic 30-day series: `on` marks which days were played. */
const series = (on: (i: number) => boolean) => Array.from({ length: 30 }, (_, i) => (on(i) ? 1 : 0))

const fc = series((i) => i % 3 !== 1 || i > 24)
const air = series((i) => i % 5 === 0 || (i > 18 && i < 23))
const sdtd = series((i) => i === 4 || i === 11 || i === 12)
const recent = fc.map((v, i) => Math.min(1, v + (air[i] ?? 0) + (sdtd[i] ?? 0)))
const sum = (s: number[]) => s.reduce((n, v) => n + v, 0)

const item = (over: Json): Json => ({
    description: '',
    imageUrl: null,
    images: [],
    refId: null,
    currency: 'POINTS',
    onSale: false,
    saleEndsAt: null,
    billing: 'ONE_TIME',
    periodDays: null,
    category: null,
    stock: 0,
    stockLeft: null,
    maxPerPlayer: 1,
    availableFrom: null,
    availableUntil: null,
    window: 'open',
    bundle: [],
    ...over,
})

const items: Json[] = [
    item({ id: 11, key: 'fx-bot', name: 'Kenney Bot', kind: 'AVATAR', price: 100, listPrice: 100, description: 'A boxy robot body for your player avatar, with its own idle and run cycle.' }),
    item({ id: 12, key: 'fx-dance', name: 'Victory Dance', kind: 'AVATAR_ACTION', price: 30, listPrice: 40, onSale: true, saleEndsAt: '2026-11-27T00:00:00Z', description: 'Celebrate a round win. Plays on the scoreboard and in third person.' }),
    item({ id: 13, key: 'fx-hat', name: 'Top Hat', kind: 'AVATAR_WEARABLE', category: 'hat', price: 60, listPrice: 60, description: 'A tall black hat. Fits every avatar body.' }),
    item({ id: 14, key: 'fx-vip', name: 'VIP monthly', kind: 'OTHER', currency: 'CREDITS', price: 5, listPrice: 5, billing: 'SUBSCRIPTION', periodDays: 30, description: 'Reserved slot on every official server, a name colour and a monthly crate.' }),
    item({ id: 15, key: 'smg-skin', name: 'Carbon SMG finish', kind: 'COSMETIC', category: 'Primary Weapon', price: 250, listPrice: 250, stock: 50, stockLeft: 7, description: 'Matte carbon weave for the SMG. Limited run.' }),
    item({ id: 16, key: 'spray-gg', name: 'GG spray', kind: 'SPRAY_PAINT', price: 15, listPrice: 15, description: 'Leave a "GG" on any wall.' }),
    item({ id: 17, key: 'rail', name: 'Rail rifle', kind: 'EQUIPMENT', category: 'Primary Weapon', price: 400, listPrice: 400, description: 'Charge, aim, one shot. Unlocks the rail rifle in custom loadouts.' }),
    item({ id: 18, key: 'winter', name: 'Winter crate', kind: 'OTHER', price: 120, listPrice: 120, window: 'soon', availableFrom: '2026-12-01T00:00:00Z', description: 'Seasonal skins and a snow spray. Opens on December 1.' }),
]

const purchase = (id: number, itemId: number, appId: number, daysAgo: number, over: Json = {}): Json => {
    const it = items.find((i) => i.id === itemId) ?? items[0]!
    return {
        id,
        createdAt: new Date(today.getTime() - daysAgo * DAY + 15 * 3_600_000).toISOString(),
        appId,
        serverId: null,
        currency: it.currency,
        price: it.price,
        status: 'COMPLETED',
        expiresAt: null,
        live: true,
        item: { id: it.id, key: it.key, name: it.name, imageUrl: it.imageUrl, kind: it.kind, billing: it.billing },
        ...over,
    }
}

const state = {
    points: 340,
    owned: new Set<number>([13, 16]),
    loadout: [{ slot: 'spray-paint', store: 'app:101', key: 'spray-gg' }] as { slot: string; store: string; key: string }[],
    purchases: [purchase(902, 16, 101, 2), purchase(901, 13, 101, 9)] as Json[],
}

const ach = (key: string, name: string, description: string, points: number, category: string, daysAgo: number | null) =>
    daysAgo === null
        ? { key, name: key === 'untouchable' ? null : name, description: key === 'untouchable' ? null : description, points, category, icon: '', secret: key === 'untouchable', unlockCnt: 3 }
        : { key, name, description, points, category, icon: '', unlockedAt: Math.floor((today.getTime() - daysAgo * DAY) / 1000), unlockCnt: 1 }

const achievements = [
    ach('first-blood', 'First blood', 'Get the first kill of a round.', 10, 'Combat', 26),
    ach('ace', 'Ace', 'Take out the whole other team in one round.', 50, 'Combat', 3),
    ach('marathon', 'Marathon', 'Play on seven days in a row.', 25, 'Dedication', 1),
    ach('flag-runner', 'Flag runner', 'Capture three flags in one match.', 25, 'Objectives', 12),
    ach('king', 'King of the hill', 'Hold the hill for two minutes straight.', 30, 'Objectives', null),
    ach('collector', 'Collector', 'Own ten store items.', 20, 'Store', null),
    ach('untouchable', 'Untouchable', 'Win a round without taking damage.', 75, 'Combat', null),
]

const stats = [
    { key: 'kills', name: 'Kills', kind: 'counter', unit: '', decimals: 0, value: 1284 },
    { key: 'deaths', name: 'Deaths', kind: 'counter', unit: '', decimals: 0, value: 931 },
    { key: 'wins', name: 'Rounds won', kind: 'counter', unit: '', decimals: 0, value: 212 },
    { key: 'flags', name: 'Flags captured', kind: 'counter', unit: '', decimals: 0, value: 37 },
    { key: 'accuracy', name: 'Accuracy', kind: 'gauge', unit: '%', decimals: 1, value: 31.27 },
    { key: 'playtime', name: 'Time played', kind: 'counter', unit: 'h', decimals: 1, value: 86.5 },
]

const names = ['Vexa', 'Morrow', 'Kite', 'Halden', 'Quill', 'Sable', 'Juno', 'Brask', 'Orrin', 'Tamsin']

function front(appId: number): Json | null {
    if (appId !== 101) {
        return {
            owner: { kind: 'app', id: appId, appId, name: 'Game' },
            storeUrl: `/dashboard/store/app/${appId}`,
            availability: { open: true, reason: null, points: true, credits: false, appPoints: false },
            labels: { points: 'Points', credits: 'Credits', appPoints: 'Game points' },
            items: [],
            wallet: wallet(),
            owned: [],
            held: {},
            subscribed: [],
        }
    }
    return {
        owner: { kind: 'app', id: 101, appId: 101, name: 'Fatal Contact' },
        storeUrl: '/dashboard/store/app/101',
        availability: { open: true, reason: null, points: true, credits: true, appPoints: false },
        labels: { points: 'Points', credits: 'Credits', appPoints: 'Game points' },
        items,
        wallet: wallet(),
        owned: [...state.owned],
        held: Object.fromEntries([...state.owned].map((id) => [String(id), 1])),
        subscribed: [],
    }
}

const wallet = () => ({ points: state.points, pointsApp: 0, pointsSite: state.points, credits: 12, appPoints: null })

/** Answers one API call, or `undefined` when the real backend should. */
export function fixture(method: string, path: string, q: URLSearchParams, body: Json | undefined): unknown {
    const app = Number(q.get('app') ?? q.get('appId') ?? body?.appId ?? 0)

    if (method === 'GET') {
        switch (path) {
            case '/me':
                return {
                    user: { id: '4242', name: 'Rook', username: 'rook', avatar: null, role: null },
                    settings: {
                        timezone: 'UTC',
                        locale: 'en',
                        ...Object.fromEntries(
                            ['emailNotifications', 'pushNotifications', 'notifySystem', 'notifyComments', 'notifyReviews', 'notifyReleases', 'notifyMentions', 'notifyModeration', 'notifyCredits', 'notifyFriends', 'notifyFollowers', 'notifyMessages', 'notifyStatus', 'notifyParties', 'notifyGroups'].map((k) => [k, true])
                        ),
                    },
                    serverTime: Math.floor(today.getTime() / 1000),
                }
            case '/me/points':
                return { total: state.points, site: { earned: 520, spent: 520 - state.points, available: state.points }, apps: [] }
            case '/store/directory':
                return { stores: [{ appId: 101, name: 'Fatal Contact', items: items.length }, { appId: 87, name: 'Airdrift', items: 6 }, { appId: 20, name: '7 Days to Die', items: 3 }, { appId: 5, name: "Garry's Mod", items: 0 }] }
            case '/store/front':
                return front(app)
            case '/store/loadout':
                return { appId: app, loadout: app === 101 ? state.loadout : [], maxSlots: 8 }
            case '/store/purchases':
                return { purchases: state.purchases, next: null }
            case '/me/stats':
                return {
                    since,
                    totals: { games: 3, daysPlayed: sum(fc) + sum(air) + sum(sdtd) + 41, activeDays: sum(recent), achievements: 4, achievementPoints: 110 },
                    recent,
                    games: [
                        { appId: 101, daysPlayed: sum(fc) + 33, firstPlayed: '2026-08-02', lastPlayed: iso(today), recent: fc, achievements: { unlocked: 4, points: 110, total: 7 } },
                        { appId: 87, daysPlayed: sum(air) + 8, firstPlayed: '2026-09-14', lastPlayed: iso(new Date(today.getTime() - 2 * DAY)), recent: air, achievements: { unlocked: 0, points: 0, total: 5 } },
                        { appId: 20, daysPlayed: sum(sdtd), firstPlayed: '2026-10-25', lastPlayed: '2026-11-03', recent: sdtd, achievements: null },
                    ],
                }
            case '/stats/achievements':
                return app === 101
                    ? { points: 110, unlocked: achievements.filter((a) => 'unlockedAt' in a), locked: achievements.filter((a) => !('unlockedAt' in a)) }
                    : { points: 0, unlocked: [], locked: [] }
            case '/stats/me':
                return { player: 'p-4242', name: 'Rook', stats: app === 101 ? stats : [] }
            case '/stats/defs':
                return app === 101 ? stats.map(({ value: _v, ...d }) => ({ ...d, description: null, players: 318 })) : []
            case '/stats/top': {
                const stat = q.get('stat') ?? 'kills'
                const rows = names.map((name, i) => ({ rank: i + 1, player: `p-${i}`, name, value: Math.round(4200 / (i + 1.4)) }))
                return { stat: { key: stat, name: stat[0]!.toUpperCase() + stat.slice(1), kind: 'counter', unit: '', decimals: 0, players: 318 }, rows, self: { rank: 14, player: 'p-4242', name: 'Rook', value: 1284 } }
            }
            default:
                return undefined
        }
    }
    if (method === 'POST' && path === '/store/purchase') {
        const it = items.find((i) => i.id === body?.itemId)
        if (!it) throw { code: 'api', message: 'No such item.' }
        if (it.currency !== 'POINTS') throw { code: 'api', message: 'Credits are paid on the website.' }
        state.points -= Number(it.price)
        state.owned.add(Number(it.id))
        const p = purchase(900 + state.purchases.length + 3, Number(it.id), 101, 0)
        state.purchases.unshift(p)
        return { purchase: { id: p.id, itemId: it.id, status: 'COMPLETED', price: it.price, currency: it.currency, createdAt: p.createdAt, expiresAt: null }, replay: false, wallet: wallet(), held: 1 }
    }
    if (method === 'PUT' && path === '/store/loadout') {
        const slot = String(body?.slot)
        state.loadout = state.loadout.filter((l) => l.slot !== slot)
        if (body?.key) state.loadout.push({ slot, store: String(body.store ?? `app:${app}`), key: String(body.key) })
        return { appId: app, loadout: state.loadout, maxSlots: 8 }
    }
    return undefined
}
