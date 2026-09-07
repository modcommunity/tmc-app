/**
 * The offline cache, and mostly the allow-list.
 *
 * This module decides what gets written to disk in cleartext, so the half of it
 * worth testing is not "does the cache work" but "does it refuse the things it
 * must refuse" — the signed-in user's profile, the audit trail, and the plugin
 * registry with its approvals. Those three are excluded by not appearing on a
 * list, which is a property a comment cannot enforce and a test can.
 *
 * `localStorage` is four stubbed methods rather than a DOM implementation. What
 * is under test is a filter and a timestamp, and neither wants jsdom.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { QueryClient } from '@tanstack/react-query'

import { attachOfflineCache } from './offline-cache'

const STORAGE_KEY = 'tmc.offline-cache.v1'

let store: Map<string, string>

beforeEach(() => {
    store = new Map()

    vi.stubGlobal('window', {
        localStorage: {
            getItem: (key: string) => store.get(key) ?? null,
            setItem: (key: string, value: string) => void store.set(key, value),
            removeItem: (key: string) => void store.delete(key),
        },
        addEventListener: () => {},
        removeEventListener: () => {},
    })
})

afterEach(() => {
    vi.unstubAllGlobals()
    vi.useRealTimers()
})

/** Seed a client, let the debounce fire, and return what was written. */
async function persist(seed: (client: QueryClient) => void): Promise<string> {
    vi.useFakeTimers()

    const client = new QueryClient()
    const detach = attachOfflineCache(client)

    seed(client)

    await vi.advanceTimersByTimeAsync(3_000)
    detach()

    vi.useRealTimers()

    return store.get(STORAGE_KEY) ?? ''
}

/** What a fresh launch would find in the cache. */
function coldStart(): QueryClient {
    const client = new QueryClient()

    attachOfflineCache(client)

    return client
}

describe('what is written', () => {
    it('keeps public catalogue data', async () => {
        const raw = await persist((client) => {
            client.setQueryData(['browse', { kind: 'mod' }], { items: [1, 2] })
            client.setQueryData(['facets', 'mod'], { apps: [{ id: 1 }] })
            client.setQueryData(['content', 'mod', 7], { name: 'Cool Mod' })
            client.setQueryData(['reviews', 'mod', 7], { items: [] })
        })

        for (const key of ['browse', 'facets', 'content', 'reviews']) {
            expect(raw).toContain(key)
        }
    })

    /*
     * The reason this file exists. Each of these would be a real disclosure:
     * an account profile, the audit trail the user controls, and a registry of
     * plugin approvals — none of them excluded by name anywhere, which is the
     * design and therefore the thing to hold in place.
     */
    it.each([
        ['the account profile', ['me'], { email: 'someone@example.test' }],
        ['the audit log', ['log'], [{ message: 'a private action' }]],
        ['the plugin registry', ['plugins'], [{ id: 'x', enabled: true }]],
        ['a log path', ['log-path'], '/home/someone/logs'],
    ])('never writes %s', async (_what, key, value) => {
        const raw = await persist((client) => {
            // Something persistable too, so an empty snapshot cannot be what
            // makes this pass.
            client.setQueryData(['browse', {}], { items: [] })
            client.setQueryData(key, value)
        })

        expect(raw).toContain('browse')
        expect(raw).not.toContain(JSON.stringify(value).slice(2, 20))
    })
})

describe('what comes back', () => {
    it('restores public data on a cold start', async () => {
        await persist((client) => {
            client.setQueryData(['browse', { kind: 'mod' }], { items: [1, 2] })
        })

        expect(coldStart().getQueryData(['browse', { kind: 'mod' }])).toEqual({
            items: [1, 2],
        })
    })

    /*
     * The single most important behaviour in the module. React Query compares
     * `dataUpdatedAt` against `staleTime` to decide whether to refetch, so
     * restoring these as fresh would show week-old data and then never go and
     * look for better — an app that is silently, permanently stale.
     */
    it('restores the original timestamp, so a refetch still happens', async () => {
        const then = Date.now() - 5 * 60 * 1000

        await persist((client) => {
            client.setQueryData(['browse', {}], { items: [] }, { updatedAt: then })
        })

        const state = coldStart()
            .getQueryCache()
            .find({ queryKey: ['browse', {}] })?.state

        expect(state?.dataUpdatedAt).toBe(then)
        expect(state?.dataUpdatedAt).toBeLessThan(Date.now() - 60_000)
    })

    /** The allow-list is enforced on READ as well, so a hand-edited snapshot
     * cannot inject a key the writer would have refused. */
    it('refuses a non-persistable key found in the file', () => {
        store.set(
            STORAGE_KEY,
            JSON.stringify([
                {
                    key: ['me'],
                    data: { email: 'injected@example.test' },
                    updatedAt: Date.now(),
                },
            ])
        )

        expect(coldStart().getQueryData(['me'])).toBeUndefined()
    })

    it('drops entries past the age cap and keeps recent ones', () => {
        const day = 24 * 60 * 60 * 1000

        store.set(
            STORAGE_KEY,
            JSON.stringify([
                {
                    key: ['browse', 'old'],
                    data: { items: [9] },
                    updatedAt: Date.now() - 30 * day,
                },
                {
                    key: ['browse', 'new'],
                    data: { items: [8] },
                    updatedAt: Date.now() - 1_000,
                },
            ])
        )

        const client = coldStart()

        expect(client.getQueryData(['browse', 'old'])).toBeUndefined()
        expect(client.getQueryData(['browse', 'new'])).toEqual({ items: [8] })
    })
})

describe('when the snapshot is unusable', () => {
    /*
     * Every byte of this cache is one fetch away, so a launch that fails
     * because it is corrupt is a far worse outcome than a launch with an empty
     * one. None of these may throw.
     */
    it.each([
        'not json at all',
        '{}',
        'null',
        '[]',
        '[{"key":"not an array"}]',
        '[null]',
        '[{"key":["browse"],"updatedAt":"yesterday"}]',
    ])('survives %s', (junk) => {
        store.set(STORAGE_KEY, junk)

        expect(() => coldStart()).not.toThrow()
    })

    it('survives storage being unavailable entirely', () => {
        vi.stubGlobal('window', {
            localStorage: {
                getItem: () => {
                    throw new Error('disabled')
                },
                setItem: () => {
                    throw new Error('disabled')
                },
                removeItem: () => {
                    throw new Error('disabled')
                },
            },
            addEventListener: () => {},
            removeEventListener: () => {},
        })

        expect(() => coldStart()).not.toThrow()
    })
})
