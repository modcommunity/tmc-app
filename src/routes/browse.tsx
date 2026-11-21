import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useInfiniteQuery, useQuery } from '@tanstack/react-query'
import { Link, useParams, useSearchParams } from 'react-router-dom'
import { FiFilter, FiGrid, FiList, FiSearch, FiStar, FiX } from 'react-icons/fi'

import { api } from '~/lib/api/client'
import {
    BrowseSortVals,
    ContentKindSchema,
    type BrowseSortT,
    type ContentKindT,
    type ContentSummaryT,
    type CommunityAgeT,
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
import {
    filterFavorites,
    refreshStoredFavorites,
    useFavorites,
} from '~/lib/favorites'
import { activeChips, removeChip } from '~/lib/api/browse-chips'
import { appLabel } from '~/lib/api/labels'
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

/** The count line's noun when there is exactly one: "1 mod", not "1 mods". */
const KIND_ONE: Record<ContentKindT, string> = {
    mod: 'mod',
    asset: 'asset',
    server: 'server',
    serverMap: 'map',
    article: 'article',
    community: 'community',
    collection: 'collection',
    user: 'member',
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

/** The tab's kinds, or just the one when the URL names no tab. */
function groupFor(param: string | undefined, kind: ContentKindT) {
    return KIND_GROUPS[param ?? ''] ?? [kind]
}

export default function BrowseRoute() {
    const params = useParams<{ kind: string }>()
    const [search, setSearch] = useSearchParams()
    const wide = useIsWide()
    const { app } = useSettings()
    const { status } = useAuth()

    const kind = useMemo(
        () => ContentKindSchema.catch('mod').parse(params.kind),
        [params.kind]
    )
    const group = groupFor(params.kind, kind)

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
    const favOnly = kind === 'server' && search.get('fav') === '1'
    const favorites = useFavorites()
    const favCount = Object.keys(favorites).length
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
            // search. See the settings split in docs/settings-and-logging.md.
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
            adultAges: isServer
                ? (param('ages')
                      ?.split(',')
                      .filter((v): v is CommunityAgeT =>
                          ['NONE', 'ADULT_18', 'ADULT_21', 'ADULT_25'].includes(v)
                      ) ?? undefined)
                : undefined,
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
            'ages',
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
        enabled: !favOnly,
        queryKey: ['browse', filters],
        initialPageParam: null as string | null,
        queryFn: ({ pageParam }) =>
            api.browse({ ...filters, cursor: pageParam ?? undefined }),
        getNextPageParam: (last) => last.nextCursor,
        staleTime: 60 * 1000,
    })

    /*
     * Favourites are a local snapshot (the app API has no favourites endpoint
     * and `/browse` cannot filter by id), so opening the view re-reads each
     * favourite from the item endpoint and writes the fresh row back. A failed
     * read leaves the old snapshot, which is stale rather than missing.
     */
    const favIds = Object.keys(favorites).slice(0, 100).join(',')

    useQuery({
        enabled: favOnly && favIds !== '',
        queryKey: ['favorites-refresh', favIds],
        staleTime: 60 * 1000,
        queryFn: async () => {
            const settled = await Promise.allSettled(
                favIds.split(',').map((id) => api.content('server', id))
            )

            refreshStoredFavorites(
                settled.flatMap((r) => (r.status === 'fulfilled' ? [r.value.summary] : []))
            )

            return true
        },
    })

    // Counts follow the list's own online switch, so a number never promises
    // rows the list hides.
    const onlineOnly = search.get('online') !== '0' && !favOnly

    const facets = useQuery({
        queryKey: ['facets', kind, onlineOnly],
        queryFn: () => api.facets(kind, onlineOnly),
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

        for (const key of ['q', 'sort', 'view', 'ping', 'fav']) {
            const value = search.get(key)

            if (value !== null) next.set(key, value)
        }

        setSearch(next, { replace: true })
    }, [search, setSearch])

    /*
     * Sentinel-driven paging, so the list loads before the user hits the end.
     *
     * Two things made this stall when scrolling fast:
     *   - the observer's root was the viewport, so the 600px margin could not
     *     reach past the page's own scroll container and the next page was
     *     only requested once the sentinel was literally on screen;
     *   - an observer only reports CHANGES, so after a page landed with the
     *     sentinel still near the edge (or after a failed fetch) nothing ever
     *     asked again.
     * So: observe against the real scroll container with a generous margin,
     * and also re-check on every scroll event and after each page settles.
     */
    const sentinel = useRef<HTMLDivElement>(null)
    const [pageError, setPageError] = useState(false)

    useEffect(() => {
        const node = sentinel.current

        if (!node) return

        let root: HTMLElement | null = node.parentElement

        while (root) {
            const oy = getComputedStyle(root).overflowY

            if (oy === 'auto' || oy === 'scroll') break

            root = root.parentElement
        }

        const near = () => {
            const box = node.getBoundingClientRect()
            const edge = root
                ? root.getBoundingClientRect().bottom
                : window.innerHeight

            return box.top < edge + 1500
        }

        const check = () => {
            if (pageError || !near()) return
            if (!listing.hasNextPage || listing.isFetchingNextPage) return

            listing.fetchNextPage().then(
                (r) => setPageError(r.isError),
                () => setPageError(true)
            )
        }

        const observer = new IntersectionObserver(check, {
            root,
            rootMargin: '1500px',
        })

        observer.observe(node)
        ;(root ?? window).addEventListener('scroll', check, { passive: true })
        check()

        return () => {
            observer.disconnect()
            ;(root ?? window).removeEventListener('scroll', check)
        }
    }, [listing, pageError])

    const favRows = useMemo(
        () =>
            favOnly
                ? filterFavorites(Object.values(favorites).reverse(), {
                      search: debounced || undefined,
                      appId: idList('app')?.[0],
                      onlineOnly: search.get('online') === '1',
                      hideEmpty: flag('empty'),
                      hideFull: flag('full'),
                      isOfficial: flag('official'),
                  })
                : [],
        [favOnly, favorites, debounced, idList, flag, search]
    )
    const fetched = favOnly
        ? favRows
        : (listing.data?.pages.flatMap((page) => page.items) ?? [])
    const total = favOnly ? favRows.length : (listing.data?.pages[0]?.total ?? null)

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

    const chips = useMemo(
        () =>
            activeChips(
                search,
                {
                    apps: new Map(
                        (facets.data?.apps ?? []).map((a) => [String(a.id), appLabel(a)])
                    ),
                    countries: new Map(
                        (facets.data?.countries ?? []).map((c) => [String(c.id), c.name])
                    ),
                },
                { favorites: favOnly }
            ),
        [search, facets.data, favOnly]
    )

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
                favorites={favOnly}
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

                {isServer && (
                    <div
                        role="tablist"
                        aria-label="Server list"
                        className="flex w-fit overflow-hidden rounded-lg border border-border text-xs"
                    >
                        {(
                            [
                                ['all', 'All servers'],
                                ['fav', 'Favourites'],
                            ] as const
                        ).map(([value, label]) => {
                            const selected = (value === 'fav') === favOnly

                            return (
                                <button
                                    key={value}
                                    type="button"
                                    role="tab"
                                    aria-selected={selected}
                                    onClick={() =>
                                        setParam('fav', value === 'fav' ? '1' : null)
                                    }
                                    className={`flex items-center gap-1.5 px-3 py-1.5 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent ${
                                        selected
                                            ? 'bg-accent text-accent-foreground'
                                            : 'text-muted hover:bg-surface-hover'
                                    }`}
                                >
                                    {value === 'fav' && <FiStar className="size-3" />}
                                    {label}
                                    {value === 'fav' && favCount > 0 && (
                                        <span className="tabular-nums opacity-80">
                                            {favCount}
                                        </span>
                                    )}
                                </button>
                            )
                        })}
                    </div>
                )}

                {isServer && chips.length > 0 && (
                    <ul className="flex flex-wrap items-center gap-1.5" aria-label="Active filters">
                        {chips.map((chip) => (
                            <li key={chip.keys.join('|')}>
                                <button
                                    type="button"
                                    onClick={() =>
                                        setSearch(removeChip(search, chip), { replace: true })
                                    }
                                    aria-label={`Remove filter ${chip.label}`}
                                    className="flex items-center gap-1 rounded-full bg-surface-secondary px-2.5 py-1 text-xs text-foreground transition-colors hover:bg-surface-hover focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent"
                                >
                                    {chip.label}
                                    <FiX className="size-3 text-muted" />
                                </button>
                            </li>
                        ))}
                    </ul>
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
                            {total === 1
                                ? KIND_ONE[kind]
                                : KIND_LABELS[kind].toLowerCase()}
                        </p>
                    )}

                    {listing.isError && (
                        <p className="rounded-lg border border-danger p-4 text-sm text-danger">
                            {listing.error.message}
                        </p>
                    )}

                    {!favOnly && listing.isPending ? (
                        <SkeletonGrid />
                    ) : items.length === 0 ? (
                        <div className="flex flex-col items-center gap-3 py-16 text-center text-sm text-muted">
                            {favOnly && favCount === 0 ? (
                                <>
                                    <FiStar className="size-8" aria-hidden />
                                    <p>
                                        No favourites yet. Star a server in the list to keep
                                        it here.
                                    </p>
                                    <button
                                        type="button"
                                        onClick={() => setParam('fav', null)}
                                        className="rounded-lg bg-accent px-3 py-1.5 text-xs font-medium text-accent-foreground hover:opacity-90"
                                    >
                                        Browse servers
                                    </button>
                                </>
                            ) : (
                                <>
                                    <FiSearch className="size-8" aria-hidden />
                                    <p>Nothing matched.</p>
                                    {(activeCount > 0 || query) && (
                                        <button
                                            type="button"
                                            onClick={clearFilters}
                                            className="rounded-lg border border-border px-3 py-1.5 text-xs text-foreground hover:bg-surface-hover"
                                        >
                                            Clear filters
                                        </button>
                                    )}
                                </>
                            )}
                        </div>
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

                    {pageError && listing.hasNextPage && (
                        <div className="flex justify-center py-4">
                            <button
                                type="button"
                                onClick={() => {
                                    setPageError(false)
                                    void listing.fetchNextPage()
                                }}
                                className="rounded-md border border-border px-3 py-1 text-xs hover:bg-surface-hover"
                            >
                                Couldn&apos;t load more — retry
                            </button>
                        </div>
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
