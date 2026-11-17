import { describe, expect, it } from 'vitest'

import {
    FriendListResponse as FriendListSchema,
    FriendRequestsResponse as FriendRequestsSchema,
    PartyInviteRespondResponse,
    PartyInvitesResponse as PartyInvitesSchema,
    PartyMineResponse as PartyMineSchema,
    StatDefsResponse,
    StatsMeResponse as StatsMeSchema,
    StatsTopResponse as StatsTopSchema,
} from './contract'
import { formatStat, isStatKey, presenceLabel } from './social'

/*
 * The payloads below are website-city's own examples (docs/api/app-social.md)
 * and the shapes its handlers build, parsed through the mirrored contract's
 * "Social" schemas — the same ones `client.ts` parses every response with.
 */

const offline = {
    status: 'offline',
    appId: null,
    appName: null,
    serverId: null,
    serverName: null,
    partyId: null,
    joinable: false,
    detail: null,
    updatedAt: null,
}

describe('friends', () => {
    it('parses a friend list with a player in a game', () => {
        const list = FriendListSchema.parse([
            {
                userId: 'clx1',
                displayName: 'Ashley',
                avatarUrl: null,
                presence: {
                    status: 'in_game',
                    appId: 3,
                    appName: 'Fatal Contact',
                    serverId: 4821,
                    serverName: 'EU #1',
                    partyId: '4471',
                    joinable: true,
                    detail: 'Round 3',
                    updatedAt: '2026-09-23T20:00:10.000Z',
                },
            },
            {
                userId: 'clx2',
                displayName: 'Bo',
                avatarUrl: null,
                presence: offline,
            },
        ])

        expect(presenceLabel(list[0]!.presence)).toBe('Playing EU #1')
        expect(presenceLabel(list[1]!.presence)).toBe('Offline')
    })

    it('refuses a presence status the site does not send', () => {
        expect(() =>
            FriendListSchema.parse([
                {
                    userId: 'x',
                    displayName: 'x',
                    avatarUrl: null,
                    presence: { ...offline, status: 'busy' },
                },
            ])
        ).toThrow()
    })

    it('parses requests, whose id is the integer the answer sends back', () => {
        const r = FriendRequestsSchema.parse({
            incoming: [
                {
                    id: 12,
                    userId: 'clx3',
                    displayName: 'Cy',
                    createdAt: '2026-09-20T00:00:00Z',
                },
            ],
            outgoing: [],
        })

        expect(r.incoming[0]!.id).toBe(12)
        expect(() =>
            FriendRequestsSchema.parse({
                incoming: [
                    { id: '12', userId: 'a', displayName: 'a', createdAt: 'x' },
                ],
                outgoing: [],
            })
        ).toThrow()
    })
})

describe('parties', () => {
    it('parses the documented party, and null for none', () => {
        const party = PartyMineSchema.parse({
            id: '4471',
            name: 'Scrim',
            type: 'PUBLIC',
            techType: 'NATIVE',
            maxUsers: 10,
            users: 4,
            stage: 'LOBBY',
            startTime: '2026-09-23T20:00:00.000Z',
            endTime: null,
            mapName: null,
            gameMode: null,
            hostId: 'clx…',
            appId: 3,
            serverId: null,
            members: [
                {
                    userId: 'clx…',
                    displayName: 'Ashley',
                    gameName: null,
                    role: 'HOST',
                    state: 'JOINED',
                    presence: 'NONE',
                    joinedAt: '2026-09-23T20:00:00.000Z',
                    leftAt: null,
                    readyAt: null,
                    connectedAt: null,
                    score: 0,
                    kills: 0,
                    deaths: 0,
                    assist: 0,
                },
            ],
        })

        expect(party?.members[0]?.role).toBe('HOST')
        expect(PartyMineSchema.parse(null)).toBeNull()
    })

    it('keeps party ids as strings, since they can pass 2^53', () => {
        const [invite] = PartyInvitesSchema.parse([
            {
                id: '9007199254740993',
                partyId: '9007199254740995',
                partyName: 'Late night',
                inviterId: null,
                message: null,
            },
        ])

        expect(invite!.id).toBe('9007199254740993')
    })

    it('reads an invite answer as null (declined) or the join (accepted)', () => {
        expect(PartyInviteRespondResponse.parse(null)).toBeNull()
        expect(
            PartyInviteRespondResponse.parse({
                partyId: '4471',
                role: 'MEMBER',
                leftPartyId: null,
            })?.partyId
        ).toBe('4471')
    })

    it('refuses a party roster missing what the site always sends', () => {
        expect(() => PartyMineSchema.parse({ id: '1', members: [] })).toThrow()
    })
})

describe('stats', () => {
    it('parses the picker list, with a kind the app has no label for', () => {
        const [def] = StatDefsResponse.parse([
            {
                key: 'kills',
                name: 'Kills',
                description: null,
                kind: 'SOMETHING_NEW',
                unit: '',
                decimals: 0,
                players: 3,
            },
        ])

        expect(def!.players).toBe(3)
    })

    it('parses a ranking with the caller beside it', () => {
        const top = StatsTopSchema.parse({
            stat: {
                key: 'kills',
                name: 'Kills',
                kind: 'COUNTER',
                unit: '',
                decimals: 0,
                players: 2,
            },
            rows: [
                { rank: 1, player: 'p1', name: 'Ashley', value: 40 },
                { rank: 1, player: 'p2', name: 'Bo', value: 40 },
            ],
            self: null,
        })

        // A tie is joint, as the site ranks it.
        expect(top.rows.map((r) => r.rank)).toEqual([1, 1])
    })

    it('parses a member with no figures yet', () => {
        expect(
            StatsMeSchema.parse({ player: 'k', name: null, stats: [] }).stats
        ).toEqual([])
    })

    it('accepts the stat keys the site accepts, and nothing else', () => {
        for (const good of ['kills', 'best_lap', 'time.trial:1', 'a-b', '9lives'])
            expect(isStatKey(good)).toBe(true)

        for (const bad of ['', '_x', 'two words', 'a/b', 'k'.repeat(65), '../x'])
            expect(isStatKey(bad)).toBe(false)
    })

    it('formats a value in the stat’s own decimals and unit', () => {
        expect(formatStat(12.3456, 2, 's')).toMatch(/^12[.,]35 s$/)
        expect(formatStat(7, 0, '')).toBe('7')
        // A definition asking for absurd precision does not throw.
        expect(() => formatStat(1, 1000, '')).not.toThrow()
    })
})
