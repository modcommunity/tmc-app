/**
 * Demo data for the screenshots. Wraps the review mock's bridge: a seeded
 * command answers with the smallest valid value for its schema (fake) merged
 * with the rows below, so a schema change adds fields instead of breaking the
 * shots. Everything here is invented: no real players, paths or accounts.
 */
import { fake } from '../../e2e/review/mock'

type Def = { type: string; element?: unknown; innerType?: unknown }
type Sch = { _zod: { def: Def } }

const g = globalThis as unknown as {
    __TAURI_INTERNALS__: { invoke: (cmd: string, args?: unknown, opts?: unknown) => Promise<unknown> }
    __reviewSchema?: unknown
}

const NOW = Date.UTC(2026, 10, 20, 18, 0, 0)
const iso = (minsAgo: number) => new Date(NOW - minsAgo * 60_000).toISOString()

/** The element schema of an array schema, through optional/default wrappers. */
function element(schema: unknown): unknown {
    let s = schema as Sch
    while (s?._zod?.def && s._zod.def.type !== 'array' && s._zod.def.innerType) s = s._zod.def.innerType as Sch
    return s?._zod?.def?.element
}

const rows = (schema: unknown, list: Record<string, unknown>[]) =>
    list.map((r) => ({ ...(fake(element(schema)) as object), ...r }))
const one = (schema: unknown, r: Record<string, unknown>) => ({ ...(fake(schema) as object), ...r })

/** A smooth, slightly noisy latency trace: deterministic, no Math.random. */
function trace(base: number, n = 40) {
    return Array.from({ length: n }, (_, i) => ({
        at: NOW - (n - i) * 15_000,
        rttMs: Math.round(base + 4 * Math.sin(i / 3) + ((i * 7919) % 5)),
    }))
}

/** Per-server demo state, keyed by position in the batch so any API list works. */
const SERVERS = [
    { name: 'Ember Ridge | Casual 24/7', map: 'cs_ashfall', players: 18, max: 24, ping: 14 },
    { name: 'Kiln Arena Duels', map: 'de_kiln', players: 9, max: 12, ping: 22 },
    { name: 'Sparkline Community', map: 'fc_harbor', players: 27, max: 32, ping: 31 },
    { name: 'Northwind Practice', map: 'aim_lanterns', players: 4, max: 16, ping: 48 },
    { name: 'Cinder Bay Retakes', map: 'de_cinder', players: 10, max: 10, ping: 63 },
    { name: 'Saffron Coast Surf', map: 'surf_saffron', players: 21, max: 40, ping: 87 },
]
const slug = (n: string) => n.split(' ')[0].toLowerCase() + '-' + n.split(' ')[1].toLowerCase().replace(/[^a-z]/g, '')
const hostOf = (i: number) => `${slug(SERVERS[i].name)}.example.net`
/** By host when the batch names one of ours, else by position. */
const server = (i: number, host?: string) => {
    const k = SERVERS.findIndex((_, j) => hostOf(j) === host)
    return SERVERS[k >= 0 ? k : i % SERVERS.length]
}

function queryResult(schema: unknown, i: number, host?: string) {
    const s = server(i, host)
    return one(schema, {
        online: true, rttMs: s.ping, name: null, map: s.map,
        players: s.players, maxPlayers: s.max, bots: 0, password: false, secure: true,
        playerList: Array.from({ length: Math.min(s.players, 8) }, (_, k) => ({
            ...(fake(element((schema as { _zod: { def: { shape: Record<string, unknown> } } })._zod.def.shape.playerList)) as object),
            name: ['NovaRift', 'Pixel Marrow', 'KiteRunner', 'Saffron Fox', 'Lumen Wolf', 'Ember', 'Quill', 'Basalt'][k],
            score: 40 - k * 4, duration: 1800 - k * 120, ping: 12 + k * 5,
        })),
    })
}

type Req = { id?: string; key?: string; port?: number; host?: string }

const seeds: Record<string, (schema: unknown, a: Record<string, unknown>) => unknown> = {
    query_servers: (schema, a) => ((a.requests as Req[]) ?? []).map((r, i) => {
        const el = element(schema) as { _zod: { def: { shape: Record<string, unknown> } } }
        const res = queryResult((el._zod.def.shape.result as Sch)._zod.def.innerType, i, r.host) as Record<string, unknown>
        return { id: r.id ?? String(i), key: r.key ?? String(i), result: { ...res, queriedPort: r.port ?? 27015 } }
    }),
    // The expanded row asks again on its own (with the roster): unseeded, it read "TO".
    query_server: (schema, a) => {
        const r = (a.request ?? {}) as Req
        const res = queryResult(((schema as { _zod: { def: { shape: Record<string, unknown> } } })._zod.def.shape.result as Sch)._zod.def.innerType, 0, r.host) as Record<string, unknown>
        return { id: r.id ?? '0', key: r.key ?? '0', result: { ...res, queriedPort: r.port ?? 27015 } }
    },
    latency_series: (schema, a) => ((a.keys as string[]) ?? []).map((key, i) => {
        const p = server(i, SERVERS.map((_, j) => hostOf(j)).find((h) => key.includes(h))).ping
        return { ...(fake(element(schema)) as object), key, samples: trace(p), last: p, min: p - 3, max: p + 6, avg: p + 1, jitter: 2.1, reliability: 100 }
    }),
    ping_server: (_s, a) => ({ host: String(a.host ?? ''), port: Number(a.port ?? 0), rttMs: 18, received: 3, attempts: 3 }),
    download_list: (schema) => {
        const d = (schema as { _zod: { def: { shape: { downloads: unknown } } } })._zod.def.shape.downloads
        const MB = 1024 * 1024
        return {
            active: 2, speedBps: 38 * MB, limitBps: null,
            downloads: rows(d, [
                { id: 'd1', label: 'Fatal Contact 1.4.0 (Linux)', status: 'running', total: 1840 * MB, done: 1214 * MB, speedBps: 26 * MB, etaSecs: 24, samples: [18, 21, 24, 23, 26, 27, 25, 26].map((v) => v * MB), queuedAt: iso(3), updatedAt: iso(0), url: 'https://example.invalid/fc.zip', dest: 'Downloads/fc.zip' },
                { id: 'd2', label: 'Harbor Lights map pack 2.1', status: 'running', total: 412 * MB, done: 96 * MB, speedBps: 12 * MB, etaSecs: 26, samples: [8, 10, 11, 12, 12, 13, 12, 12].map((v) => v * MB), queuedAt: iso(2), updatedAt: iso(0), url: 'https://example.invalid/a.zip', dest: 'Downloads/a.zip' },
                { id: 'd3', label: 'Ember HUD theme 0.9', status: 'queued', total: 18 * MB, done: 0, speedBps: 0, etaSecs: null, queuedAt: iso(1), updatedAt: iso(1), url: 'https://example.invalid/b.zip', dest: 'Downloads/b.zip' },
                { id: 'd4', label: 'Kiln Arena weapon sounds', status: 'done', total: 64 * MB, done: 64 * MB, speedBps: 0, etaSecs: 0, queuedAt: iso(40), updatedAt: iso(38), url: 'https://example.invalid/c.zip', dest: 'Downloads/c.zip' },
                { id: 'd5', label: 'Saffron Coast skybox', status: 'done', total: 128 * MB, done: 128 * MB, speedBps: 0, etaSecs: 0, queuedAt: iso(90), updatedAt: iso(86), url: 'https://example.invalid/d.zip', dest: 'Downloads/d.zip' },
            ]),
        }
    },
    playtime_summary: () => ({
        byApp: { 100: { seconds: 41 * 3600 + 1200, launches: 63, lastPlayedMs: NOW - 2 * 3600_000 }, 101: { seconds: 27 * 3600, launches: 38, lastPlayedMs: NOW - 26 * 3600_000 } },
        bySandbox: { 1: { seconds: 30 * 3600, launches: 41, lastPlayedMs: NOW - 2 * 3600_000 }, 2: { seconds: 11 * 3600, launches: 22, lastPlayedMs: NOW - 5 * 86400_000 }, 3: { seconds: 27 * 3600, launches: 38, lastPlayedMs: NOW - 26 * 3600_000 } },
    }),
    sandbox_strategies: (schema) => rows(schema, ['hardlink', 'symlink', 'direct'].map((strategy) => ({ strategy, available: true, reason: null }))),
    sandbox_spec: (schema) => {
        const base = fake((schema as Sch)._zod.def.innerType ?? schema) as Obj
        return { ...base, deploy: { ...(base.deploy as Obj), defaultStrategy: 'hardlink', supportedStrategies: ['direct', 'hardlink', 'symlink'] } }
    },
    sandbox_list: (schema) => {
        const mod = (name: string, i: number, v: string) => ({ modKey: `mod:${2 + i}`, kind: 'mod', itemId: 2 + i, name, enabled: true, priority: i, releaseId: 100 + i, version: v, stagedAt: iso(60 * 20), lastError: null })
        const box = (id: number, appId: number, appSlug: string, appName: string, name: string, description: string, mods: unknown[], isDefault: boolean) => ({
            id, remoteId: 4000 + id, appId, appSlug, appName, name, description, environment: 'client', strategy: 'hardlink', gameVersion: null, loader: null, preset: null,
            isDefault, cloudSync: true, autoUpdate: true, gameDir: null, options: {}, launchArgs: [], launchEnv: {}, deployedAt: iso(60 * 3), lastDeploy: null,
            createdAt: iso(60 * 24 * 30), updatedAt: iso(60 * 3), mods, needsDeploy: false, deployedFiles: 214 + id * 37, targetDir: `~/Games/${appName.replace(/ /g, '')}`,
        })
        return rows(schema, [
            box(1, 100, 'playlot', 'Playlot', 'Party night', 'Murder Mystery, Prop Hunt and Deathrun maps for Friday games.', [mod('Murder Mystery Maps', 0, '2.3.0'), mod('Prop Hunt Maps', 2, '1.2.2'), mod('Deathrun Maps', 3, '1.4.3')], true),
            box(2, 100, 'playlot', 'Playlot', 'Vanilla', 'Exactly what ships. Nothing added.', [], false),
            box(3, 101, 'fatal-contact', 'Fatal Contact', 'Competitive', 'Tournament maps and the Ember HUD.', [mod('Zombie Zone Maps', 1, '1.3.1')], true),
        ])
    },
}

/** Content packs with art in ./art (banners from the GameSpice map repos). */
const PACKS = [
    { art: 'murder-mystery', name: 'Murder Mystery Maps', desc: 'Manors, trains and harbours built for one killer and a room full of suspects.', owner: 'novarift', who: 'NovaRift', tags: ['social', 'deduction'], dl: 18420, likes: 2310 },
    { art: 'zombie-zone', name: 'Zombie Zone Maps', desc: 'Barricade, hold and survive: eight co-op arenas with waves that adapt.', owner: 'kiterunner', who: 'KiteRunner', tags: ['co-op', 'survival'], dl: 15102, likes: 1890 },
    { art: 'mg-prop-hunt', name: 'Prop Hunt Maps', desc: 'Kitchens, warehouses and toy shops full of places to hide in plain sight.', owner: 'pixelmarrow', who: 'Pixel Marrow', tags: ['party', 'hide'], dl: 12877, likes: 1502 },
    { art: 'mg-deathrun', name: 'Deathrun Maps', desc: 'Traps, timing and one runner too many: twelve courses with records.', owner: 'saffronfox', who: 'Saffron Fox', tags: ['parkour', 'traps'], dl: 9640, likes: 1214 },
    { art: 'hide-n-seek', name: 'Hide and Seek Maps', desc: 'Big, quiet places with too many corners. Ready, steady, seek.', owner: 'lumenwolf', who: 'Lumen Wolf', tags: ['party', 'family'], dl: 7311, likes: 988 },
    { art: 'mg-dangerous-delivery', name: 'Dangerous Delivery Maps', desc: 'Get the parcel across town before the rival crew does.', owner: 'novarift', who: 'NovaRift', tags: ['racing', 'teams'], dl: 5208, likes: 702 },
    { art: 'lam', name: 'Look At Me Maps', desc: 'Stages and spotlights for the game where everyone wants to be seen.', owner: 'pixelmarrow', who: 'Pixel Marrow', tags: ['social', 'arena'], dl: 3920, likes: 515 },
]
const art = (name: string) => `${location.origin}/scripts/shots/art/${name}.webp`
const PLAYLOT = { id: 100, name: 'Playlot', url: 'playlot', icon: 'https://tmc-city-dev.s3.us-east-1.amazonaws.com/public/app/playground/icon.webp' }

type Obj = Record<string, unknown>

/** API reads the dev site has too little of (or not yet): built on its real shapes. */
async function apiSeed(a: Obj, real: () => Promise<unknown>): Promise<unknown> {
    const path = String(a.path)
    const q = new Map((a.query as [string, string][] | undefined) ?? [])
    if (path === '/browse' && q.get('kind') === 'mod') {
        const r = (await real()) as { items: Obj[]; total: number; nextCursor: unknown }
        const t = r.items[0]
        return {
            ...r, total: PACKS.length, nextCursor: null,
            items: PACKS.map((p, i) => ({
                ...t, id: String(2 + i), url: p.art, name: p.name, description: p.desc,
                images: { card: art(p.art), banner: art(p.art), icon: null },
                app: PLAYLOT, community: null, categories: [],
                owner: { id: `demo-${p.owner}`, name: p.who, username: p.owner, avatar: null },
                tags: p.tags.map((n, k) => ({ id: 9000 + i * 10 + k, name: n, url: null })),
                createdAt: iso(60 * 24 * (40 - i * 3)), updatedAt: iso(60 * (5 + i * 7)),
                stats: { ...(t.stats as Obj), views: p.dl * 3, downloads: p.dl, favorites: Math.round(p.likes / 3), likes: p.likes, comments: Math.round(p.likes / 9), reviews: Math.round(p.likes / 20), rating: 4.4 + (i % 5) / 10 },
                webUrl: `https://tmcdev.net/m/${2 + i}`,
            })),
        }
    }
    if (path === '/facets' && (q.get('kind') === 'server' || q.get('kind') === 'mod')) {
        const r = (await real()) as Obj
        const fc = { id: 101, name: 'Fatal Contact', url: 'fatal-contact', icon: 'https://tmc-city-dev.s3.us-east-1.amazonaws.com/public/app/arena/icon.webp' }
        const apps = q.get('kind') === 'server' ? [{ ...PLAYLOT, count: 3 }, { ...fc, count: 3 }] : [{ ...PLAYLOT, count: PACKS.length }]
        return { ...r, apps, categories: [], ...(q.get('kind') === 'server' ? { maps: [] } : {}) }
    }
    if (path === '/browse' && q.get('kind') === 'server') {
        const r = (await real()) as { items: Obj[]; total: number }
        const t = r.items[0]
        const ts = t.server as Obj
        const FC = { id: 101, name: 'Fatal Contact', url: 'fatal-contact', icon: 'https://tmc-city-dev.s3.us-east-1.amazonaws.com/public/app/arena/icon.webp' }
        const demo = SERVERS.map((d, i) => ({
            ...t, id: `demo-${i}`, url: slug(d.name), name: d.name, description: null,
            app: i % 2 ? PLAYLOT : FC, owner: { id: 'demo-ember', name: 'Ember', username: 'ember', avatar: null },
            server: { ...ts, host: hostOf(i), port: 27015, queryPort: 27015, curUsers: d.players, maxUsers: d.max, bots: 0, map: d.map, online: true },
            webUrl: `https://tmcdev.net/s/${slug(d.name)}`,
        }))
        return { ...r, total: demo.length, nextCursor: null, items: demo }
    }
    if (path === '/content/mod/2') {
        const r = (await real()) as Obj
        const p = PACKS[0]
        const sum = r.summary as Obj
        return {
            ...r,
            // The pack art carries its own title, which doubled the page's heading
            // under a dark banner: the page gets a text-free strip of the same banner.
            summary: { ...sum, name: p.name, url: p.art, description: p.desc, app: PLAYLOT, community: null,
                images: { card: art(p.art), banner: art(`${p.art}-banner`), icon: null },
                owner: { id: `demo-${p.owner}`, name: p.who, username: p.owner, avatar: null },
                tags: p.tags.map((n, k) => ({ id: 9000 + k, name: n, url: null })),
                stats: { ...(sum.stats as Obj), views: 55260, downloads: p.dl, likes: p.likes, favorites: 770, comments: 256, reviews: 115, rating: 4.8 } },
            content: [
                '## Six maps, one killer', '',
                'A manor on a stormy night, a sleeper train, a harbour market, a theatre, a ski lodge and a museum after closing. Every map has secret passages, clue spots and a lights-out switch.', '',
                '## What you get', '',
                '- Six maps sized for 6 to 16 players', '- Clue props and spawn points for the Murder Mystery mode', '- Night and storm variants of every map', '',
                '## Installing', '',
                'Press **Install** and pick a sandbox. Nothing in your game folder is overwritten.',
            ].join('\n'),
            links: [], dependencies: [], media: [],
            releases: ((r.releases as Obj[]) ?? []).slice(0, 1).map((x) => ({ ...x, version: '2.3.0', createdAt: iso(60 * 30) })),
        }
    }
    // The review account has played nothing, earned nothing and its friends are
    // all offline: Stats, Store and Friends were empty states in the tour.
    if (path === '/me/stats') {
        // Days are YYYY-MM-DD; a realistic month has rest days in it.
        const day = (minsAgo: number) => iso(minsAgo).slice(0, 10)
        const pl = [1, 1, 0, 1, 1, 1, 0, 0, 1, 1, 0, 1, 1, 1, 1, 0, 1, 0, 1, 1, 1, 0, 0, 1, 1, 1, 0, 1, 1, 1]
        const fc = [0, 1, 0, 0, 1, 0, 0, 1, 0, 1, 0, 0, 1, 0, 1, 0, 0, 1, 0, 1, 0, 0, 1, 0, 1, 1, 0, 0, 1, 0]
        const recent = pl.map((v, d) => v + fc[d])
        return {
            since: day(60 * 24 * 29),
            totals: { games: 2, daysPlayed: 64, activeDays: 51, achievements: 23, achievementPoints: 1840 },
            recent,
            games: [
                { appId: 100, daysPlayed: 41, firstPlayed: day(60 * 24 * 120), lastPlayed: day(0), recent: pl, achievements: { unlocked: 15, points: 1210, total: 32 } },
                { appId: 101, daysPlayed: 23, firstPlayed: day(60 * 24 * 75), lastPlayed: day(60 * 24), recent: fc, achievements: { unlocked: 8, points: 630, total: 24 } },
            ],
        }
    }
    if (path === '/me/points') {
        return {
            site: { earned: 2400, spent: 600, available: 1800 },
            apps: [
                { appId: 100, trusted: true, earned: 1650, spent: 400, available: 1250 },
                { appId: 101, trusted: true, earned: 980, spent: 280, available: 700 },
            ],
            total: 3750,
        }
    }
    if (path === '/store/directory') {
        const r = (await real()) as { stores?: Obj[] } | null
        const have = new Set((r?.stores ?? []).map((s) => s.appId))
        return { stores: [...(r?.stores ?? []), ...(have.has(100) ? [] : [{ appId: 100, name: 'Playlot', items: 36 }])] }
    }
    if (path === '/friends') {
        const r = (await real()) as Obj[] | null
        if (!Array.isArray(r)) return r
        const live: Obj[] = [
            { status: 'in_game', appId: 101, appName: 'Fatal Contact', serverId: null, serverName: SERVERS[0].name, partyId: null, joinable: true, detail: SERVERS[0].map, updatedAt: iso(4) },
            { status: 'online', appId: null, appName: null, serverId: null, serverName: null, partyId: null, joinable: false, detail: null, updatedAt: iso(1) },
            { status: 'in_game', appId: 100, appName: 'Playlot', serverId: null, serverName: null, partyId: null, joinable: false, detail: null, updatedAt: iso(12) },
        ]
        return r.map((f, i) => (live[i] ? { ...f, presence: live[i] } : f))
    }
    if (path === '/play/page') {
        // A dev game with no art yet is a blank card in a marketing shot
        // (Hungario, The Room): show the games that have their banner.
        const r = (await real()) as { apps?: Obj[]; totals?: Obj } | null
        if (!r?.apps) return r
        const apps = r.apps.filter((x) => x.bannerUrl)
        return { ...r, apps, totals: r.totals && 'games' in r.totals ? { ...r.totals, games: apps.length } : r.totals }
    }
    if (path === '/workshop/catalog') {
        const games = [{ id: 100, url: 'playlot', contentId: 'app:100', name: 'Playlot' }, { id: 101, url: 'fatal-contact', contentId: 'app:101', name: 'Fatal Contact' }]
        return {
            total: PACKS.length, page: 1, per: 24, subscribed: [3, 5],
            facets: { kinds: ['maps', 'modes', 'skins'], games },
            items: PACKS.map((p, i) => ({
                id: 2 + i, name: p.name, summary: p.desc, body: null, image: art(p.art),
                owner: { name: p.who, url: null }, official: i < 2, kinds: ['maps'], compat: 'listed', games: [games[0]],
                appId: 100, version: `1.${4 - (i % 3)}.${i}`, bytes: (40 + i * 23) * 1024 * 1024, runsCode: false,
                subscribers: Math.round(p.dl / 4), createdAt: iso(60 * 24 * (50 - i)), updatedAt: iso(60 * (3 + i * 5)), webUrl: `https://tmcdev.net/m/${2 + i}`,
            })),
        }
    }
    return undefined
}

/** Dev placeholder text ("asdsadsad") reads as a bug in a marketing shot: drop those rows. */
const mash = (v: unknown) => typeof v === 'string' && /^(asd|qwe|test|lorem)[a-z]*$/i.test(v.trim())
function scrub(r: unknown): unknown {
    if (!r || typeof r !== 'object') return r
    const o = r as Obj
    for (const k of ['apps', 'items']) {
        if (Array.isArray(o[k])) o[k] = (o[k] as Obj[]).filter((x) => !mash(x.description) && !mash(x.summary))
    }
    return o
}

/** Extra rows a screen can opt into before load (window.__shotsSeed). */
const extra = (globalThis as { __shotsSeed?: Record<string, (schema: unknown, a: Record<string, unknown>) => unknown> }).__shotsSeed

const inner = g.__TAURI_INTERNALS__.invoke.bind(g.__TAURI_INTERNALS__)
g.__TAURI_INTERNALS__.invoke = async (cmd, args, opts) => {
    if (cmd === 'settings_get' || cmd === 'settings_patch') {
        // The theme a shot asks for (capture.mjs sets __shotsTheme), and English.
        const r = (await inner(cmd, args, opts)) as Record<string, unknown>
        const theme = (globalThis as { __shotsTheme?: string }).__shotsTheme
        // The app's own defaults, not the mock's zeros.
        Object.assign(r, cmd === 'settings_get' && !r.__shots ? {
            __shots: true, version: 1, uiScale: 1, liveLatency: true, latencyIntervalMs: 30000, latencyConcurrency: 8,
            gameDirs: { 100: '~/Games/Playlot', 101: '~/Games/FatalContact' }, autoUpdateCheck: true, crashReports: true, downloadConcurrency: 3, downloadKeepHistory: true,
            gameCacheKeep: true, gameUseUdp: true, requireSignedPlugins: true, allowConfigEditing: true, minimiseToTray: true,
        } : {})
        return theme ? Object.assign(r, { theme }) : r
    }
    if (cmd === 'api_get') {
        const real = () => inner(cmd, args, opts)
        const seeded = await apiSeed((args ?? {}) as Obj, real)
        return seeded === undefined ? scrub(await real()) : seeded
    }
    if (cmd === 'avatar_asset') {
        // Keep the asset's host (vite.config.ts, /__shots-asset).
        const u = new URL(String((args as Obj).url))
        const res = await fetch(`/__shots-asset/${u.host}${u.pathname}${u.search}`)
        if (!res.ok) throw { code: 'api', message: `HTTP ${res.status}` }
        return res.arrayBuffer()
    }
    const fn = extra?.[cmd] ?? seeds[cmd]
    if (!fn) return inner(cmd, args, opts)
    return fn(g.__reviewSchema, (args ?? {}) as Record<string, unknown>)
}
