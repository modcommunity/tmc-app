import { useMemo, useState } from 'react'
import { useInfiniteQuery } from '@tanstack/react-query'
import { Link, useSearchParams } from 'react-router-dom'
import {
    FiBox,
    FiCpu,
    FiGlobe,
    FiLayers,
    FiPackage,
    FiPlay,
    FiSearch,
    FiServer,
    FiStar,
    FiUsers,
    FiX,
} from 'react-icons/fi'

import { api } from '~/lib/api/client'
import { messageOf } from '~/lib/ipc'
import {
    AppSortVals,
    AppTypeVals,
    type AppSortT,
    type AppSummaryT,
    type AppTypeT,
} from '~/lib/api/contract'
import { GameIcon } from '~/components/game-icon'
import Select from '~/components/select'
import PlayDialog, { type PlayTargetT } from '~/components/play-dialog'

/**
 * **Apps** — every game, engine and platform TMC tracks, and what can be done
 * with each one from this device.
 *
 * The website has no page like this and structurally cannot: its chrome is
 * built around one chosen game, so "what is there?" is a question it answers by
 * making you choose first. The app opens on a flat grid across the whole
 * catalogue, which is why the endpoint behind this exists at all.
 *
 * Each card answers three questions in the order somebody asks them: what is
 * this, how much is there for it, and can I start it right now. The third is
 * the reason the tab is not just a filter on the browse screen — a Play button
 * is a thing this device can do and the website's copy of the same list cannot.
 *
 * WHAT THE CARD DOES NOT DECIDE
 * ----------------------------
 * Whether a launch is permitted. `directPlay` gates the SERVERLESS case only,
 * and `/play/launch` enforces it however the button was drawn — so a Play
 * button that should not be here fails loudly rather than starting something.
 */

const TYPE_LABELS: Record<AppTypeT, string> = {
    GAME: 'Game',
    GAME_SINGLEPLAYER: 'Singleplayer',
    GAME_ENGINE: 'Engine',
    VOIP: 'Voice',
    OTHER: 'Other',
}

const SORT_LABELS: Record<AppSortT, string> = {
    players: 'Players online',
    servers: 'Servers',
    content: 'Mods and assets',
    name: 'Name',
}

function compact(n: number): string {
    if (n < 1000) return String(n)
    if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`

    return `${(n / 1_000_000).toFixed(1)}m`
}

function Stat({
    icon: Icon,
    value,
    label,
}: {
    icon: typeof FiServer
    value: number
    label: string
}) {
    if (value <= 0) return null

    return (
        <span className="flex items-center gap-1" title={`${value} ${label}`}>
            <Icon className="size-3" />
            {compact(value)}
        </span>
    )
}

function AppCard({
    app,
    onPlay,
}: {
    app: AppSummaryT
    onPlay: (target: PlayTargetT) => void
}) {
    const art = app.images.card ?? app.images.banner

    /*
     * A Play button on the APP is the serverless launch, which is exactly what
     * `directPlay` governs. A game that is playable only INTO a server gets no
     * button here and its Servers link instead — which is true, and is where
     * the launch actually lives.
     */
    const canPlay = app.play?.directPlay === true

    return (
        <article className="flex flex-col overflow-hidden rounded-xl border border-border bg-surface">
            <div className="relative aspect-[16/7] w-full overflow-hidden bg-surface-2">
                {art ? (
                    <img
                        src={art}
                        alt=""
                        aria-hidden
                        loading="lazy"
                        className="size-full object-cover"
                    />
                ) : null}

                {app.isOfficial && (
                    <span className="absolute right-2 top-2 flex items-center gap-1 rounded-full bg-black/60 px-2 py-0.5 text-[10px] font-medium text-white backdrop-blur">
                        <FiStar className="size-2.5" />
                        Official
                    </span>
                )}
            </div>

            <div className="flex flex-1 flex-col gap-2 p-3">
                <div className="flex items-start gap-2.5">
                    <GameIcon
                        app={{ id: app.id, name: app.name, icon: app.images.icon }}
                        size="md"
                    />

                    <div className="min-w-0 flex-1">
                        <h3 className="truncate text-sm font-semibold">
                            {app.name}
                        </h3>
                        <p className="truncate text-[11px] text-muted">
                            {TYPE_LABELS[app.type]}
                            {app.engine ? ` · ${app.engine.name}` : ''}
                        </p>
                    </div>
                </div>

                {app.description && (
                    <p className="line-clamp-2 text-[11px] text-muted">
                        {app.description}
                    </p>
                )}

                <div className="mt-auto flex flex-wrap items-center gap-3 text-[11px] text-muted">
                    <Stat icon={FiPackage} value={app.counts.mods} label="mods" />
                    <Stat icon={FiBox} value={app.counts.assets} label="assets" />
                    <Stat
                        icon={FiServer}
                        value={app.counts.servers}
                        label="servers"
                    />
                    <Stat
                        icon={FiUsers}
                        value={app.counts.players}
                        label="players online"
                    />
                </div>

                <div className="flex flex-wrap items-center gap-1.5 pt-1">
                    {canPlay && (
                        <button
                            type="button"
                            onClick={() => onPlay({ appId: app.id, app })}
                            className="flex items-center gap-1.5 rounded-lg bg-accent px-2.5 py-1 text-[11px] font-semibold text-accent-foreground"
                        >
                            <FiPlay className="size-3" />
                            Play
                        </button>
                    )}

                    {app.counts.mods > 0 && (
                        <Link
                            to={`/browse/mod?apps=${app.id}`}
                            className="rounded-lg border border-border px-2.5 py-1 text-[11px]"
                        >
                            Mods
                        </Link>
                    )}

                    {app.counts.assets > 0 && (
                        <Link
                            to={`/browse/asset?apps=${app.id}`}
                            className="rounded-lg border border-border px-2.5 py-1 text-[11px]"
                        >
                            Assets
                        </Link>
                    )}

                    {app.hasServers && app.counts.servers > 0 && (
                        <Link
                            to={`/browse/server?apps=${app.id}`}
                            className="rounded-lg border border-border px-2.5 py-1 text-[11px]"
                        >
                            Servers
                        </Link>
                    )}

                    {/* Says what the app can do for this game, not what the
                        game is. A user scanning for something to install reads
                        this before they read the counts. */}
                    {app.integrations.includes('TMC_APP_MANAGE') && (
                        <span
                            className="ml-auto flex items-center gap-1 text-[10px] text-success"
                            title="Mods and assets for this game can be installed and kept updated from here."
                        >
                            <FiLayers className="size-2.5" />
                            Manageable
                        </span>
                    )}
                </div>
            </div>
        </article>
    )
}

export default function AppsRoute() {
    const [params, setParams] = useSearchParams()
    const [target, setTarget] = useState<PlayTargetT | null>(null)

    const search = params.get('q') ?? ''
    const type = (params.get('type') as AppTypeT | null) ?? null
    const sort = (params.get('sort') as AppSortT | null) ?? 'players'
    const playable = params.get('playable') === '1'
    const managed = params.get('managed') === '1'

    /*
     * Every filter lives in the URL, never in component state — the same rule
     * the browse screen follows and for the same reasons: a filtered view has
     * to survive a reload and the back button, and a HashRouter over a static
     * bundle has nowhere else durable to put it.
     */
    const set = (key: string, value: string | null) => {
        const next = new URLSearchParams(params)

        if (value === null || value === '') next.delete(key)
        else next.set(key, value)

        setParams(next, { replace: true })
    }

    const filters = useMemo(
        () => ({
            search: search || undefined,
            type: type ?? undefined,
            sort,
            playable: playable || undefined,
            managed: managed || undefined,
            limit: 40,
        }),
        [search, type, sort, playable, managed]
    )

    const listing = useInfiniteQuery({
        queryKey: ['apps', filters],
        initialPageParam: null as string | null,
        queryFn: ({ pageParam }) =>
            api.apps({ ...filters, cursor: pageParam ?? undefined }),
        getNextPageParam: (last) => last.nextCursor,
        // The catalogue changes on the scale of weeks; the counts on it change
        // by the minute, which is not worth a refetch on a screen somebody
        // opens to pick a game.
        staleTime: 5 * 60 * 1000,
    })

    const apps = useMemo(
        () => listing.data?.pages.flatMap((p) => p.apps) ?? [],
        [listing.data]
    )

    /*
     * The play centre being switched off entirely, which is a different fact
     * from "no game here is playable". Read from the first page because every
     * page carries it and the first is the one that has arrived.
     */
    const playDisabled = listing.data?.pages[0]?.playEnabled === false

    return (
        <div className="flex h-full flex-col">
            <header className="flex flex-col gap-3 border-b border-border p-4">
                <div className="flex items-center gap-2">
                    <div className="relative flex-1">
                        <FiSearch className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted" />
                        <input
                            type="search"
                            value={search}
                            onChange={(e) => set('q', e.target.value)}
                            placeholder="Search games and apps"
                            className="w-full rounded-lg border border-border bg-surface-2 py-1.5 pl-8 pr-8 text-xs"
                        />
                        {search && (
                            <button
                                type="button"
                                onClick={() => set('q', null)}
                                aria-label="Clear the search"
                                className="absolute right-2 top-1/2 -translate-y-1/2 text-muted"
                            >
                                <FiX className="size-3.5" />
                            </button>
                        )}
                    </div>

                    <Select
                        label="Sort"
                        value={sort}
                        options={AppSortVals.map((v) => ({
                            value: v,
                            label: SORT_LABELS[v],
                        }))}
                        onChange={(next) => set('sort', next)}
                    />
                </div>

                <div className="flex flex-wrap items-center gap-1.5">
                    <Chip
                        active={type === null}
                        onClick={() => set('type', null)}
                        label="All"
                    />
                    {AppTypeVals.map((value) => (
                        <Chip
                            key={value}
                            active={type === value}
                            onClick={() => set('type', value)}
                            label={TYPE_LABELS[value]}
                        />
                    ))}

                    <span className="mx-1 h-4 w-px bg-border" />

                    <Chip
                        active={playable}
                        onClick={() => set('playable', playable ? null : '1')}
                        label="Playable"
                        icon={FiGlobe}
                    />
                    <Chip
                        active={managed}
                        onClick={() => set('managed', managed ? null : '1')}
                        label="Moddable here"
                        icon={FiCpu}
                    />
                </div>

                {playDisabled && (
                    <p className="rounded-lg border border-border bg-surface-2 px-3 py-2 text-[11px] text-muted">
                        Playing games from the app is switched off site-wide at the
                        moment, so no Play buttons are shown. Everything else on
                        this screen still works.
                    </p>
                )}
            </header>

            <div className="flex-1 overflow-y-auto p-4">
                {listing.isPending ? (
                    <p className="text-xs text-muted">Loading the catalogue…</p>
                ) : listing.isError ? (
                    /*
                     * The reason, not just the fact. This screen is the front
                     * door, so its failure is the first thing anybody sees —
                     * and "could not be loaded" on its own is indistinguishable
                     * between an offline laptop, an old build talking to a
                     * newer site and an endpoint the site has not deployed. The
                     * last of those is what it actually was, and it cost a
                     * round trip through a bug report to find out.
                     */
                    <div className="space-y-1">
                        <p className="text-xs text-danger">
                            The catalogue could not be loaded.
                        </p>
                        <p className="text-[11px] text-muted">
                            {messageOf(listing.error)}
                        </p>
                        <button
                            type="button"
                            onClick={() => void listing.refetch()}
                            className="text-[11px] text-accent underline underline-offset-2"
                        >
                            Try again
                        </button>
                    </div>
                ) : apps.length < 1 ? (
                    <p className="text-xs text-muted">
                        Nothing matched. Try a broader filter.
                    </p>
                ) : (
                    <>
                        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
                            {apps.map((app) => (
                                <AppCard
                                    key={app.id}
                                    app={app}
                                    onPlay={setTarget}
                                />
                            ))}
                        </div>

                        {listing.hasNextPage && (
                            <div className="mt-4 flex justify-center">
                                <button
                                    type="button"
                                    disabled={listing.isFetchingNextPage}
                                    onClick={() => void listing.fetchNextPage()}
                                    className="rounded-lg border border-border px-4 py-2 text-xs disabled:opacity-50"
                                >
                                    {listing.isFetchingNextPage
                                        ? 'Loading…'
                                        : 'Show more'}
                                </button>
                            </div>
                        )}
                    </>
                )}
            </div>

            {target && (
                <PlayDialog target={target} onClose={() => setTarget(null)} />
            )}
        </div>
    )
}

function Chip({
    active,
    onClick,
    label,
    icon: Icon,
}: {
    active: boolean
    onClick: () => void
    label: string
    icon?: typeof FiGlobe
}) {
    return (
        <button
            type="button"
            onClick={onClick}
            aria-pressed={active}
            className={`flex items-center gap-1 rounded-full border px-2.5 py-1 text-[11px] transition ${
                active
                    ? 'border-accent bg-accent/10 text-accent'
                    : 'border-border text-muted'
            }`}
        >
            {Icon && <Icon className="size-2.5" />}
            {label}
        </button>
    )
}
