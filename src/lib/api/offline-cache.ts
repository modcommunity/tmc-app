/**
 * What the app can show before the network answers, including when it never
 * will.
 *
 * React Query is memory-only, so a cold launch started offline — a laptop on a
 * train, a phone in a lift, a desktop whose Wi-Fi has not associated yet —
 * showed empty lists and a spinner, then an error. For a browser tab that is
 * correct behaviour. For an installed app it reads as the app being broken,
 * because a native app that has been opened before is expected to remember what
 * it showed.
 *
 * So the last successful responses are written to `localStorage` and put back
 * into the cache at boot, with their ORIGINAL timestamps. That last part is the
 * whole design: React Query then sees entries older than `staleTime`, renders
 * them immediately and refetches in the background. Restoring them as fresh
 * would suppress the refetch and pin the app to whatever it last saw.
 *
 * WHAT IS PERSISTED, AND WHY IT IS AN ALLOW-LIST
 * ---------------------------------------------
 * Only [`PERSISTED`] — browse pages, item details, facets, reviews and the app
 * catalogue. All five are public catalogue data that the server would hand to
 * anybody.
 *
 * `apps` earns its place more than most: it is the FIRST screen, so a cold
 * launch with no network shows the games it saw last time instead of an empty
 * grid, and the catalogue changes on the scale of weeks.
 *
 * An allow-list rather than "everything except the sensitive ones", because the
 * failure modes are not symmetrical: forgetting to exclude a new query that
 * holds account data writes it to disk in cleartext, while forgetting to
 * include a new public one costs a spinner. Three of the keys in use today
 * would be actively wrong to store —
 *
 *   * `me` is the signed-in user's own profile and settings;
 *   * `log` is the audit trail, which exists precisely so that what the app did
 *     on someone's behalf is recorded in ONE place they control;
 *   * `plugins` is the installed-plugin registry with its approvals and
 *     fingerprints, and a stale copy of that on screen would show permissions
 *     that may since have drifted.
 *
 * — and none of them is excluded by name here, which is the point.
 *
 * `localStorage` rather than the app's SQLite: this is a render-time cache of
 * public data that is thrown away whenever it is inconvenient, and putting it
 * through the IPC bridge would mean a command that writes arbitrary
 * webview-supplied bytes to disk. There is deliberately no such command.
 */

import type { QueryClient } from '@tanstack/react-query'

const STORAGE_KEY = 'tmc.offline-cache.v1'

/** Query-key prefixes that may be written to disk. Public data only. */
const PERSISTED = ['browse', 'content', 'facets', 'reviews', 'apps'] as const

/**
 * How old a stored entry may be and still be worth showing.
 *
 * A week. Long enough that an app opened offline after a fortnight's holiday
 * still shows something rather than nothing, short enough that a mod list from
 * months ago is not presented as the catalogue. It is replaced by the first
 * successful fetch either way, so this only ever decides what is on screen for
 * the second or two before that returns — or for as long as the device stays
 * offline, which is exactly the case this exists for.
 */
const MAX_AGE_MS = 7 * 24 * 60 * 60 * 1000

/**
 * Ceiling on the stored snapshot.
 *
 * `localStorage` is a few megabytes per origin and throws when full — a browse
 * of every game with images would otherwise fill it and start failing writes
 * for everything else. Entries are dropped oldest-first until it fits.
 */
const MAX_BYTES = 2 * 1024 * 1024

/** How long to wait after a change before writing. */
const WRITE_DELAY_MS = 2_000

type Entry = {
    key: unknown[]
    data: unknown
    updatedAt: number
}

function persistable(key: readonly unknown[]): boolean {
    const head = key[0]

    return (
        typeof head === 'string' && (PERSISTED as readonly string[]).includes(head)
    )
}

/**
 * Put the last session's responses back, then keep the store in step.
 *
 * Returns an unsubscribe function. Called once, from the app's root.
 */
export function attachOfflineCache(client: QueryClient): () => void {
    restore(client)

    let timer: ReturnType<typeof setTimeout> | null = null

    const schedule = () => {
        if (timer !== null) return

        timer = setTimeout(() => {
            timer = null
            save(client)
        }, WRITE_DELAY_MS)
    }

    /*
     * Debounced, and on a timer rather than on every event: a browse with
     * infinite scroll updates its cache entry on every page, and serialising
     * the whole store per page would be a JSON stringify of megabytes during a
     * scroll. The cost of the delay is at most one lost page on a hard kill.
     */
    const unsubscribe = client.getQueryCache().subscribe((event) => {
        /*
         * Any update schedules a write, without checking first whether THIS
         * query is one that gets stored. `save` filters anyway, and the check
         * here would only save the cost of a debounced no-op — while requiring
         * the event's key to be narrowed out of a union that types it loosely,
         * which is a cast this file would rather not have in it.
         */
        if (event.type === 'updated') schedule()
    })

    /*
     * A last write on the way out, because the debounce above may be pending.
     * `pagehide` rather than `unload`: `unload` never fires in a webview that
     * is closed by the OS, which on mobile is how an app usually ends.
     */
    const onHide = () => save(client)

    window.addEventListener('pagehide', onHide)

    return () => {
        unsubscribe()
        window.removeEventListener('pagehide', onHide)

        if (timer !== null) clearTimeout(timer)
    }
}

function restore(client: QueryClient): void {
    let entries: Entry[]

    try {
        const raw = window.localStorage.getItem(STORAGE_KEY)

        if (!raw) return

        const parsed: unknown = JSON.parse(raw)

        if (!Array.isArray(parsed)) return

        entries = parsed as Entry[]
    } catch {
        /*
         * Anything unreadable is DROPPED rather than repaired. This is a cache
         * of public data; every byte of it is one fetch away, and a launch that
         * fails because a cache file is corrupt is a far worse outcome than a
         * launch with an empty one.
         */
        clear()

        return
    }

    const cutoff = Date.now() - MAX_AGE_MS

    for (const entry of entries) {
        if (
            !entry ||
            !Array.isArray(entry.key) ||
            typeof entry.updatedAt !== 'number' ||
            entry.updatedAt < cutoff ||
            !persistable(entry.key)
        ) {
            continue
        }

        /*
         * The original timestamp, not `now`. React Query compares it against
         * `staleTime` to decide whether to refetch, so restoring these as fresh
         * would show week-old data and then never go and look for better.
         */
        client.setQueryData(entry.key, entry.data, {
            updatedAt: entry.updatedAt,
        })
    }
}

function save(client: QueryClient): void {
    const entries: Entry[] = []

    for (const query of client.getQueryCache().getAll()) {
        if (query.state.status !== 'success') continue
        if (query.state.data === undefined) continue
        if (!persistable(query.queryKey)) continue

        entries.push({
            key: [...query.queryKey],
            data: query.state.data,
            updatedAt: query.state.dataUpdatedAt,
        })
    }

    // Newest first, so the oldest are what falls off the end when trimming.
    entries.sort((a, b) => b.updatedAt - a.updatedAt)

    let payload = stringify(entries)

    while (payload !== null && payload.length > MAX_BYTES && entries.length > 0) {
        entries.pop()
        payload = stringify(entries)
    }

    if (payload === null) return

    try {
        window.localStorage.setItem(STORAGE_KEY, payload)
    } catch {
        /*
         * Out of quota, or a webview with storage disabled. Clearing rather
         * than retrying: a half-written or rejected snapshot is worth nothing,
         * and the next save starts from a store that has room.
         */
        clear()
    }
}

/** `JSON.stringify`, or null when the data holds something it cannot encode. */
function stringify(entries: Entry[]): string | null {
    try {
        return JSON.stringify(entries)
    } catch {
        return null
    }
}

function clear(): void {
    try {
        window.localStorage.removeItem(STORAGE_KEY)
    } catch {
        // A webview with storage disabled entirely. Nothing to undo.
    }
}
