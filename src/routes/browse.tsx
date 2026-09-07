import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useInfiniteQuery, useQuery } from '@tanstack/react-query'
import { Link, useParams, useSearchParams } from 'react-router-dom'
import { FiFilter, FiGrid, FiList, FiSearch, FiX } from 'react-icons/fi'

import { api } from '~/lib/api/client'
import {
    BrowseSortVals,
    ContentKindSchema,
    type BrowseSortT,
    type ContentKindT,
    type ContentSummaryT,
} from '~/lib/api/contract'
import { useAuth } from '~/lib/auth/provider'
import { useIsWide } from '~/lib/hooks/use-breakpoint'
import { useLiveQuery } from '~/lib/hooks/use-live-query'
import { useSettings } from '~/lib/settings/provider'
import {
    flag as readFlag,
    idList as readIdList,
    num as readNum,
} from '~/lib/api/query-params'
import BrowseFilters from '~/components/browse-filters'
import Select from '~/components/select'
import ContentCard from '~/components/content-card'
import ServerTable from '~/components/server-table'

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
    reviews: 'Most reviewed',
    favorites: 'Most favourited',

    curUsers: 'Most players',
    maxUsers: 'Most slots',
    avgUsers: 'Busiest on average',
    bots: 'Most bots',
    map: 'Map name',
    lastOnline: 'Recently online',
    lastScanned: 'Recently scanned',

    // The contract keeps this as a deprecated alias of `curUsers` so shipped
    // builds do not 400. Never offered in the dropdown.
    players: 'Most players',
}

/** Sorts the API can actually order this kind by. */
const SERVER_ONLY: BrowseSortT[] = [
    'curUsers',
    'maxUsers',
    'avgUsers',
    'bots',
    'map',
    'lastOnline',
    'lastScanned',
]

/**
 * The value the sort dropdown should show for a given wire value.
 *
 * `players` and `curUsers` are the same ordering under two spellings, and the
 * server default sends the older one so it works against any deployed server.
 * Without this the dropdown could not find `players` among its options and fell
 * back to displaying the first one — so the browser said "Newest" while it was
 * very obviously sorted by player count.
 */
function displaySort(sort: BrowseSortT): BrowseSortT {
    return sort === 'players' ? 'curUsers' : sort
}

function sortsFor(kind: ContentKindT): BrowseSortT[] {
    return BrowseSortVals.filter((sort) => {
        // The deprecated alias is honoured on the wire, never offered.
        if (sort === 'players') return false

        if (SERVER_ONLY.includes(sort)) return kind === 'server'
        if (sort === 'downloads') return kind === 'mod' || kind === 'asset'

        // `Collection` has no ratings relation, so the API falls this back to
        // "newest" — offering it would be a control that silently does nothing.
        if (sort === 'rating') return kind !== 'collection'

        return true
    })
}

export default function BrowseRoute() {
    const params = useParams<{ kind: string }>()
    const [search, setSearch] = useSearchParams()
    const wide = useIsWide()
    const { app } = useSettings()
    const { status } = useAuth()

    const kind = ContentKindSchema.catch('mod').parse(params.kind)
    const group = KIND_GROUPS[params.kind ?? ''] ?? [kind]

    const [query, setQuery] = useState(search.get('q') ?? '')
    const [showFilters, setShowFilters] = useState(false)

    /*
     * Per-kind defaults, copied from the website's own
     * (`lib/user/settings/default.ts`).
     *
     * A server browser sorted by "newest" is the single worst default this app
     * had: the catalogue holds 2.6 million servers, most freshly imported and
     * never once seen online, so the first screen was a wall of dead rows all
     * reading `TO`. The website sorts by `curUsers` desc and pins
     * `onlineOnly` — "the busiest servers I can actually join" — and that is
     * what a server browser is FOR.
     *
     * `onlineOnly` is a default, not a pin: the Server group can turn it off.
     *
     * THE DEFAULT DELIBERATELY SENDS `players`, NOT `curUsers`.
     *
     * They are the same ordering — the contract keeps `players` as a deprecated
     * alias and maps both to `Server.curUsers`. The difference is that a server
     * older than this build has never heard of `curUsers` and rejects the whole
     * query, which blanks the browser on the app's most important screen. The
     * alias is understood by every deployed version, so the DEFAULT view keeps
     * working across a deploy window in either direction. The newer sorts are
     * opt-in clicks, where a failure is traceable to the click that caused it.
     */
    const defaultSort: BrowseSortT = kind === 'server' ? 'players' : 'createdAt'

    const sort = (search.get('sort') as BrowseSortT | null) ?? defaultSort
    const sortDir: 'asc' | 'desc' = search.get('dir') === 'asc' ? 'asc' : 'desc'
    const byPing = kind === 'server' && search.get('ping') === '1'

    /*
     * The URL is the single source of truth for every filter.
     *
     * Not component state: a filtered browse has to survive a reload, a deep
     * link and the back button, and the app's router is a HashRouter over a
     * static bundle — there is nowhere else durable to put it. The readers
     * below are the only place the URL's string encoding is decoded.
     */
    const param = useCallback((key: string) => search.get(key), [search])

    const flag = useCallback((key: string) => readFlag(search, key), [search])

    const num = useCallback((key: string) => readNum(search, key), [search])

    const idList = useCallback((key: string) => readIdList(search, key), [search])

    /*
     * Servers default to the TABLE, everything else to the grid.
     *
     * A server is a row of comparable numbers — ping, players, map — and the
     * question the browser answers about it is "which of these should I join?",
     * which is a comparison. A mod's card answers "what is this?", which is a
     * picture and a sentence. `?view=` overrides either way, so the choice
     * survives a reload and can be shared.
     */
    const canTable = kind === 'server'
    const view: 'grid' | 'table' =
        canTable && search.get('view') !== 'grid' ? 'table' : 'grid'

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

    const isServer = kind === 'server'

    /*
     * The URL, decoded into the contract's shape.
     *
     * Server-only filters are sent as `undefined` for every other kind rather
     * than dropped from the object: the API ignores what a kind cannot express,
     * but leaving a stale `hideEmpty` in the query key would split the cache
     * between two listings that are actually identical.
     */
    const filters = useMemo(
        () => ({
            kind,
            search: debounced || undefined,
            apps: idList('app'),
            categories: idList('cats'),
            tags: idList('tags'),
            tagsOr: flag('tagsOr'),
            environment:
                kind === 'mod' || kind === 'asset'
                    ? ((param('env') as 'SERVER' | 'CLIENT' | null) ?? undefined)
                    : undefined,

            sort,
            sortDir,

            // NSFW is an APP setting, not a browse filter — it is a standing
            // preference about this install, not something to re-pick per
            // search. See the settings split in CLAUDE.md.
            nsfw: app?.showNsfw ?? false,
            archived: flag('archived'),
            mine: flag('mine'),

            // Default ON for servers, matching the website. `online=0` in the
            // URL is how the checkbox turns it off, so the default and the
            // explicit choice stay distinguishable.
            onlineOnly: isServer ? search.get('online') !== '0' : undefined,
            wasOnline: isServer ? flag('wasOnline') : undefined,
            password: isServer ? flag('password') : undefined,
            secure: isServer ? flag('secure') : undefined,
            isOfficial: isServer ? flag('official') : undefined,
            os: isServer
                ? ((param('os') as 'WINDOWS' | 'LINUX' | 'MAC' | null) ?? undefined)
                : undefined,
            mapName: isServer ? (param('map') ?? undefined) : undefined,
            countries: isServer ? idList('countries') : undefined,
            hideEmpty: isServer ? flag('empty') : undefined,
            hideFull: isServer ? flag('full') : undefined,
            minUsers: isServer ? num('minUsers') : undefined,
            maxUsers: isServer ? num('maxUsers') : undefined,
            minSlots: isServer ? num('minSlots') : undefined,
            maxSlots: isServer ? num('maxSlots') : undefined,

            limit: 30,
        }),
        [
            kind,
            isServer,
            debounced,
            sort,
            sortDir,
            search,
            app?.showNsfw,
            param,
            flag,
            num,
            idList,
        ]
    )

    /** Filters currently narrowing the listing, for the panel's Clear button. */
    const activeCount = useMemo(() => {
        const keys = [
            'app',
            'cats',
            'tags',
            'tagsOr',
            'env',
            'archived',
            'mine',
            'online',
            'wasOnline',
            'password',
            'secure',
            'official',
            'os',
            'map',
            'countries',
            'empty',
            'full',
            'minUsers',
            'maxUsers',
            'minSlots',
            'maxSlots',
        ]

        return keys.filter((k) => {
            const v = search.get(k)

            return v !== null && v !== ''
        }).length
    }, [search])

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

    /**
     * Drop every filter, keeping the things that are not filters.
     *
     * The search text, the sort, the grid/table choice and the ping ordering
     * all survive: they describe how the user is LOOKING at the list, not which
     * rows it contains, and clearing them would undo work nobody asked to undo.
     */
    const clearFilters = useCallback(() => {
        const next = new URLSearchParams()

        for (const key of ['q', 'sort', 'view', 'ping']) {
            const value = search.get(key)

            if (value !== null) next.set(key, value)
        }

        setSearch(next, { replace: true })
    }, [search, setSearch])

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
        <div className="flex flex-col gap-3 text-sm">
            <div className="flex flex-col gap-1.5">
                <span className="text-xs font-medium uppercase tracking-wide text-muted">
                    Sort
                </span>
                <Select
                    label="Sort"
                    fullWidth
                    value={displaySort(sort)}
                    onChange={(next) => setParam('sort', next)}
                    options={sortsFor(kind).map((value) => ({
                        value,
                        label: SORT_LABELS[value],
                    }))}
                />
            </div>

            {/* Sort by ping sits with Sort, not with the filters: it reorders
                what is already on screen rather than changing what the API
                returns, and it is the only control here that never refetches. */}
            {isServer && (
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
            )}

            <BrowseFilters
                kind={kind}
                get={param}
                set={setParam}
                facets={facets.data}
                signedIn={status === 'signedIn'}
                activeCount={activeCount}
                onClear={clearFilters}
            />
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

                    {canTable && (
                        <div className="flex shrink-0 overflow-hidden rounded-lg border border-border">
                            {(
                                [
                                    ['table', FiList, 'Table'],
                                    ['grid', FiGrid, 'Cards'],
                                ] as const
                            ).map(([value, Icon, label]) => (
                                <button
                                    key={value}
                                    type="button"
                                    onClick={() => setParam('view', value)}
                                    aria-label={label}
                                    aria-pressed={view === value}
                                    title={label}
                                    className={`p-2 transition-colors ${
                                        view === value
                                            ? 'bg-accent text-accent-foreground'
                                            : 'text-muted hover:bg-surface-hover'
                                    }`}
                                >
                                    <Icon className="size-4" />
                                </button>
                            ))}
                        </div>
                    )}

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
                            {total.toLocaleString()}{' '}
                            {KIND_LABELS[kind].toLowerCase()}
                        </p>
                    )}

                    {listing.isError && (
                        <p className="rounded-lg border border-danger p-4 text-sm text-danger">
                            {listing.error.message}
                        </p>
                    )}

                    {listing.isPending ? (
                        <SkeletonGrid />
                    ) : items.length === 0 ? (
                        <p className="py-12 text-center text-sm text-muted">
                            Nothing matched.
                        </p>
                    ) : view === 'table' ? (
                        <ServerTable
                            items={items}
                            sort={sort}
                            sortDir={sortDir}
                            onSort={(key, dir) => {
                                const next = new URLSearchParams(search)

                                next.set('sort', key)
                                next.set('dir', dir)

                                setSearch(next, { replace: true })
                            }}
                        />
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
