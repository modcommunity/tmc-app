import { useCallback, useMemo, useState } from 'react'
import { useInfiniteQuery, useMutation, useQuery } from '@tanstack/react-query'
import { Link, useSearchParams } from 'react-router-dom'
import {
    FiAlertTriangle,
    FiCompass,
    FiGlobe,
    FiPower,
    FiSearch,
    FiServer,
    FiUsers,
    FiX,
} from 'react-icons/fi'

import { api } from '~/lib/api/client'
import { messageOf } from '~/lib/ipc'
import { useAuth } from '~/lib/auth/provider'
import { useAppIcons } from '~/lib/hooks/use-app-icons'
import type { PlayModeT } from '~/lib/api/contract'
import {
    MODE_LABELS,
    SORT_LABELS,
    TAB_LABELS,
    TAB_SORTS,
    compact,
    filtersFromParams,
    filtersToParams,
    isNarrowed,
    pageQuery,
    resolveSort,
    type PlayFilters,
    type PlayTab,
} from '~/lib/api/play'
import Select from '~/components/select'
import PlayDialog, { type PlayTargetT } from '~/components/play-dialog'
import { InviteRow } from '~/routes/friends'
import {
    BTN_GHOST,
    BTN_PRIMARY,
    GameCard,
    LobbyRow,
    PartyBar,
    QuickMatchDialog,
    ServerRow,
    type JoinTarget,
    type PlayAppRowT,
} from '~/components/play-center'

/**
 * **Play** — the app's play center, the counterpart of the website's `/play`.
 *
 * Four tabs over one endpoint (`GET /play/page`): the games you can start, the
 * servers you can join right now, the lobbies people are gathering, and, once
 * signed in, the invitations addressed to you. The filters live in the URL for
 * the reason the browse screen's do.
 *
 * WHAT THIS SCREEN DOES NOT DECIDE
 * --------------------------------
 * Whether anything can be launched. It asks `/play/playable` which games have a
 * Play button and draws accordingly, and the launcher it opens (`PlayDialog`)
 * asks `/play/launch`, which enforces the same rules. Quick play only ranks;
 * pressing Join opens the launcher on the chosen server, so how the game starts
 * (window, installed copy, connect link) stays the player's.
 *
 * Skipped on purpose: the site's 3D avatar rail. It needs a renderer this
 * webview does not carry, and a party member's face is an initial here.
 */

const MODE_ANY = 'any'

type Tone = 'games' | 'servers' | 'lobbies' | 'invites'

export default function PlayRoute() {
    const [params, setParams] = useSearchParams()
    const { status } = useAuth()
    const signedIn = status === 'signedIn'

    const decoded = useMemo(() => filtersFromParams(params), [params])
    // The invites tab is the viewer's own: signed out, a link to it opens games.
    const filters: PlayFilters = useMemo(
        () => (decoded.tab === 'invites' && !signedIn ? { ...decoded, tab: 'games' } : decoded),
        [decoded, signedIn]
    )
    const { tab } = filters

    const update = useCallback(
        (patch: Partial<PlayFilters>) => {
            // Changing tab keeps the search and the game but not a sort the new
            // tab cannot express, and never carries another tab's switch over.
            const next: PlayFilters = { ...filters, ...patch }

            if (patch.tab && patch.tab !== filters.tab) {
                next.sort = undefined
                next.populated = false
                next.joinable = false
            }

            setParams(filtersToParams(next), { replace: true })
        },
        [filters, setParams]
    )

    // ------------------------------------------------------------ Data
    const listing = useInfiniteQuery({
        queryKey: ['play', 'page', filters],
        initialPageParam: 0,
        queryFn: ({ pageParam }) => api.playPage(pageQuery(filters, pageParam)),
        getNextPageParam: (last) => last?.nextCursor ?? undefined,
        staleTime: 15_000,
        // Servers and lobbies are live; games change by the hour.
        refetchInterval: tab === 'servers' || tab === 'lobbies' ? 30_000 : false,
        enabled: tab !== 'invites',
    })

    const playable = useQuery({
        queryKey: ['play', 'playable'],
        queryFn: api.playPlayable,
        staleTime: 5 * 60_000,
    })

    const invites = useQuery({
        queryKey: ['social', 'invites'],
        queryFn: api.partyInvites,
        enabled: signedIn,
        refetchInterval: 20_000,
    })

    // The first page carries the shared facts; every page carries its own rows.
    const pages = listing.data?.pages
    const first = pages?.[0]
    const off = pages !== undefined && first === null
    // GameSpice's own games lead; the sort is stable, so the server's order
    // (and the user's chosen sort) still decides within each half.
    const apps = useMemo(() => {
        const all = pages?.flatMap((p) => p?.apps ?? []) ?? []

        return [...all].sort((a, b) => Number(b.isOfficial) - Number(a.isOfficial))
    }, [pages])
    const servers = useMemo(() => pages?.flatMap((p) => p?.servers ?? []) ?? [], [pages])
    const lobbies = useMemo(() => pages?.flatMap((p) => p?.lobbies ?? []) ?? [], [pages])

    const listed = useMemo(
        () => (playable.data?.enabled ? new Set(playable.data.appIds) : null),
        [playable.data]
    )

    // Rows name their game but carry no artwork; the games tab does. One map,
    // filled from whatever has been loaded, so a server row mostly gets its
    // icon without a request. GameIcon falls back to initials when it misses.
    const icons = useMemo(() => {
        const map = new Map<number, string | null>()

        for (const a of apps) map.set(a.id, a.iconUrl)

        return map
    }, [apps])
    const facetIcon = useAppIcons()
    const iconOf = (id: number) => icons.get(id) ?? facetIcon(id)

    // ------------------------------------------------------------ Launching
    const [target, setTarget] = useState<PlayTargetT | null>(null)
    const [quick, setQuick] = useState<{ id: number; name: string } | null>(null)

    const join = useMutation({
        mutationFn: async (t: JoinTarget) => {
            // The launcher wants the server's full row (its connect link, which
            // platforms its owner closed). The page row is deliberately thin.
            const detail = await api.content('server', String(t.serverId))

            return { t, summary: detail.summary }
        },
        onSuccess: ({ t, summary }) => {
            setQuick(null)
            setTarget({
                appId: t.appId,
                server: summary,
                fallback: { name: t.appName, icon: iconOf(t.appId), slug: null },
            })
        },
    })

    const openGame = (app: PlayAppRowT) =>
        setTarget({
            appId: app.id,
            fallback: { name: app.name, icon: app.iconUrl, slug: null },
        })

    const focusGame = (app: PlayAppRowT) => update({ tab: 'servers', appId: app.id, q: '' })

    // ------------------------------------------------------------ Render
    const sorts = TAB_SORTS[tab]
    const narrowed = isNarrowed(filters)
    const inviteCount = invites.data?.length ?? 0
    const totals = first?.totals

    const tabs: PlayTab[] = signedIn
        ? ['games', 'servers', 'lobbies', 'invites']
        : ['games', 'servers', 'lobbies']

    const gameFilterName =
        filters.appId !== undefined
            ? (servers[0]?.appName ?? lobbies[0]?.appName ?? `Game ${filters.appId}`)
            : null

    const rows = tab === 'games' ? apps.length : tab === 'servers' ? servers.length : lobbies.length

    return (
        <div className="flex h-full flex-col">
            <header className="flex flex-col gap-4 border-b border-border bg-gradient-to-b from-accent/[0.07] to-transparent px-4 pb-0 pt-5 sm:px-6">
                <div className="flex flex-wrap items-end justify-between gap-x-6 gap-y-3">
                    <div>
                        <h1 className="text-xl font-bold tracking-tight">Play</h1>
                        <p className="mt-0.5 text-xs text-muted">
                            Jump into a game, pick a server, or let quick play choose.
                        </p>
                    </div>

                    {totals && !off && (
                        <dl className="flex items-center gap-5 text-xs">
                            <Total icon={FiGlobe} value={totals.games} label="games" />
                            <Total icon={FiServer} value={totals.servers} label="servers" />
                            <Total icon={FiUsers} value={totals.players} label="playing" live />
                        </dl>
                    )}
                </div>

                {!off && (
                    <nav aria-label="Play sections" className="-mb-px flex gap-1 overflow-x-auto">
                        {tabs.map((t) => (
                            <TabButton
                                key={t}
                                active={tab === t}
                                label={TAB_LABELS[t]}
                                count={
                                    t === 'invites' && inviteCount > 0
                                        ? inviteCount
                                        : t === 'lobbies' && totals && totals.lobbies > 0
                                          ? totals.lobbies
                                          : undefined
                                }
                                hot={t === 'invites' && inviteCount > 0}
                                onClick={() => update({ tab: t })}
                            />
                        ))}
                    </nav>
                )}
            </header>

            {!off && tab !== 'invites' && (
                <div className="flex flex-wrap items-center gap-2 border-b border-border px-4 py-3 sm:px-6">
                    <div className="relative min-w-[180px] flex-1 basis-60">
                        <FiSearch className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted" />
                        <input
                            type="search"
                            value={filters.q}
                            onChange={(e) => update({ q: e.target.value })}
                            placeholder={
                                tab === 'games'
                                    ? 'Search games'
                                    : tab === 'servers'
                                      ? 'Search servers'
                                      : 'Search lobbies'
                            }
                            aria-label="Search"
                            className="w-full rounded-lg border border-border bg-surface-2 py-1.5 pl-8 pr-8 text-xs"
                        />
                        {filters.q && (
                            <button
                                type="button"
                                onClick={() => update({ q: '' })}
                                aria-label="Clear the search"
                                className="absolute right-2 top-1/2 -translate-y-1/2 text-muted"
                            >
                                <FiX className="size-3.5" />
                            </button>
                        )}
                    </div>

                    <Select
                        label="Way to play"
                        value={filters.mode ?? MODE_ANY}
                        options={[
                            { value: MODE_ANY, label: 'Any way to play' },
                            ...(Object.keys(MODE_LABELS) as PlayModeT[]).map((m) => ({
                                value: m,
                                label: MODE_LABELS[m],
                            })),
                        ]}
                        onChange={(v) =>
                            update({ mode: v === MODE_ANY ? undefined : v })
                        }
                    />

                    {sorts.length > 0 && (
                        <Select
                            label="Sort"
                            value={resolveSort(tab, filters.sort)}
                            options={sorts.map((s) => ({ value: s, label: SORT_LABELS[s] }))}
                            onChange={(v) => update({ sort: v })}
                        />
                    )}

                    {tab === 'servers' && (
                        <Switch
                            on={filters.populated}
                            label="Has players"
                            onClick={() => update({ populated: !filters.populated })}
                        />
                    )}
                    {tab === 'lobbies' && (
                        <Switch
                            on={filters.joinable}
                            label="Still gathering"
                            onClick={() => update({ joinable: !filters.joinable })}
                        />
                    )}

                    {gameFilterName && (
                        <button
                            type="button"
                            onClick={() => update({ appId: undefined })}
                            className="flex items-center gap-1.5 rounded-full border border-accent bg-accent/10 py-1 pl-3 pr-2 text-[11px] text-accent"
                            aria-label={`Remove the ${gameFilterName} filter`}
                        >
                            {gameFilterName}
                            <FiX className="size-3" />
                        </button>
                    )}
                </div>
            )}

            <div className="flex-1 overflow-y-auto px-4 py-5 sm:px-6">
                <div className="mx-auto flex max-w-[1400px] flex-col gap-5">
                    {!off && first?.party && (
                        <PartyBar party={first.party} busy={join.isPending} onJoin={(t) => join.mutate(t)} />
                    )}

                    {join.isError && (
                        <p className="flex items-center gap-2 rounded-lg border border-danger/40 bg-danger/10 px-3 py-2 text-xs text-danger">
                            <FiAlertTriangle className="size-3.5 shrink-0" />
                            Could not open that server: {messageOf(join.error)}
                        </p>
                    )}

                    {tab === 'invites' ? (
                        <InvitesPanel
                            loading={invites.isPending}
                            error={invites.error}
                            invites={invites.data ?? []}
                        />
                    ) : listing.isPending ? (
                        <Skeleton tab={tab} />
                    ) : listing.isError ? (
                        <div className="space-y-1">
                            <p className="text-xs text-danger">The play center could not be loaded.</p>
                            <p className="text-[11px] text-muted">{messageOf(listing.error)}</p>
                            <button
                                type="button"
                                onClick={() => void listing.refetch()}
                                className="text-[11px] text-accent underline underline-offset-2"
                            >
                                Try again
                            </button>
                        </div>
                    ) : off ? (
                        <Empty
                            icon={FiPower}
                            title="Play is switched off"
                            body="The play center is turned off on the site at the moment, so there is nothing to launch from here. Browsing games, mods and servers still works."
                        >
                            <Link to="/apps" className={BTN_PRIMARY}>
                                <FiCompass className="size-3.5" />
                                Browse games
                            </Link>
                            <Link to="/browse/server" className={BTN_GHOST}>
                                <FiServer className="size-3.5" />
                                Browse servers
                            </Link>
                        </Empty>
                    ) : first && !first.anyPlayable ? (
                        <Empty
                            icon={FiGlobe}
                            title="Nothing is playable yet"
                            body="No game has a way to start from here yet. A game shows up on its own once it ships a web build or app support, so check back soon. Until then you can browse what exists and find servers directly."
                        >
                            <Link to="/apps" className={BTN_PRIMARY}>
                                <FiCompass className="size-3.5" />
                                Browse games
                            </Link>
                            <Link to="/browse/server" className={BTN_GHOST}>
                                <FiServer className="size-3.5" />
                                Browse servers
                            </Link>
                        </Empty>
                    ) : rows === 0 && narrowed ? (
                        <Empty
                            icon={FiSearch}
                            title={
                                filters.q
                                    ? `Nothing matched “${filters.q}”`
                                    : `No ${TAB_LABELS[tab].toLowerCase()} match those filters`
                            }
                            body="Try a different word, or loosen a filter."
                        >
                            <button
                                type="button"
                                onClick={() => setParams(filtersToParams({ ...filters, q: '', appId: undefined, mode: undefined, populated: false, joinable: false }), { replace: true })}
                                className={BTN_PRIMARY}
                            >
                                Clear filters
                            </button>
                        </Empty>
                    ) : rows === 0 ? (
                        <Empty
                            icon={tab === 'servers' ? FiServer : FiUsers}
                            title={
                                tab === 'games'
                                    ? 'No games to show'
                                    : tab === 'servers'
                                      ? 'No servers are online right now'
                                      : 'No lobbies are open right now'
                            }
                            body={
                                tab === 'lobbies'
                                    ? 'Lobbies appear while people gather a party for a game. Start one from a game, or check the games tab.'
                                    : 'Check back in a moment, or head to the games tab.'
                            }
                        >
                            {tab !== 'games' && (
                                <button type="button" onClick={() => update({ tab: 'games' })} className={BTN_PRIMARY}>
                                    See games
                                </button>
                            )}
                        </Empty>
                    ) : (
                        <>
                            {tab === 'games' && (
                                <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4">
                                    {apps.map((app) => (
                                        <GameCard
                                            key={app.id}
                                            app={app}
                                            listed={listed ? listed.has(app.id) : null}
                                            onPlay={openGame}
                                            onQuick={(a) => setQuick({ id: a.id, name: a.name })}
                                            onServers={focusGame}
                                        />
                                    ))}
                                </div>
                            )}

                            {tab === 'servers' && (
                                <ul className="flex flex-col gap-2">
                                    {servers.map((s) => (
                                        <ServerRow
                                            key={s.id}
                                            server={s}
                                            iconOf={iconOf}
                                            joinable={listed ? listed.has(s.appId) : true}
                                            busy={join.isPending && join.variables?.serverId === s.id}
                                            onJoin={(t) => join.mutate(t)}
                                        />
                                    ))}
                                </ul>
                            )}

                            {tab === 'lobbies' && (
                                <ul className="flex flex-col gap-2">
                                    {lobbies.map((l) => (
                                        <LobbyRow
                                            key={l.id}
                                            lobby={l}
                                            iconOf={iconOf}
                                            busy={join.isPending && join.variables?.serverId === l.serverId}
                                            onJoin={(t) => join.mutate(t)}
                                        />
                                    ))}
                                </ul>
                            )}

                            {listing.hasNextPage && (
                                <div className="flex justify-center">
                                    <button
                                        type="button"
                                        disabled={listing.isFetchingNextPage}
                                        onClick={() => void listing.fetchNextPage()}
                                        className={BTN_GHOST}
                                    >
                                        {listing.isFetchingNextPage ? 'Loading…' : 'Show more'}
                                    </button>
                                </div>
                            )}
                        </>
                    )}
                </div>
            </div>

            {quick && (
                <QuickMatchDialog
                    app={quick}
                    busy={join.isPending}
                    onJoin={(t) => join.mutate(t)}
                    onClose={() => setQuick(null)}
                />
            )}

            {target && <PlayDialog target={target} onClose={() => setTarget(null)} />}
        </div>
    )
}

/* ------------------------------------------------------------------------ */

function Total({
    icon: Icon,
    value,
    label,
    live,
}: {
    icon: typeof FiGlobe
    value: number
    label: string
    live?: boolean
}) {
    return (
        <div className="flex items-center gap-2">
            <span className="relative flex size-7 items-center justify-center rounded-lg bg-surface-2 text-muted">
                <Icon className="size-3.5" />
                {live && value > 0 && (
                    <span className="absolute -right-0.5 -top-0.5 size-2 rounded-full bg-success ring-2 ring-background" />
                )}
            </span>
            <div className="leading-tight">
                <dd className="text-sm font-semibold tabular-nums">{compact(value)}</dd>
                <dt className="text-[10px] text-muted">{label}</dt>
            </div>
        </div>
    )
}

function TabButton({
    active,
    label,
    count,
    hot,
    onClick,
}: {
    active: boolean
    label: string
    count?: number
    hot?: boolean
    onClick: () => void
}) {
    return (
        <button
            type="button"
            onClick={onClick}
            aria-current={active ? 'page' : undefined}
            className={`flex items-center gap-2 whitespace-nowrap border-b-2 px-2.5 py-2.5 text-sm transition sm:px-3.5 ${
                active
                    ? 'border-accent font-semibold text-foreground'
                    : 'border-transparent text-muted hover:text-foreground'
            }`}
        >
            {label}
            {count !== undefined && (
                <span
                    className={`rounded-full px-1.5 py-px text-[10px] font-semibold tabular-nums ${
                        hot ? 'bg-accent text-accent-foreground' : 'bg-surface-2 text-muted'
                    }`}
                >
                    {count}
                </span>
            )}
        </button>
    )
}

function Switch({ on, label, onClick }: { on: boolean; label: string; onClick: () => void }) {
    return (
        <button
            type="button"
            role="switch"
            aria-checked={on}
            onClick={onClick}
            className="flex items-center gap-2 rounded-lg border border-border px-2.5 py-1.5 text-xs transition hover:border-accent"
        >
            <span
                className={`relative h-4 w-7 rounded-full transition ${on ? 'bg-accent' : 'bg-surface-2 ring-1 ring-inset ring-border'}`}
            >
                <span
                    className={`absolute top-0.5 size-3 rounded-full bg-white shadow transition-all ${on ? 'left-3.5' : 'left-0.5'}`}
                />
            </span>
            {label}
        </button>
    )
}

function Empty({
    icon: Icon,
    title,
    body,
    children,
}: {
    icon: typeof FiGlobe
    title: string
    body: string
    children?: React.ReactNode
}) {
    return (
        <div className="mx-auto flex max-w-md flex-col items-center gap-3 rounded-2xl border border-dashed border-border px-6 py-12 text-center">
            <span className="flex size-12 items-center justify-center rounded-2xl bg-surface-2 text-muted">
                <Icon className="size-5" />
            </span>
            <h2 className="text-sm font-semibold">{title}</h2>
            <p className="text-xs leading-relaxed text-muted">{body}</p>
            {children && <div className="mt-1 flex flex-wrap justify-center gap-2">{children}</div>}
        </div>
    )
}

function Skeleton({ tab }: { tab: Tone }) {
    const pulse = 'animate-pulse rounded-2xl bg-surface-2/70'

    return tab === 'games' ? (
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4" aria-busy>
            {Array.from({ length: 8 }, (_, i) => (
                <div key={i} className={`${pulse} h-64`} />
            ))}
        </div>
    ) : (
        <div className="flex flex-col gap-2" aria-busy>
            {Array.from({ length: 6 }, (_, i) => (
                <div key={i} className={`${pulse} h-16`} />
            ))}
        </div>
    )
}

function InvitesPanel({
    loading,
    error,
    invites,
}: {
    loading: boolean
    error: unknown
    invites: { id: string; partyId: string; partyName: string | null; message: string | null }[]
}) {
    if (loading) return <Skeleton tab="invites" />

    if (error) return <p className="text-xs text-danger">{messageOf(error)}</p>

    if (invites.length === 0)
        return (
            <Empty
                icon={FiUsers}
                title="No invitations"
                body="When a friend invites you to a party it lands here, and in the Friends screen."
            >
                <Link to="/friends" className={BTN_GHOST}>
                    <FiUsers className="size-3.5" />
                    Open Friends
                </Link>
            </Empty>
        )

    return (
        <div className="divide-y divide-border overflow-hidden rounded-2xl border border-border bg-surface">
            {invites.map((invite) => (
                <InviteRow
                    key={invite.id}
                    id={invite.id}
                    label={invite.partyName ?? `Party ${invite.partyId}`}
                    message={invite.message}
                />
            ))}
        </div>
    )
}
