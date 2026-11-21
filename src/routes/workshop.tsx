import { useEffect, useRef, useState } from 'react'
import { Link } from 'react-router-dom'
import {
    useInfiniteQuery,
    useMutation,
    useQuery,
    useQueryClient,
} from '@tanstack/react-query'
import { Button } from '@modcommunity/shared'

import { api } from '~/lib/api/client'
import type { WorkshopOwnedItemT } from '~/lib/api/contract'
import { useAuth } from '~/lib/auth/provider'
import { messageOf } from '~/lib/ipc'
import { ownedStatus, toggleId, unavailableLabel } from '~/lib/gamespice/workshop'
import Select from '~/components/select'
import { PackCard, PackCardSkeleton } from '~/components/workshop-card'
import { FiPackage, FiSearch, FiX } from 'react-icons/fi'

/**
 * **Workshop** — the whole GameSpice Workshop in the app: browse and search
 * every pack (Browse), manage what the member subscribed to (Subscribed: pause,
 * auto-update, which games it loads into) and the member's own packs (My packs:
 * edit, where it is offered, delete). Installing follows a subscription through
 * the library sync; uploading a new version is still on the website.
 */
type Tab = 'browse' | 'subscribed' | 'mine'

const TABS = [
    ['browse', 'Browse'],
    ['subscribed', 'Subscribed'],
    ['mine', 'My packs'],
] as const

const SORTS = [
    { value: 'popular', label: 'Most subscribed' },
    { value: 'new', label: 'Newest' },
    { value: 'updated', label: 'Recently updated' },
    { value: 'name', label: 'Name' },
] as const

type SortT = (typeof SORTS)[number]['value']

const GRID = 'grid grid-cols-[repeat(auto-fill,minmax(16rem,1fr))] gap-4'
const field = 'rounded-lg border border-border bg-surface px-2 py-1 text-sm'

function Alert({ error }: { error: unknown }) {
    return (
        <p role="alert" className="text-sm text-danger">
            {messageOf(error)}
        </p>
    )
}

export default function WorkshopRoute() {
    const { status, signIn } = useAuth()
    const [tab, setTab] = useState<Tab>('browse')
    const signedIn = status === 'signedIn'
    const resolving = status === 'loading'

    return (
        <div className="mx-auto flex w-full max-w-[96rem] flex-col gap-4 p-4">
            <header className="flex flex-wrap items-center gap-3">
                <h1 className="text-lg font-bold">Workshop</h1>
                <nav role="tablist" className="flex gap-1">
                    {TABS.map(([id, label]) => (
                        <Button
                            key={id}
                            role="tab"
                            aria-selected={tab === id}
                            btnType={tab === id ? 'primary' : 'secondary'}
                            onClick={() => setTab(id)}
                        >
                            {label}
                        </Button>
                    ))}
                </nav>
            </header>
            {tab === 'browse' && <BrowseTab signedIn={signedIn} />}
            {tab !== 'browse' && resolving && (
                <p role="status" className="text-sm text-muted">
                    Checking your account…
                </p>
            )}
            {tab !== 'browse' && !signedIn && !resolving && (
                <div className="flex flex-col items-start gap-3">
                    <p className="text-sm text-muted">
                        Sign in to manage your Workshop packs.
                    </p>
                    <Button btnType="primary" onClick={() => void signIn()}>
                        Sign in
                    </Button>
                </div>
            )}
            {tab === 'subscribed' && signedIn && <SubscribedTab />}
            {tab === 'mine' && signedIn && <MineTab />}
        </div>
    )
}

function useDebounced<T>(value: T, ms: number): T {
    const [v, setV] = useState(value)
    useEffect(() => {
        const t = setTimeout(() => setV(value), ms)

        return () => clearTimeout(t)
    }, [value, ms])

    return v
}

function BrowseTab({ signedIn }: { signedIn: boolean }) {
    const qc = useQueryClient()
    const [q, setQ] = useState('')
    const [kind, setKind] = useState('')
    const [game, setGame] = useState('')
    const [sort, setSort] = useState<SortT>('popular')
    // Optimistic subscribe state, by asset id; cleared when the catalog refetches.
    const [override, setOverride] = useState<Record<number, boolean>>({})
    const dq = useDebounced(q.trim(), 300)
    const filters = {
        q: dq || undefined,
        kind: kind || undefined,
        game: game || undefined,
        sort,
    }
    const cat = useInfiniteQuery({
        queryKey: ['gamespice', 'workshop-catalog', filters],
        initialPageParam: 1,
        queryFn: ({ pageParam }) =>
            api.workshopCatalog({ ...filters, page: pageParam, per: 24 }),
        getNextPageParam: (last) =>
            last.page * last.per < last.total ? last.page + 1 : undefined,
        staleTime: 30_000,
    })
    const sub = useMutation({
        mutationFn: (v: { id: number; on: boolean }) =>
            api.subscribe('asset', v.id, v.on),
        onMutate: (v) => setOverride((o) => ({ ...o, [v.id]: v.on })),
        onError: (_e, v) =>
            setOverride((o) => {
                const { [v.id]: _drop, ...rest } = o

                return rest
            }),
        onSuccess: () => qc.invalidateQueries({ queryKey: ['gamespice'] }),
    })
    const pages = cat.data?.pages ?? []
    const items = pages.flatMap((p) => p.items)
    const subscribed = new Set(pages.flatMap((p) => p.subscribed))
    const facets = pages[0]?.facets
    const total = pages[0]?.total ?? 0
    const filtered = Boolean(dq || kind || game)
    const pending = sub.isPending ? sub.variables?.id : undefined

    // Infinite scroll: a sentinel under the grid loads the next page when it nears the viewport.
    const sentinel = useRef<HTMLDivElement>(null)
    const { hasNextPage, isFetchingNextPage, fetchNextPage } = cat
    useEffect(() => {
        const el = sentinel.current
        if (!el || !hasNextPage || typeof IntersectionObserver === 'undefined')
            return
        const io = new IntersectionObserver(
            (es) => {
                if (es.some((e) => e.isIntersecting) && !isFetchingNextPage)
                    void fetchNextPage()
            },
            { rootMargin: '400px' }
        )
        io.observe(el)

        return () => io.disconnect()
    }, [hasNextPage, isFetchingNextPage, fetchNextPage])

    const chips: { label: string; clear: () => void }[] = [
        ...(dq ? [{ label: `"${dq}"`, clear: () => setQ('') }] : []),
        ...(kind ? [{ label: kind, clear: () => setKind('') }] : []),
        ...(game
            ? [
                  {
                      label:
                          facets?.games.find((g) => String(g.id) === game)?.name ??
                          'Game',
                      clear: () => setGame(''),
                  },
              ]
            : []),
    ]

    return (
        <section className="flex flex-col gap-3" aria-label="Browse the Workshop">
            <div className="flex flex-wrap items-end gap-2">
                <label className="relative min-w-48 flex-1">
                    <span className="sr-only">Search the Workshop</span>
                    <FiSearch
                        aria-hidden="true"
                        className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted"
                    />
                    <input
                        type="search"
                        placeholder="Search packs"
                        className={`${field} w-full py-1.5 pl-8 focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent`}
                        value={q}
                        onChange={(e) => setQ(e.target.value)}
                    />
                </label>
                <Select
                    label="Kind"
                    value={kind}
                    onChange={setKind}
                    options={[
                        { value: '', label: 'Any kind' },
                        ...(facets?.kinds ?? []).map((k) => ({
                            value: k,
                            label: k,
                        })),
                    ]}
                />
                <Select
                    label="Game"
                    value={game}
                    onChange={setGame}
                    options={[
                        { value: '', label: 'Any game' },
                        ...(facets?.games ?? []).map((g) => ({
                            value: String(g.id),
                            label: g.name,
                        })),
                    ]}
                />
                <Select
                    label="Sort"
                    value={sort}
                    onChange={setSort}
                    options={[...SORTS]}
                />
            </div>
            {chips.length > 0 && (
                <div
                    className="flex flex-wrap items-center gap-1.5"
                    aria-label="Active filters"
                >
                    {chips.map((c, i) => (
                        <button
                            key={`${i}-${c.label}`}
                            type="button"
                            onClick={c.clear}
                            aria-label={`Remove filter ${c.label}`}
                            className="inline-flex items-center gap-1 rounded-full border border-border bg-surface-2 px-2 py-0.5 text-xs hover:border-accent focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent"
                        >
                            {c.label} <FiX aria-hidden="true" />
                        </button>
                    ))}
                    <button
                        type="button"
                        className="text-xs text-muted underline"
                        onClick={() => {
                            setQ('')
                            setKind('')
                            setGame('')
                        }}
                    >
                        Clear all
                    </button>
                </div>
            )}
            {cat.data && (
                <p className="text-xs text-muted" aria-live="polite">
                    {total} {total === 1 ? 'pack' : 'packs'}
                </p>
            )}
            {sub.isError && <Alert error={sub.error} />}
            {cat.isError && (
                <div
                    role="alert"
                    className="flex flex-col items-start gap-2 rounded-xl border border-danger/40 bg-danger/10 p-4"
                >
                    <p className="text-sm text-danger">
                        Could not load the Workshop: {messageOf(cat.error)}
                    </p>
                    <Button btnType="secondary" onClick={() => void cat.refetch()}>
                        Try again
                    </Button>
                </div>
            )}
            {cat.data && items.length === 0 && (
                <div className="flex flex-col items-center gap-2 rounded-xl border border-dashed border-border p-10 text-center">
                    <FiPackage className="size-8 text-muted" aria-hidden="true" />
                    <p className="text-sm font-semibold">No packs match</p>
                    <p className="text-xs text-muted">
                        {filtered
                            ? 'Try a different search or clear the filters.'
                            : 'The Workshop is empty right now.'}
                    </p>
                </div>
            )}
            <ul className={GRID} aria-busy={cat.isFetching}>
                {cat.isPending &&
                    Array.from({ length: 8 }, (_, i) => (
                        <PackCardSkeleton key={i} />
                    ))}
                {items.map((it) => {
                    const on = override[it.id] ?? subscribed.has(it.id)

                    return (
                        <PackCard
                            key={it.id}
                            testId="workshop-item"
                            name={it.name}
                            href={`/view/asset/${it.id}`}
                            image={it.image}
                            summary={it.summary}
                            author={it.owner}
                            official={it.official}
                            kinds={it.kinds}
                            games={
                                it.compat === 'any'
                                    ? 'Every game'
                                    : it.games.map((g) => g.name).join(', ')
                            }
                            version={it.version}
                            bytes={it.bytes}
                            subscribers={it.subscribers}
                            runsCode={it.runsCode}
                            updatedAt={it.updatedAt}
                            status={on ? 'Subscribed' : null}
                            footer={
                                signedIn && (
                                    <Button
                                        btnType={on ? 'secondary' : 'primary'}
                                        className="w-full"
                                        disabled={pending === it.id}
                                        aria-pressed={on}
                                        aria-label={`${on ? 'Unsubscribe from' : 'Subscribe to'} ${it.name}`}
                                        onClick={() =>
                                            sub.mutate({ id: it.id, on: !on })
                                        }
                                    >
                                        {pending === it.id
                                            ? 'Saving…'
                                            : on
                                              ? 'Subscribed'
                                              : 'Subscribe'}
                                    </Button>
                                )
                            }
                        />
                    )
                })}
                {isFetchingNextPage &&
                    Array.from({ length: 3 }, (_, i) => (
                        <PackCardSkeleton key={`n${i}`} />
                    ))}
            </ul>
            <div ref={sentinel} className="flex justify-center">
                {hasNextPage && (
                    <Button
                        btnType="secondary"
                        disabled={isFetchingNextPage}
                        onClick={() => void fetchNextPage()}
                    >
                        {isFetchingNextPage ? 'Loading…' : 'Load more'}
                    </Button>
                )}
            </div>
        </section>
    )
}

type ScopeChange = { id: string } & (
    { all: boolean } | { game: string; on: boolean }
)

function SubscribedTab() {
    const qc = useQueryClient()
    const mine = useQuery({
        queryKey: ['gamespice', 'workshop'],
        queryFn: () => api.workshopMine(),
    })
    // The games that take subscriptions: the per-game picker's choices.
    const games = useQuery({
        queryKey: ['gamespice', 'workshop-catalog', 'facets'],
        queryFn: () => api.workshopCatalog({ per: 1 }),
        staleTime: 300_000,
    })
    const done = () => qc.invalidateQueries({ queryKey: ['gamespice'] })
    const scope = useMutation({
        mutationFn: ({ id, ...change }: ScopeChange) =>
            api.workshopScope(id, change),
        onSuccess: done,
    })
    const prefs = useMutation({
        mutationFn: (v: { id: number; paused?: boolean; autoUpdate?: boolean }) =>
            api.subscriptionPrefs('asset', v.id, {
                paused: v.paused,
                autoUpdate: v.autoUpdate,
            }),
        onSuccess: done,
    })
    const unsub = useMutation({
        mutationFn: (id: number) => api.subscribe('asset', id, false),
        onSuccess: done,
    })
    const busy = scope.isPending || prefs.isPending || unsub.isPending
    const failed = [
        mine.error,
        games.error,
        scope.error,
        prefs.error,
        unsub.error,
    ].filter((e) => e)

    return (
        <section className="flex flex-col gap-2" aria-label="Subscribed packs">
            <p className="text-xs text-muted">
                Subscribed packs install through your{' '}
                <Link to="/library" className="underline">
                    Library
                </Link>
                ; pausing takes a pack out of your games until you resume it.
            </p>
            {failed.map((e, i) => (
                <Alert key={i} error={e} />
            ))}
            {mine.isError && (
                <Button btnType="secondary" className="self-start" onClick={() => void mine.refetch()}>
                    Try again
                </Button>
            )}
            {mine.data?.items.length === 0 && (
                <p className="rounded-xl border border-dashed border-border p-8 text-center text-sm text-muted">
                    You have no Workshop subscriptions yet. Browse the Workshop and
                    subscribe to a pack.
                </p>
            )}
            <ul className={GRID}>
                {mine.isPending &&
                    Array.from({ length: 4 }, (_, i) => (
                        <PackCardSkeleton key={i} />
                    ))}
                {mine.data?.items.map((it) => {
                    const why = unavailableLabel(it)
                    const picked = new Set(it.games.map((g) => g.contentId))

                    return (
                        <PackCard
                            key={it.subscriptionId}
                            testId="workshop-sub"
                            name={it.name}
                            href={`/view/asset/${it.assetId}`}
                            image={it.image}
                            games={
                                it.scope === 'all'
                                    ? 'Every game'
                                    : `${it.games.length} ${it.games.length === 1 ? 'game' : 'games'}`
                            }
                            status={it.paused ? 'Paused' : null}
                            summary={
                                [it.via ? `via ${it.via.name}` : '', why ?? '']
                                    .filter(Boolean)
                                    .join(' · ') || null
                            }
                            footer={
                                <div className="flex flex-col gap-2">
                                    <div className="flex flex-wrap gap-2">
                                        <Button
                                            btnType="secondary"
                                            disabled={busy}
                                            onClick={() =>
                                                prefs.mutate({
                                                    id: it.assetId,
                                                    paused: !it.paused,
                                                })
                                            }
                                        >
                                            {it.paused ? 'Resume' : 'Pause'}
                                        </Button>
                                        <Button
                                            btnType="secondary"
                                            disabled={busy}
                                            onClick={() =>
                                                prefs.mutate({
                                                    id: it.assetId,
                                                    autoUpdate: !it.autoUpdate,
                                                })
                                            }
                                        >
                                            {it.autoUpdate
                                                ? 'Auto-update on'
                                                : 'Auto-update off'}
                                        </Button>
                                        <Button
                                            btnType="secondary"
                                            disabled={busy}
                                            onClick={() =>
                                                scope.mutate({
                                                    id: it.subscriptionId,
                                                    all: it.scope !== 'all',
                                                })
                                            }
                                        >
                                            {it.scope === 'all'
                                                ? 'Pick games'
                                                : 'Use in all games'}
                                        </Button>
                                        <Button
                                            btnType="secondary"
                                            disabled={busy || it.via !== null}
                                            onClick={() => unsub.mutate(it.assetId)}
                                        >
                                            Unsubscribe
                                        </Button>
                                    </div>
                                    {it.scope === 'listed' && (
                                        <fieldset
                                            className="flex flex-wrap gap-3 text-xs"
                                            aria-label={`Games for ${it.name}`}
                                        >
                                            {(games.data?.facets.games ?? []).map(
                                                (g) => (
                                                    <label
                                                        key={g.contentId}
                                                        className="flex items-center gap-1"
                                                    >
                                                        <input
                                                            type="checkbox"
                                                            checked={picked.has(
                                                                g.contentId
                                                            )}
                                                            disabled={busy}
                                                            onChange={(e) =>
                                                                scope.mutate({
                                                                    id: it.subscriptionId,
                                                                    game: g.contentId,
                                                                    on: e.target
                                                                        .checked,
                                                                })
                                                            }
                                                        />
                                                        {g.name}
                                                    </label>
                                                )
                                            )}
                                        </fieldset>
                                    )}
                                </div>
                            }
                        />
                    )
                })}
            </ul>
        </section>
    )
}

function MineTab() {
    const owned = useQuery({
        queryKey: ['gamespice', 'workshop-owned'],
        queryFn: () => api.workshopOwned(),
    })

    return (
        <section className="flex flex-col gap-2" aria-label="My packs">
            <p className="text-xs text-muted">
                Uploading a new pack or version is on the website for now; edit and
                list your packs here.
            </p>
            {owned.isError && (
                <>
                    <Alert error={owned.error} />
                    <Button btnType="secondary" className="self-start" onClick={() => void owned.refetch()}>
                        Try again
                    </Button>
                </>
            )}
            {owned.data?.items.length === 0 && (
                <p className="text-sm text-muted">
                    You have no packs that provide Workshop content.
                </p>
            )}
            <ul className={GRID}>
                {owned.isPending &&
                    Array.from({ length: 4 }, (_, i) => (
                        <PackCardSkeleton key={i} />
                    ))}
                {owned.data?.items.map((it) => (
                    <OwnedRow key={it.id} item={it} choices={owned.data.choices} />
                ))}
            </ul>
        </section>
    )
}

function OwnedRow({
    item,
    choices,
}: {
    item: WorkshopOwnedItemT
    choices: { id: number; contentId: string; name: string }[]
}) {
    const qc = useQueryClient()
    const [name, setName] = useState(item.name)
    const [summary, setSummary] = useState(item.summary ?? '')
    const [compat, setCompat] = useState(item.compat)
    const [games, setGames] = useState(item.games)
    const [confirm, setConfirm] = useState(false)
    const done = () => qc.invalidateQueries({ queryKey: ['gamespice'] })
    const save = useMutation({
        mutationFn: () =>
            api.workshopItemPatch({
                id: item.id,
                name: name.trim(),
                summary: summary.trim(),
                compat,
                ...(compat === 'listed' ? { games } : {}),
            }),
        onSuccess: done,
    })
    const del = useMutation({
        mutationFn: () => api.workshopItemDelete(item.id),
        onSuccess: done,
    })

    return (
        <PackCard
            testId="workshop-owned"
            name={item.name}
            href={`/view/asset/${item.id}`}
            image={item.image}
            summary={ownedStatus(item)}
            kinds={item.version?.kinds}
            version={item.version?.version}
            subscribers={item.subscribers}
            runsCode={item.version?.runsCode}
            status={item.hidden ? 'Hidden' : null}
            footer={
                <div className="flex flex-col gap-2">
                    <input
                        aria-label="Pack name"
                        className={field}
                        value={name}
                        maxLength={100}
                        onChange={(e) => setName(e.target.value)}
                    />
                    <textarea
                        aria-label="Pack summary"
                        className={field}
                        value={summary}
                        maxLength={500}
                        rows={2}
                        onChange={(e) => setSummary(e.target.value)}
                    />
                    <Select
                        label="Offered on"
                        value={compat}
                        onChange={setCompat}
                        options={[
                            { value: 'none', label: 'Not in the Workshop' },
                            {
                                value: 'any',
                                label: 'Every game',
                                disabled: !item.canList,
                            },
                            {
                                value: 'listed',
                                label: 'Games I pick',
                                disabled: !item.canList,
                            },
                        ]}
                    />
                    {compat === 'listed' && (
                        <fieldset
                            className="flex flex-wrap gap-3 text-xs"
                            aria-label={`Games for ${item.name}`}
                        >
                            {choices.map((g) => (
                                <label
                                    key={g.id}
                                    className="flex items-center gap-1"
                                >
                                    <input
                                        type="checkbox"
                                        checked={games.includes(g.id)}
                                        onChange={() =>
                                            setGames(toggleId(games, g.id))
                                        }
                                    />
                                    {g.name}
                                </label>
                            ))}
                        </fieldset>
                    )}
                    {save.isError && <Alert error={save.error} />}
                    {del.isError && <Alert error={del.error} />}
                    <div className="flex flex-wrap gap-2">
                        <Button
                            btnType="primary"
                            disabled={save.isPending || !name.trim()}
                            onClick={() => save.mutate()}
                        >
                            {save.isPending ? 'Saving…' : 'Save'}
                        </Button>
                        {save.isSuccess && !save.isPending && (
                            <span role="status" className="self-center text-xs text-success">
                                Saved
                            </span>
                        )}
                        {confirm ? (
                            <>
                                <Button
                                    btnType="danger"
                                    disabled={del.isPending}
                                    onClick={() => del.mutate()}
                                >
                                    Delete for good
                                </Button>
                                <Button
                                    btnType="secondary"
                                    onClick={() => setConfirm(false)}
                                >
                                    Keep it
                                </Button>
                            </>
                        ) : (
                            <Button
                                btnType="secondary"
                                onClick={() => setConfirm(true)}
                            >
                                Delete
                            </Button>
                        )}
                    </div>
                </div>
            }
        />
    )
}
