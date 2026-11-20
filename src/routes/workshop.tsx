import { useState } from 'react'
import { Link } from 'react-router-dom'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Button } from '@modcommunity/shared'

import { api } from '~/lib/api/client'
import type { WorkshopOwnedItemT } from '~/lib/api/contract'
import { useAuth } from '~/lib/auth/provider'
import { messageOf } from '~/lib/ipc'
import { ownedStatus, packSize, toggleId, unavailableLabel } from '~/lib/gamespice/workshop'
import Select from '~/components/select'

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

    return (
        <div className="flex flex-col gap-4 p-4">
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
            {tab !== 'browse' && !signedIn && (
                <div className="flex flex-col items-start gap-3">
                    <p className="text-sm text-muted">Sign in to manage your Workshop packs.</p>
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

function BrowseTab({ signedIn }: { signedIn: boolean }) {
    const qc = useQueryClient()
    const [q, setQ] = useState('')
    const [kind, setKind] = useState('')
    const [game, setGame] = useState('')
    const [sort, setSort] = useState<SortT>('popular')
    const [page, setPage] = useState(1)
    const query = {
        q: q || undefined,
        kind: kind || undefined,
        game: game || undefined,
        sort,
        page,
        per: 24,
    }
    const cat = useQuery({
        queryKey: ['gamespice', 'workshop-catalog', query],
        queryFn: () => api.workshopCatalog(query),
        staleTime: 30_000,
    })
    const sub = useMutation({
        mutationFn: (v: { id: number; on: boolean }) => api.subscribe('asset', v.id, v.on),
        onSuccess: () => qc.invalidateQueries({ queryKey: ['gamespice'] }),
    })
    const subscribed = new Set(cat.data?.subscribed ?? [])
    const facets = cat.data?.facets
    const pages = cat.data ? Math.max(1, Math.ceil(cat.data.total / cat.data.per)) : 1
    const reset = <T,>(set: (v: T) => void) => (v: T) => {
        set(v)
        setPage(1)
    }

    return (
        <section className="flex flex-col gap-3" aria-label="Browse the Workshop">
            <div className="flex flex-wrap items-end gap-2">
                <input
                    aria-label="Search the Workshop"
                    placeholder="Search packs"
                    className={field}
                    value={q}
                    onChange={(e) => reset(setQ)(e.target.value)}
                />
                <Select
                    label="Kind"
                    value={kind}
                    onChange={reset(setKind)}
                    options={[
                        { value: '', label: 'Any kind' },
                        ...(facets?.kinds ?? []).map((k) => ({ value: k, label: k })),
                    ]}
                />
                <Select
                    label="Game"
                    value={game}
                    onChange={reset(setGame)}
                    options={[
                        { value: '', label: 'Any game' },
                        ...(facets?.games ?? []).map((g) => ({ value: String(g.id), label: g.name })),
                    ]}
                />
                <Select label="Sort" value={sort} onChange={reset(setSort)} options={[...SORTS]} />
            </div>
            {cat.isPending && <p className="text-sm text-muted">Loading…</p>}
            {cat.isError && <Alert error={cat.error} />}
            {sub.isError && <Alert error={sub.error} />}
            {cat.data?.items.length === 0 && <p className="text-sm text-muted">No packs match.</p>}
            <ul className="grid grid-cols-1 gap-3 xs:grid-cols-2 lg:grid-cols-3">
                {cat.data?.items.map((it) => {
                    const on = subscribed.has(it.id)

                    return (
                        <li
                            key={it.id}
                            data-testid="workshop-item"
                            className="flex flex-col gap-2 rounded-xl border border-border bg-surface p-3"
                        >
                            {it.image && (
                                <img src={it.image} alt="" className="aspect-video w-full rounded-lg object-cover" />
                            )}
                            <Link to={`/view/asset/${it.id}`} className="truncate text-sm font-semibold">
                                {it.name}
                            </Link>
                            {it.summary && <p className="line-clamp-2 text-xs text-muted">{it.summary}</p>}
                            <p className="text-[11px] text-muted">
                                {it.owner?.name ?? 'Unknown'} · v{it.version} · {packSize(it.bytes)} ·{' '}
                                {it.subscribers} subscribed
                                {it.compat === 'any'
                                    ? ' · every game'
                                    : ` · ${it.games.map((g) => g.name).join(', ')}`}
                                {it.runsCode ? ' · runs code' : ''}
                            </p>
                            {signedIn && (
                                <Button
                                    btnType={on ? 'secondary' : 'primary'}
                                    disabled={sub.isPending}
                                    onClick={() => sub.mutate({ id: it.id, on: !on })}
                                >
                                    {on ? 'Unsubscribe' : 'Subscribe'}
                                </Button>
                            )}
                        </li>
                    )
                })}
            </ul>
            {pages > 1 && (
                <div className="flex items-center gap-2 text-sm">
                    <Button btnType="secondary" disabled={page <= 1} onClick={() => setPage(page - 1)}>
                        Previous
                    </Button>
                    <span>
                        Page {page} of {pages}
                    </span>
                    <Button btnType="secondary" disabled={page >= pages} onClick={() => setPage(page + 1)}>
                        Next
                    </Button>
                </div>
            )}
        </section>
    )
}

type ScopeChange = { id: string } & ({ all: boolean } | { game: string; on: boolean })

function SubscribedTab() {
    const qc = useQueryClient()
    const mine = useQuery({ queryKey: ['gamespice', 'workshop'], queryFn: () => api.workshopMine() })
    // The games that take subscriptions: the per-game picker's choices.
    const games = useQuery({
        queryKey: ['gamespice', 'workshop-catalog', 'facets'],
        queryFn: () => api.workshopCatalog({ per: 1 }),
        staleTime: 300_000,
    })
    const done = () => qc.invalidateQueries({ queryKey: ['gamespice'] })
    const scope = useMutation({
        mutationFn: ({ id, ...change }: ScopeChange) => api.workshopScope(id, change),
        onSuccess: done,
    })
    const prefs = useMutation({
        mutationFn: (v: { id: number; paused?: boolean; autoUpdate?: boolean }) =>
            api.subscriptionPrefs('asset', v.id, { paused: v.paused, autoUpdate: v.autoUpdate }),
        onSuccess: done,
    })
    const unsub = useMutation({
        mutationFn: (id: number) => api.subscribe('asset', id, false),
        onSuccess: done,
    })
    const busy = scope.isPending || prefs.isPending || unsub.isPending
    const failed = [mine.error, games.error, scope.error, prefs.error, unsub.error].filter((e) => e)

    return (
        <section className="flex flex-col gap-2" aria-label="Subscribed packs">
            <p className="text-xs text-muted">
                Subscribed packs install through your{' '}
                <Link to="/library" className="underline">
                    Library
                </Link>
                ; pausing takes a pack out of your games until you resume it.
            </p>
            {mine.isPending && <p className="text-sm text-muted">Loading…</p>}
            {failed.map((e, i) => (
                <Alert key={i} error={e} />
            ))}
            {mine.data?.items.length === 0 && (
                <p className="text-sm text-muted">You have no Workshop subscriptions yet.</p>
            )}
            <ul className="flex flex-col gap-2">
                {mine.data?.items.map((it) => {
                    const why = unavailableLabel(it)
                    const picked = new Set(it.games.map((g) => g.contentId))

                    return (
                        <li
                            key={it.subscriptionId}
                            data-testid="workshop-sub"
                            className="flex flex-col gap-2 rounded-xl border border-border bg-surface p-3"
                        >
                            <div className="flex items-center gap-3">
                                {it.image ? (
                                    <img src={it.image} alt="" className="size-12 rounded-lg object-cover" />
                                ) : (
                                    <div className="size-12 rounded-lg bg-surface-2" />
                                )}
                                <div className="min-w-0 flex-1">
                                    <Link
                                        to={`/view/asset/${it.assetId}`}
                                        className="block truncate text-sm font-semibold"
                                    >
                                        {it.name}
                                    </Link>
                                    <p className="text-[11px] text-muted">
                                        {it.scope === 'all' ? 'Applies to every game' : 'Only the games you picked'}
                                        {it.via ? ` · via ${it.via.name}` : ''}
                                        {it.paused ? ' · paused' : ''}
                                    </p>
                                    {why && <p className="text-[11px] text-danger">{why}</p>}
                                </div>
                            </div>
                            <div className="flex flex-wrap gap-2">
                                <Button
                                    btnType="secondary"
                                    disabled={busy}
                                    onClick={() => prefs.mutate({ id: it.assetId, paused: !it.paused })}
                                >
                                    {it.paused ? 'Resume' : 'Pause'}
                                </Button>
                                <Button
                                    btnType="secondary"
                                    disabled={busy}
                                    onClick={() => prefs.mutate({ id: it.assetId, autoUpdate: !it.autoUpdate })}
                                >
                                    {it.autoUpdate ? 'Auto-update on' : 'Auto-update off'}
                                </Button>
                                <Button
                                    btnType="secondary"
                                    disabled={busy}
                                    onClick={() => scope.mutate({ id: it.subscriptionId, all: it.scope !== 'all' })}
                                >
                                    {it.scope === 'all' ? 'Pick games' : 'Use in all games'}
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
                                <fieldset className="flex flex-wrap gap-3 text-xs" aria-label={`Games for ${it.name}`}>
                                    {(games.data?.facets.games ?? []).map((g) => (
                                        <label key={g.contentId} className="flex items-center gap-1">
                                            <input
                                                type="checkbox"
                                                checked={picked.has(g.contentId)}
                                                disabled={busy}
                                                onChange={(e) =>
                                                    scope.mutate({
                                                        id: it.subscriptionId,
                                                        game: g.contentId,
                                                        on: e.target.checked,
                                                    })
                                                }
                                            />
                                            {g.name}
                                        </label>
                                    ))}
                                </fieldset>
                            )}
                        </li>
                    )
                })}
            </ul>
        </section>
    )
}

function MineTab() {
    const owned = useQuery({ queryKey: ['gamespice', 'workshop-owned'], queryFn: () => api.workshopOwned() })

    return (
        <section className="flex flex-col gap-2" aria-label="My packs">
            <p className="text-xs text-muted">
                Uploading a new pack or version is on the website for now; edit and list your packs here.
            </p>
            {owned.isPending && <p className="text-sm text-muted">Loading…</p>}
            {owned.isError && <Alert error={owned.error} />}
            {owned.data?.items.length === 0 && (
                <p className="text-sm text-muted">You have no packs that provide Workshop content.</p>
            )}
            <ul className="flex flex-col gap-2">
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
    const del = useMutation({ mutationFn: () => api.workshopItemDelete(item.id), onSuccess: done })

    return (
        <li
            data-testid="workshop-owned"
            className="flex flex-col gap-2 rounded-xl border border-border bg-surface p-3"
        >
            <p className="text-[11px] text-muted">
                {ownedStatus(item)} · {item.subscribers} subscribed
                {item.version ? ` · v${item.version.version}` : ''}
            </p>
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
                    { value: 'any', label: 'Every game', disabled: !item.canList },
                    { value: 'listed', label: 'Games I pick', disabled: !item.canList },
                ]}
            />
            {compat === 'listed' && (
                <fieldset className="flex flex-wrap gap-3 text-xs" aria-label={`Games for ${item.name}`}>
                    {choices.map((g) => (
                        <label key={g.id} className="flex items-center gap-1">
                            <input
                                type="checkbox"
                                checked={games.includes(g.id)}
                                onChange={() => setGames(toggleId(games, g.id))}
                            />
                            {g.name}
                        </label>
                    ))}
                </fieldset>
            )}
            {save.isError && <Alert error={save.error} />}
            {del.isError && <Alert error={del.error} />}
            <div className="flex flex-wrap gap-2">
                <Button btnType="primary" disabled={save.isPending || !name.trim()} onClick={() => save.mutate()}>
                    Save
                </Button>
                {confirm ? (
                    <>
                        <Button btnType="danger" disabled={del.isPending} onClick={() => del.mutate()}>
                            Delete for good
                        </Button>
                        <Button btnType="secondary" onClick={() => setConfirm(false)}>
                            Keep it
                        </Button>
                    </>
                ) : (
                    <Button btnType="secondary" onClick={() => setConfirm(true)}>
                        Delete
                    </Button>
                )}
            </div>
        </li>
    )
}
