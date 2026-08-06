import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useInfiniteQuery, useQuery } from '@tanstack/react-query'
import { Link, useParams, useSearchParams } from 'react-router-dom'
import { FiFilter, FiSearch, FiX } from 'react-icons/fi'

import { api } from '~/lib/api/client'
import {
    BrowseSortVals,
    ContentKindSchema,
    type BrowseSortT,
    type ContentKindT,
    type ContentSummaryT,
} from '~/lib/api/contract'
import { useIsWide } from '~/lib/hooks/use-breakpoint'
import { useLiveQuery } from '~/lib/hooks/use-live-query'
import { useSettings } from '~/lib/settings/provider'
import ContentCard from '~/components/content-card'

/**
 * The browser, for every content kind.
 *
 * Infinite scroll rather than the website's numbered pages: a page selector is
 * a mouse affordance, and the app is as often a phone. Cursor paging (which the
 * API only offers) is also the only variant that does not duplicate rows at the
 * boundary while people are publishing.
 */

const KIND_LABELS: Record<ContentKindT, string> = {
    mod: 'Mods',
    asset: 'Assets',
    server: 'Servers',
    serverMap: 'Maps',
    article: 'Articles',
    community: 'Communities',
    collection: 'Collections',
    user: 'Members',
}

/** Which kinds each tab offers as sub-filters. */
const KIND_GROUPS: Record<string, ContentKindT[]> = {
    mod: ['mod'],
    asset: ['asset'],
    server: ['server', 'serverMap'],
    community: ['community', 'article', 'collection', 'user'],
}

const SORT_LABELS: Record<BrowseSortT, string> = {
    createdAt: 'Newest',
    lastEdit: 'Recently updated',
    name: 'Name',
    views: 'Most viewed',
    downloads: 'Most downloaded',
    rating: 'Highest rated',
    favorites: 'Most favourited',
    players: 'Most players',
}

/** Sorts that only make sense for a given kind. */
function sortsFor(kind: ContentKindT): BrowseSortT[] {
    return BrowseSortVals.filter((sort) => {
        if (sort === 'players') return kind === 'server'
        if (sort === 'downloads') return kind === 'mod' || kind === 'asset'

        return true
    })
}

export default function BrowseRoute() {
    const params = useParams<{ kind: string }>()
    const [search, setSearch] = useSearchParams()
    const wide = useIsWide()
    const { app } = useSettings()

    const kind = ContentKindSchema.catch('mod').parse(params.kind)
    const group = KIND_GROUPS[params.kind ?? ''] ?? [kind]

    const [query, setQuery] = useState(search.get('q') ?? '')
    const [showFilters, setShowFilters] = useState(false)

    const sort = (search.get('sort') as BrowseSortT | null) ?? 'createdAt'
    const appId = search.get('app')
    const onlineOnly = search.get('online') === '1'
    const byPing = kind === 'server' && search.get('ping') === '1'

    // `version` advances when a batch of measurements lands; the accessor
    // itself is stable, so it is the version the sort below keys on.
    const { get: liveResult, version: liveVersion } = useLiveQuery()

    const latencyOf = useCallback(
        (item: ContentSummaryT): number | null =>
            liveResult(`${item.kind}:${item.id}`)?.outcome.result?.rttMs ?? null,
        [liveResult]
    )

    /*
     * Debounced, because every keystroke would otherwise be a query with its
     * own cursor chain. 300ms is the usual sweet spot: fast enough to feel
     * live, slow enough that a typed word is one request.
     */
    const [debounced, setDebounced] = useState(query)

    useEffect(() => {
        const timer = window.setTimeout(() => setDebounced(query), 300)

        return () => window.clearTimeout(timer)
    }, [query])

    const filters = useMemo(
        () => ({
            kind,
            search: debounced || undefined,
            apps: appId ? [Number(appId)] : undefined,
            sort,
            sortDir: 'desc' as const,
            nsfw: app?.showNsfw ?? false,
            onlineOnly: kind === 'server' ? onlineOnly : undefined,
            limit: 30,
        }),
        [kind, debounced, appId, sort, app?.showNsfw, onlineOnly]
    )

    const listing = useInfiniteQuery({
        queryKey: ['browse', filters],
        initialPageParam: null as string | null,
        queryFn: ({ pageParam }) =>
            api.browse({ ...filters, cursor: pageParam ?? undefined }),
        getNextPageParam: (last) => last.nextCursor,
        staleTime: 60 * 1000,
    })

    const facets = useQuery({
        queryKey: ['facets', kind],
        queryFn: () => api.facets(kind),
        staleTime: 10 * 60 * 1000,
    })

    const setParam = useCallback(
        (key: string, value: string | null) => {
            const next = new URLSearchParams(search)

            if (value === null) next.delete(key)
            else next.set(key, value)

            setSearch(next, { replace: true })
        },
        [search, setSearch]
    )

    // Sentinel-driven paging, so the list loads before the user hits the end.
    const sentinel = useRef<HTMLDivElement>(null)

    useEffect(() => {
        const node = sentinel.current

        if (!node) return

        const observer = new IntersectionObserver(
            (entries) => {
                if (
                    entries.some((e) => e.isIntersecting) &&
                    listing.hasNextPage &&
                    !listing.isFetchingNextPage
                ) {
                    void listing.fetchNextPage()
                }
            },
            { rootMargin: '600px' }
        )

        observer.observe(node)

        return () => observer.disconnect()
    }, [listing])

    const fetched = listing.data?.pages.flatMap((page) => page.items) ?? []
    const total = listing.data?.pages[0]?.total ?? null

    /*
     * "Sort by ping" is CLIENT-side and only it could be.
     *
     * Latency is a fact about this device's route to a server, so the API has
     * no way to order by it — a Sydney player and a Frankfurt one want opposite
     * orderings of the same list. It therefore reorders what has already been
     * fetched and measured, exactly like an in-game server browser, and rows
     * with no measurement yet sink to the bottom rather than being hidden.
     */
    const items = useMemo(() => {
        if (!byPing) return fetched

        return [...fetched].sort((a, b) => {
            const left = latencyOf(a)
            const right = latencyOf(b)

            if (left === right) return 0
            if (left == null) return 1
            if (right == null) return -1

            return left - right
        })
        // `fetched` is a fresh array each render; its CONTENT is what matters,
        // and that changes only when a page loads or a measurement lands.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [byPing, fetched.length, listing.dataUpdatedAt, liveVersion, latencyOf])

    const filterPanel = (
        <div className="flex flex-col gap-4 text-sm">
            <div className="flex flex-col gap-1.5">
                <span className="text-xs font-medium uppercase tracking-wide text-muted">
                    Sort
                </span>
                <select
                    value={sort}
                    onChange={(e) => setParam('sort', e.target.value)}
                    className="rounded-lg border border-border bg-surface px-2 py-1.5"
                >
                    {sortsFor(kind).map((value) => (
                        <option key={value} value={value}>
                            {SORT_LABELS[value]}
                        </option>
                    ))}
                </select>
            </div>

            {kind === 'server' && (
                <>
                    <label className="flex items-center gap-2">
                        <input
                            type="checkbox"
                            checked={onlineOnly}
                            onChange={(e) =>
                                setParam('online', e.target.checked ? '1' : null)
                            }
                        />
                        Online only
                    </label>

                    <label className="flex flex-col gap-1">
                        <span className="flex items-center gap-2">
                            <input
                                type="checkbox"
                                checked={byPing}
                                onChange={(e) =>
                                    setParam('ping', e.target.checked ? '1' : null)
                                }
                            />
                            Sort by ping
                        </span>
                        <span className="pl-6 text-xs text-muted">
                            Orders the servers already measured on this device.
                        </span>
                    </label>
                </>
            )}

            {(facets.data?.apps.length ?? 0) > 0 && (
                <div className="flex flex-col gap-1.5">
                    <span className="text-xs font-medium uppercase tracking-wide text-muted">
                        Game
                    </span>
                    <select
                        value={appId ?? ''}
                        onChange={(e) => setParam('app', e.target.value || null)}
                        className="rounded-lg border border-border bg-surface px-2 py-1.5"
                    >
                        <option value="">All games</option>
                        {facets.data?.apps.map((game) => (
                            <option key={game.id} value={game.id}>
                                {game.name} ({game.count})
                            </option>
                        ))}
                    </select>
                </div>
            )}
        </div>
    )

    return (
        <div className="flex h-full flex-col">
            <header className="flex flex-col gap-3 border-b border-border bg-surface px-4 py-3">
                <div className="flex items-center gap-2">
                    <div className="relative min-w-0 flex-1">
                        <FiSearch className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted" />
                        <input
                            value={query}
                            onChange={(e) => {
                                setQuery(e.target.value)
                                setParam('q', e.target.value || null)
                            }}
                            placeholder={`Search ${KIND_LABELS[kind].toLowerCase()}…`}
                            className="w-full rounded-lg border border-border bg-background py-2 pl-9 pr-8 text-sm outline-none focus:border-accent"
                        />
                        {query && (
                            <button
                                type="button"
                                onClick={() => {
                                    setQuery('')
                                    setParam('q', null)
                                }}
                                aria-label="Clear search"
                                className="absolute right-2 top-1/2 -translate-y-1/2 text-muted"
                            >
                                <FiX className="size-4" />
                            </button>
                        )}
                    </div>

                    {!wide && (
                        <button
                            type="button"
                            onClick={() => setShowFilters((open) => !open)}
                            aria-label="Filters"
                            className="rounded-lg border border-border p-2 text-muted"
                        >
                            <FiFilter className="size-4" />
                        </button>
                    )}
                </div>

                {group.length > 1 && (
                    <div className="flex gap-1 overflow-x-auto">
                        {group.map((value) => (
                            <Link
                                key={value}
                                to={`/browse/${value}`}
                                className={`shrink-0 rounded-full px-3 py-1 text-xs transition-colors ${
                                    value === kind
                                        ? 'bg-accent text-accent-foreground'
                                        : 'bg-surface-secondary text-muted'
                                }`}
                            >
                                {KIND_LABELS[value]}
                            </Link>
                        ))}
                    </div>
                )}

                {!wide && showFilters && filterPanel}
            </header>

            <div className="flex min-h-0 flex-1">
                {wide && (
                    <aside className="w-60 shrink-0 overflow-y-auto border-r border-border p-4">
                        {filterPanel}
                    </aside>
                )}

                <div className="min-w-0 flex-1 overflow-y-auto p-4">
                    {total !== null && (
                        <p className="mb-3 text-xs text-muted">
                            {total.toLocaleString()} {KIND_LABELS[kind].toLowerCase()}
                        </p>
                    )}

                    {listing.isError && (
                        <p className="rounded-lg border border-danger p-4 text-sm text-danger">
                            {(listing.error).message}
                        </p>
                    )}

                    {listing.isPending ? (
                        <SkeletonGrid />
                    ) : items.length === 0 ? (
                        <p className="py-12 text-center text-sm text-muted">
                            Nothing matched.
                        </p>
                    ) : (
                        <div className="grid grid-cols-1 gap-3 xs:grid-cols-2 lg:grid-cols-3 2xl:grid-cols-4">
                            {items.map((item) => (
                                <ContentCard
                                    key={`${item.kind}-${item.id}`}
                                    item={item}
                                />
                            ))}
                        </div>
                    )}

                    <div ref={sentinel} className="h-8" />

                    {listing.isFetchingNextPage && (
                        <p className="py-4 text-center text-xs text-muted">
                            Loading more…
                        </p>
                    )}
                </div>
            </div>
        </div>
    )
}

function SkeletonGrid() {
    return (
        <div className="grid grid-cols-1 gap-3 xs:grid-cols-2 lg:grid-cols-3 2xl:grid-cols-4">
            {Array.from({ length: 8 }, (_, i) => (
                <div
                    key={i}
                    className="h-56 animate-pulse rounded-xl border border-border bg-surface-secondary"
                />
            ))}
        </div>
    )
}
