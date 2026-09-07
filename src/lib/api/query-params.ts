/**
 * The only place a browse URL's encoding is decoded.
 *
 * Every browse filter lives in the URL rather than in component state: a
 * filtered browse has to survive a reload, a deep link and the back button, and
 * the app's router is a HashRouter over a static bundle, so there is nowhere
 * else durable to put it.
 *
 * That makes this file small and load-bearing at the same time. A query string
 * carries no types — everything is a string — and the contract coerces per
 * field, so what these functions decide is which of two wrong answers a
 * hand-edited or stale URL produces: a dropped filter, or a request the API
 * rejects and which therefore takes the whole listing with it. They pick the
 * first, every time.
 *
 * Extracted from `routes/browse.tsx` so the encoding is testable rather than
 * closed over `useSearchParams`. The route wraps each in a `useCallback`; these
 * are pure.
 */

/**
 * A `1`-means-true flag.
 *
 * Only the literal `1`. Not `Boolean(raw)`, and not `raw !== null` — the URL is
 * shared and hand-edited, and `?empty=0` plainly means "no" to whoever typed
 * it, while both of those readings make it mean yes. Absent and false are the
 * same answer here (`undefined`) because the contract's own default is false
 * and sending it explicitly would only split the query cache.
 */
export function flag(search: URLSearchParams, key: string): true | undefined {
    return search.get(key) === '1' ? true : undefined
}

/**
 * A non-negative number, or nothing.
 *
 * A non-numeric value drops the filter rather than sending `NaN`, which the
 * contract rejects — taking the entire listing with it rather than one filter.
 */
export function num(search: URLSearchParams, key: string): number | undefined {
    const raw = search.get(key)

    if (raw === null || raw.trim() === '') return undefined

    const value = Number(raw)

    return Number.isFinite(value) && value >= 0 ? value : undefined
}

/**
 * A comma-separated list of positive ids.
 *
 * Every id in this app is a positive integer from a database sequence, so `0`
 * and negatives are as wrong as `abc` is. Bad entries are dropped individually
 * rather than failing the list: half a filter from a stale link is closer to
 * what the user meant than none of it, and the ids that survive are all real.
 *
 * An empty result is `undefined`, not `[]` — an empty array would be a filter
 * that matches nothing, which is the opposite of "no filter".
 */
export function idList(search: URLSearchParams, key: string): number[] | undefined {
    const raw = search.get(key)

    if (!raw) return undefined

    const out = raw
        .split(',')
        .map((value) => Number(value.trim()))
        .filter((value) => Number.isInteger(value) && value > 0)

    return out.length > 0 ? out : undefined
}

/** A plain string, or nothing. Empty is nothing. */
export function text(search: URLSearchParams, key: string): string | undefined {
    const raw = search.get(key)

    return raw === null || raw === '' ? undefined : raw
}

/**
 * One of a closed set of values, or nothing.
 *
 * For the fields whose values reach the API as an enum. A URL naming something
 * not in the set drops the filter — the alternative is a 400 that empties the
 * listing, and the user cannot tell that apart from "no results".
 */
export function oneOf<T extends string>(
    search: URLSearchParams,
    key: string,
    allowed: readonly T[]
): T | undefined {
    const raw = search.get(key)

    return raw !== null && (allowed as readonly string[]).includes(raw)
        ? (raw as T)
        : undefined
}

/**
 * How a multi-select is written back.
 *
 * The inverse of [`idList`], and here rather than in the route so the two
 * cannot drift into disagreeing about the separator.
 */
export function encodeIdList(ids: readonly number[]): string | null {
    const clean = ids.filter((id) => Number.isInteger(id) && id > 0)

    return clean.length > 0 ? clean.join(',') : null
}
