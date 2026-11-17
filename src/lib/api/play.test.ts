/**
 * The play center's data layer: the three responses parsed through the
 * contract's own schemas, and the URL and query the screen builds.
 *
 * The fixtures are shaped from `src/lib/api/contract.ts` (not captured from a
 * live site, which had not deployed these routes when this was written), so
 * they pin the app's side of the agreement: a schema change that stops them
 * parsing is a contract drift worth seeing here rather than on a screen.
 */

import { describe, expect, it } from 'vitest'

import {
    PlayLaunchResponseSchema,
    PlayPageEnvelopeSchema,
    PlayPageQuerySchema,
    PlayPlayableResponseSchema,
    PlayQuickMatchResponseSchema,
    PartyInvitesResponse,
} from './contract'
import {
    DEFAULT_FILTERS,
    TAB_SORTS,
    fill,
    filtersFromParams,
    filtersToParams,
    isNarrowed,
    pageQuery,
    players,
    resolveSort,
    type PlayFilters,
} from './play'
import fixtures from './play.fixtures.json'

const q = (search: string) => new URLSearchParams(search)

describe('responses', () => {
    it.each(['games', 'gamesLast', 'servers', 'lobbies', 'invites', 'emptyNothing', 'emptySearch'] as const)(
        'parses the %s page',
        (key) => {
            expect(PlayPageEnvelopeSchema.safeParse(fixtures[key]).success).toBe(true)
        }
    )

    it('keeps the other tabs empty and the cursor for the next page', () => {
        const page = PlayPageEnvelopeSchema.parse(fixtures.games)

        expect(page?.apps.length).toBeGreaterThan(0)
        expect(page?.servers).toEqual([])
        expect(page?.nextCursor).toBe(40)
        expect(page?.party?.app.name).toBe('Fatal Contact')
    })

    it('reads the feature being off as null, not as an error', () => {
        expect(PlayPageEnvelopeSchema.parse(null)).toBeNull()
        expect(PlayQuickMatchResponseSchema.parse(null)).toBeNull()
    })

    it('rejects a page whose rows are missing a field', () => {
        const broken = structuredClone(fixtures.servers) as unknown as { servers: { curUsers?: number }[] }

        delete broken.servers[0]?.curUsers

        expect(PlayPageEnvelopeSchema.safeParse(broken).success).toBe(false)
    })

    it('defaults a game with no launch options to an empty list', () => {
        const row = structuredClone(fixtures.games.apps[0]) as Record<string, unknown>

        delete row.options

        const page = PlayPageEnvelopeSchema.parse({ ...fixtures.games, apps: [row] })

        expect(page?.apps[0]?.options).toEqual([])
    })

    it('parses playable and quick match', () => {
        const playable = PlayPlayableResponseSchema.parse(fixtures.playable)

        expect(playable.directIds).toEqual(expect.arrayContaining(playable.soloIds))

        const quick = PlayQuickMatchResponseSchema.parse(fixtures.quick)

        expect(quick?.best?.id).toBe(quick?.top[0]?.id)
        expect(PlayQuickMatchResponseSchema.parse(fixtures.quickNone)?.best).toBeNull()
    })

    it('parses the party invites the invites tab shows', () => {
        expect(PartyInvitesResponse.parse(fixtures.invitesList)).toHaveLength(2)
    })
})

describe('filters in the URL', () => {
    it('opens on games with nothing narrowed', () => {
        expect(filtersFromParams(q(''))).toEqual({
            tab: 'games',
            q: '',
            appId: undefined,
            mode: undefined,
            sort: undefined,
            populated: false,
            joinable: false,
        })
    })

    it('drops a bad value instead of the page', () => {
        const f = filtersFromParams(q('tab=nope&sort=sideways&mode=vr&app=abc&populated=true'))

        expect(f.tab).toBe('games')
        expect(f.sort).toBeUndefined()
        expect(f.mode).toBeUndefined()
        expect(f.appId).toBeUndefined()
        // Only the literal 1 is a switch, as everywhere else.
        expect(f.populated).toBe(false)
    })

    it('round-trips and leaves defaults out of the address', () => {
        const f: PlayFilters = {
            ...DEFAULT_FILTERS,
            tab: 'servers',
            q: 'surf',
            appId: 4,
            mode: 'app',
            sort: 'name',
            populated: true,
        }

        expect(filtersFromParams(filtersToParams(f))).toEqual({ ...f, joinable: false })
        expect(filtersToParams(DEFAULT_FILTERS).toString()).toBe('')
        expect(filtersToParams({ ...DEFAULT_FILTERS, tab: 'lobbies', sort: 'recent' }).toString()).toBe(
            'tab=lobbies'
        )
    })

    it('caps a hand-edited search at the contract limit', () => {
        expect(filtersFromParams(q(`q=${'x'.repeat(500)}`)).q).toHaveLength(120)
    })
})

describe('the request for a page', () => {
    it('sends only the switch that belongs to the tab', () => {
        const all = { ...DEFAULT_FILTERS, populated: true, joinable: true }

        expect(pageQuery({ ...all, tab: 'servers' }).populated).toBe(true)
        expect(pageQuery({ ...all, tab: 'servers' }).joinable).toBeUndefined()
        expect(pageQuery({ ...all, tab: 'lobbies' }).joinable).toBe(true)
        expect(pageQuery({ ...all, tab: 'lobbies' }).populated).toBeUndefined()
        expect(pageQuery({ ...all, tab: 'games' }).populated).toBeUndefined()
    })

    it('leaves out a sort the tab cannot express', () => {
        expect(pageQuery({ ...DEFAULT_FILTERS, tab: 'servers', sort: 'trending' }).sort).toBeUndefined()
        expect(pageQuery({ ...DEFAULT_FILTERS, tab: 'games', sort: 'trending' }).sort).toBe('trending')
    })

    it('sends nothing but the tab for invites', () => {
        expect(pageQuery({ ...DEFAULT_FILTERS, tab: 'invites', q: 'x', appId: 3 })).toEqual({ tab: 'invites' })
    })

    it('does not narrow the games list by a game', () => {
        expect(pageQuery({ ...DEFAULT_FILTERS, appId: 4 }).appId).toBeUndefined()
        expect(pageQuery({ ...DEFAULT_FILTERS, tab: 'servers', appId: 4 }).appId).toBe(4)
    })

    it('is accepted by the contract, and carries the cursor', () => {
        const parsed = PlayPageQuerySchema.parse(
            pageQuery({ ...DEFAULT_FILTERS, tab: 'servers', q: 'surf', populated: true }, 40)
        )

        expect(parsed).toMatchObject({ tab: 'servers', q: 'surf', populated: true, cursor: 40 })
        expect(PlayPageQuerySchema.parse(pageQuery(DEFAULT_FILTERS)).cursor).toBe(0)
    })

    it('resolves every offered sort to itself and anything else to the default', () => {
        for (const [tab, sorts] of Object.entries(TAB_SORTS))
            for (const sort of sorts)
                expect(resolveSort(tab as keyof typeof TAB_SORTS, sort)).toBe(sort)

        expect(resolveSort('lobbies', 'trending')).toBe('recent')
        expect(resolveSort('games', undefined)).toBe('players')
    })
})

describe('row facts', () => {
    it('tells narrowing from emptiness', () => {
        expect(isNarrowed(DEFAULT_FILTERS)).toBe(false)
        expect(isNarrowed({ ...DEFAULT_FILTERS, q: 'x' })).toBe(true)
        expect(isNarrowed({ ...DEFAULT_FILTERS, joinable: true })).toBe(true)
    })

    it('treats a server with no declared limit as unbounded, never full', () => {
        expect(fill(5, 0)).toBeNull()
        expect(players(5, 0)).toBe('5')
        expect(fill(40, 32)).toBe(1)
        expect(players(3, 16)).toBe('3/16')
    })
})

describe('app-mode launch response', () => {
    it('carries the solo pack, and an older answer without it still parses', () => {
        const uri = 'tmc://play/app/fatal-contact?solo=tmc/fatal-contact&solo-version=1.2.0'
        const withSolo = PlayLaunchResponseSchema.parse({
            mode: 'app',
            uri,
            solo: { contentId: 'tmc/fatal-contact', version: '1.2.0' },
        })
        expect(withSolo).toMatchObject({ solo: { contentId: 'tmc/fatal-contact', version: '1.2.0' } })

        const latest = PlayLaunchResponseSchema.parse({
            mode: 'app',
            uri,
            solo: { contentId: 'tmc/fatal-contact', version: null },
        })
        expect(latest).toMatchObject({ solo: { version: null } })

        expect(PlayLaunchResponseSchema.parse({ mode: 'app', uri })).toMatchObject({ uri })
        expect(PlayLaunchResponseSchema.parse({ mode: 'app', uri, solo: null })).toMatchObject({ uri })
    })
})
