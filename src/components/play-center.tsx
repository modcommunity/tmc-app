import { useState, type ReactNode } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import {
    FiAlertTriangle,
        FiClock,
    FiExternalLink,
    FiLock,
    FiLogOut,
    FiMapPin,
    FiPlay,
    FiRefreshCw,
    FiServer,
    FiStar,
    FiUser,
    FiUsers,
    FiX,
    FiZap,
} from 'react-icons/fi'

import { api } from '~/lib/api/client'
import { openExternally } from '~/lib/external'
import { messageOf } from '~/lib/ipc'
import { appLabel } from '~/lib/api/labels'
import type {
    PlayAppRowSchema,
    PlayLobbyRowSchema,
    PlayPartySchema,
    PlayQuickResultSchema,
    PlayServerRowSchema,
} from '~/lib/api/contract'
import { MODE_LABELS, compact, fill, players } from '~/lib/api/play'
import { canonicalSlug, gameHeadingStyle, gameLook } from '~/lib/gamespice/games'
import { GameIcon } from '~/components/game-icon'
import type { z } from 'zod'

export type PlayAppRowT = z.infer<typeof PlayAppRowSchema>
export type PlayServerRowT = z.infer<typeof PlayServerRowSchema>
export type PlayLobbyRowT = z.infer<typeof PlayLobbyRowSchema>
export type PlayPartyT = z.infer<typeof PlayPartySchema>
export type PlayQuickT = z.infer<typeof PlayQuickResultSchema>

/** What a row hands the screen when somebody presses Join. */
export type JoinTarget = {
    serverId: number
    appId: number
    appName: string
    /** Left-click quick play: start without the chooser. */
    quick?: boolean
}

/* ------------------------------------------------------------------------ */
/*  Small pieces                                                            */
/* ------------------------------------------------------------------------ */

function Pill({
    children,
    tone = 'muted',
    title,
}: {
    children: ReactNode
    tone?: 'muted' | 'accent' | 'success' | 'warning'
    title?: string
}) {
    const cls = {
        muted: 'border-border bg-surface-2 text-muted',
        accent: 'border-accent/40 bg-accent/10 text-accent',
        success: 'border-success/40 bg-success/10 text-success',
        warning: 'border-warning/40 bg-warning/10 text-warning',
    }[tone]

    return (
        <span
            title={title}
            className={`inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[10px] font-medium ${cls}`}
        >
            {children}
        </span>
    )
}

/** Players against capacity. The bar is a hint; the number beside it is the fact. */
function Occupancy({ cur, max, offline }: { cur: number; max: number; offline?: boolean }) {
    const ratio = fill(cur, max)
    const full = ratio !== null && ratio >= 1
    const tone =
        ratio === null || ratio < 0.6
            ? 'bg-success'
            : ratio < 0.9
              ? 'bg-warning'
              : 'bg-danger'

    return (
        <div className="flex min-w-[88px] flex-col gap-1">
            <div className="flex items-baseline justify-between gap-2 text-xs tabular-nums">
                <span className={offline ? 'text-muted' : 'font-semibold'}>
                    {offline ? 'Offline' : players(cur, max)}
                </span>
                {full && !offline && (
                    <span className="text-[10px] font-medium text-danger">Full</span>
                )}
            </div>
            <div className="h-1 overflow-hidden rounded-full bg-surface-2">
                {!offline && (
                    <div
                        className={`h-full rounded-full ${tone}`}
                        style={{ width: `${Math.max(ratio ?? (cur > 0 ? 0.25 : 0), cur > 0 ? 0.06 : 0) * 100}%` }}
                    />
                )}
            </div>
        </div>
    )
}

function ModePills({ modes }: { modes: PlayAppRowT['modes'] }) {
    return (
        <>
            {modes.map((m) => (
                <Pill key={m}>{MODE_LABELS[m]}</Pill>
            ))}
        </>
    )
}

const BTN_PRIMARY =
    'inline-flex items-center justify-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 font-semibold text-accent-foreground transition hover:brightness-110 disabled:opacity-50'
const BTN_GHOST =
    'inline-flex items-center justify-center gap-1.5 rounded-lg border border-border px-3 py-1.5 font-medium transition hover:border-accent disabled:opacity-50'

export { BTN_GHOST, BTN_PRIMARY }

/* ------------------------------------------------------------------------ */
/*  Game card                                                               */
/* ------------------------------------------------------------------------ */

export function GameCard({
    app,
    listed,
    onPlay,
    onQuick,
    onServers,
}: {
    app: PlayAppRowT
    /** Whether quick play and Join work for this game. Unknown (null) counts as yes. */
    listed: boolean | null
    onPlay: (app: PlayAppRowT, advanced: boolean) => void
    onQuick: (app: PlayAppRowT, advanced: boolean) => void
    onServers: (app: PlayAppRowT) => void
}) {
    const art = app.bannerUrl ?? app.iconUrl
    const canQuick = listed !== false && app.serverCnt > 0
    return (
        <article className="group flex flex-col overflow-hidden rounded-2xl border border-border bg-surface transition duration-200 hover:-translate-y-0.5 hover:border-accent/50 hover:shadow-lg hover:shadow-black/20">
            <div className="relative aspect-[16/8] w-full overflow-hidden bg-surface-2">
                {art ? (
                    <img
                        src={art}
                        alt=""
                        aria-hidden
                        loading="lazy"
                        className="size-full object-cover transition duration-300 group-hover:scale-105"
                    />
                ) : (
                    <div className="size-full bg-gradient-to-br from-accent/25 via-surface-2 to-surface" />
                )}
                <div className="absolute inset-0 bg-gradient-to-t from-surface via-surface/20 to-transparent" />

                <div className="absolute left-2.5 top-2.5 flex flex-wrap gap-1">
                    {app.isOfficial && (
                        <span className="flex items-center gap-1 rounded-full bg-black/60 px-2 py-0.5 text-[10px] font-medium text-white backdrop-blur">
                            <FiStar className="size-2.5" />
                            Official
                        </span>
                    )}
                    {app.solo && (
                        <span className="flex items-center gap-1 rounded-full bg-black/60 px-2 py-0.5 text-[10px] font-medium text-white backdrop-blur">
                            <FiUser className="size-2.5" />
                            Solo
                        </span>
                    )}
                </div>

                {app.playerCnt > 0 && (
                    <span className="absolute right-2.5 top-2.5 flex items-center gap-1.5 rounded-full bg-black/60 px-2 py-0.5 text-[10px] font-medium text-white backdrop-blur">
                        <span className="size-1.5 rounded-full bg-success" />
                        {compact(app.playerCnt)} playing
                    </span>
                )}
            </div>

            <div className="relative flex flex-1 flex-col gap-3 px-4 pb-4">
                <div className="-mt-6 flex items-end gap-3">
                    <GameIcon
                        app={{ id: app.id, name: app.name, icon: app.iconUrl }}
                        size="lg"
                        className="shrink-0 shadow-md ring-2 ring-surface"
                    />
                    <div className="min-w-0 pb-0.5">
                        <h3 className="truncate text-sm font-semibold">{appLabel(app)}</h3>
                        <div className="flex items-center gap-3 text-[11px] text-muted">
                            {app.serverCnt > 0 ? (
                                <button
                                    type="button"
                                    onClick={() => onServers(app)}
                                    title={`See the ${app.serverCnt} servers`}
                                    className="flex items-center gap-1 transition hover:text-accent"
                                >
                                    <FiServer className="size-3" />
                                    {app.serverCnt} {app.serverCnt === 1 ? 'server' : 'servers'}
                                </button>
                            ) : (
                                <span className="flex items-center gap-1">
                                    <FiServer className="size-3" />
                                    No servers
                                </span>
                            )}
                            {app.lobbyCnt > 0 && (
                                <span className="flex items-center gap-1" title={`${app.lobbyCnt} lobbies`}>
                                    <FiUsers className="size-3" />
                                    {app.lobbyCnt} {app.lobbyCnt === 1 ? 'lobby' : 'lobbies'}
                                </span>
                            )}
                        </div>
                    </div>
                </div>

                <p className="line-clamp-2 min-h-[2rem] text-[11px] leading-relaxed text-muted">
                    {app.description ?? 'No description yet.'}
                </p>

                <div className="flex flex-wrap gap-1">
                    <ModePills modes={app.modes} />
                </div>

                <GameActions app={app} canQuick={canQuick} onPlay={onPlay} onQuick={onQuick} />
            </div>
        </article>
    )
}

/** Quick play (left click: best server, right click: chooser) and Play / Play solo. */
function GameActions({
    app,
    canQuick,
    onPlay,
    onQuick,
}: {
    app: PlayAppRowT
    canQuick: boolean
    onPlay: (app: PlayAppRowT, advanced: boolean) => void
    onQuick: (app: PlayAppRowT, advanced: boolean) => void
}) {
    const nothingToDo = !canQuick && !app.directPlay

    return (
        <div className="mt-auto flex flex-wrap items-center gap-2 pt-1">
                    {canQuick && (
                        <button
                            type="button"
                            title="Right-click for advanced options"
                            onClick={() => onQuick(app, false)}
                            onContextMenu={(e) => {
                                e.preventDefault()
                                onQuick(app, true)
                            }}
                            className={BTN_PRIMARY}
                        >
                            <FiZap className="size-3.5" />
                            Quick play
                        </button>
                    )}
                    {app.directPlay && (
                        <button
                            type="button"
                            title="Right-click for advanced options"
                            onClick={() => onPlay(app, false)}
                            onContextMenu={(e) => {
                                e.preventDefault()
                                onPlay(app, true)
                            }}
                            className={canQuick ? BTN_GHOST : BTN_PRIMARY}
                        >
                            <FiPlay className="size-3.5" />
                            {app.solo ? 'Play solo' : 'Play'}
                        </button>
                    )}
                    {nothingToDo && (
                        <span className="text-[11px] text-muted">
                            Nothing to join right now.
                        </span>
                    )}
                </div>
    )
}

/**
 * A GameSpice platform game: the game's own accent, display face and badge,
 * with the same Quick play and Play / Solo buttons as every other card.
 */
export function PlatformGameCard({
    app,
    listed,
    onPlay,
    onQuick,
    onServers,
}: {
    app: PlayAppRowT
    listed: boolean | null
    onPlay: (app: PlayAppRowT, advanced: boolean) => void
    onQuick: (app: PlayAppRowT, advanced: boolean) => void
    onServers: (app: PlayAppRowT) => void
}) {
    const look = gameLook(app.url ?? app.name)
    const art = app.bannerUrl ?? app.iconUrl
    const slug = app.url ? canonicalSlug(app.url) : null
    const canQuick = listed !== false && app.serverCnt > 0

    return (
        <article
            className="group flex flex-col overflow-hidden rounded-2xl border border-border bg-surface shadow-md transition duration-200 hover:-translate-y-0.5 hover:shadow-lg hover:shadow-black/30"
            style={{ borderTopColor: look.accent, borderTopWidth: 3 }}
        >
            <div className="relative aspect-[16/9] w-full overflow-hidden bg-surface-2">
                {art ? (
                    <img
                        src={art}
                        alt=""
                        aria-hidden
                        loading="lazy"
                        className="size-full object-cover transition duration-300 group-hover:scale-105"
                    />
                ) : (
                    <div
                        className="size-full"
                        style={{
                            background: `linear-gradient(135deg, hsl(${look.hue} 60% 35%), hsl(${(look.hue + 40) % 360} 50% 18%))`,
                        }}
                    />
                )}
                <div
                    className="absolute inset-0"
                    style={{
                        background: `linear-gradient(to top, var(--color-surface, #111), transparent 70%), linear-gradient(135deg, hsl(${look.hue} 70% 50% / 0.18), transparent 60%)`,
                    }}
                />
                <span
                    className="absolute left-2.5 top-2.5 flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-bold uppercase tracking-wide text-black"
                    style={{ background: look.accent }}
                >
                    <FiStar className="size-2.5" />
                    GameSpice
                </span>
                {app.playerCnt > 0 && (
                    <span className="absolute right-2.5 top-2.5 flex items-center gap-1.5 rounded-full bg-black/60 px-2 py-0.5 text-[10px] font-medium text-white backdrop-blur">
                        <span className="size-1.5 rounded-full bg-success" />
                        {compact(app.playerCnt)} playing
                    </span>
                )}
            </div>

            <div className="flex flex-1 flex-col gap-3 p-4">
                <div>
                    <h3 className="text-xl font-extrabold leading-tight" style={gameHeadingStyle(look)}>
                        {app.name}
                    </h3>
                    <p className="mt-1 line-clamp-2 min-h-[2rem] text-xs text-muted">
                        {app.description ?? 'A GameSpice game.'}
                    </p>
                </div>

                <p className="flex flex-wrap items-center gap-3 text-[11px] text-muted">
                    <button
                        type="button"
                        disabled={app.serverCnt === 0}
                        onClick={() => onServers(app)}
                        className="flex items-center gap-1 transition hover:text-accent disabled:hover:text-muted"
                    >
                        <FiServer className="size-3" />
                        {app.serverCnt} {app.serverCnt === 1 ? 'server' : 'servers'}
                    </button>
                    {app.lobbyCnt > 0 && (
                        <span className="flex items-center gap-1">
                            <FiUsers className="size-3" />
                            {app.lobbyCnt} {app.lobbyCnt === 1 ? 'lobby' : 'lobbies'}
                        </span>
                    )}
                    {app.solo && (
                        <span className="flex items-center gap-1">
                            <FiUser className="size-3" />
                            Solo
                        </span>
                    )}
                </p>

                <GameActions app={app} canQuick={canQuick} onPlay={onPlay} onQuick={onQuick} />

                {slug && (
                    <Link to={`/games/${slug}`} className="text-[11px] text-muted transition hover:text-accent">
                        Game page
                    </Link>
                )}
            </div>
        </article>
    )
}

/* ------------------------------------------------------------------------ */
/*  Server and lobby rows                                                   */
/* ------------------------------------------------------------------------ */

const ROW =
    'flex flex-wrap items-center gap-x-4 gap-y-2 rounded-xl border border-border bg-surface px-3.5 py-3 transition hover:border-accent/50'

export function ServerRow({
    server,
    iconOf,
    joinable,
    busy,
    onJoin,
}: {
    server: PlayServerRowT
    iconOf: (appId: number) => string | null
    /** False when the game has no launch route, so there is nothing a Join could do. */
    joinable: boolean
    busy: boolean
    onJoin: (target: JoinTarget) => void
}) {
    const name = server.name?.trim() || `Server ${server.id}`

    return (
        <li className={`${ROW} ${server.online ? '' : 'opacity-70'}`}>
            <GameIcon
                app={{ id: server.appId, name: server.appName, icon: iconOf(server.appId) }}
                size="md"
            />

            <div className="min-w-0 flex-1 basis-48">
                <p className="truncate text-sm font-medium">{name}</p>
                <p className="flex items-center gap-1.5 truncate text-[11px] text-muted">
                    <span className="truncate">{server.appName}</span>
                    {server.map && (
                        <>
                            <span aria-hidden>·</span>
                            <span className="flex items-center gap-1 truncate">
                                <FiMapPin className="size-2.5 shrink-0" />
                                <span className="truncate">{server.map}</span>
                            </span>
                        </>
                    )}
                </p>
            </div>

            <div className="hidden gap-1 sm:flex">
                <ModePills modes={server.modes} />
            </div>

            <Occupancy cur={server.curUsers} max={server.maxUsers} offline={!server.online} />

            {joinable ? (
                <button
                    type="button"
                    disabled={!server.online || busy}
                    onClick={() =>
                        onJoin({ serverId: server.id, appId: server.appId, appName: server.appName })
                    }
                    className={BTN_PRIMARY}
                >
                    <FiPlay className="size-3.5" />
                    {busy ? 'Opening' : 'Join'}
                </button>
            ) : (
                <button type="button" disabled className={BTN_GHOST} title="This game cannot be launched from here yet.">
                    Not playable
                </button>
            )}
        </li>
    )
}

export function LobbyRow({
    lobby,
    iconOf,
    busy,
    onJoin,
}: {
    lobby: PlayLobbyRowT
    iconOf: (appId: number) => string | null
    busy: boolean
    onJoin: (target: JoinTarget) => void
}) {
    const gathering = lobby.stage === 'LOBBY'
    const name = lobby.name?.trim() || (lobby.hostName ? `${lobby.hostName}'s lobby` : `Lobby ${lobby.id}`)

    return (
        <li className={ROW}>
            <GameIcon
                app={{ id: lobby.appId, name: lobby.appName, icon: iconOf(lobby.appId) }}
                size="md"
            />

            <div className="min-w-0 flex-1 basis-48">
                <p className="flex items-center gap-1.5 truncate text-sm font-medium">
                    <span className="truncate">{name}</span>
                    {lobby.needsPassword && (
                        <FiLock className="size-3 shrink-0 text-muted" aria-label="Needs a password" />
                    )}
                </p>
                <p className="truncate text-[11px] text-muted">
                    {lobby.appName}
                    {lobby.hostName ? ` · hosted by ${lobby.hostName}` : ''}
                </p>
            </div>

            <Pill tone={gathering ? 'success' : 'warning'}>
                {gathering ? <FiClock className="size-2.5" /> : <FiPlay className="size-2.5" />}
                {gathering ? 'Gathering' : 'In a match'}
            </Pill>

            <Occupancy cur={lobby.users} max={lobby.maxUsers} />

            <div className="flex items-center gap-2">
                <button type="button" onClick={() => void openExternally({ webUrl: lobby.url })} className={BTN_GHOST}>
                    <FiExternalLink className="size-3.5" />
                    Open
                </button>
                {lobby.serverId != null && (
                    <button
                        type="button"
                        disabled={busy}
                        onClick={() =>
                            onJoin({ serverId: lobby.serverId as number, appId: lobby.appId, appName: lobby.appName })
                        }
                        className={BTN_PRIMARY}
                    >
                        <FiPlay className="size-3.5" />
                        Join server
                    </button>
                )}
            </div>
        </li>
    )
}

/* ------------------------------------------------------------------------ */
/*  The viewer's party                                                      */
/* ------------------------------------------------------------------------ */

function Face({ name }: { name: string | null }) {
    return (
        <span
            aria-hidden
            className="flex size-6 items-center justify-center rounded-full border-2 border-surface bg-surface-2 text-[10px] font-semibold"
        >
            {(name ?? '?').slice(0, 1).toUpperCase()}
        </span>
    )
}

/**
 * The party the response carries, as a bar above the lists.
 *
 * Reuses the friends screen's vocabulary (leave through `partyLeave`, the same
 * query key family) rather than a party system of its own: this only displays
 * what the page answered and offers the two things worth doing from here.
 */
export function PartyBar({
    party,
    busy,
    onJoin,
}: {
    party: PlayPartyT
    busy: boolean
    onJoin: (target: JoinTarget) => void
}) {
    const client = useQueryClient()
    const leave = useMutation({
        mutationFn: () => api.partyLeave(party.id),
        onSettled: () => {
            void client.invalidateQueries({ queryKey: ['social'] })
            void client.invalidateQueries({ queryKey: ['play'] })
        },
    })

    const server = party.server

    return (
        <section
            aria-label="Your party"
            className="flex flex-wrap items-center gap-x-4 gap-y-3 rounded-2xl border border-accent/30 bg-gradient-to-r from-accent/10 via-surface to-surface px-4 py-3"
        >
            <GameIcon
                app={{ id: party.app.id, name: party.app.name, icon: party.app.iconUrl }}
                size="lg"
            />

            <div className="min-w-0 flex-1 basis-56">
                <p className="text-[10px] font-semibold uppercase tracking-wider text-accent">
                    Your party
                </p>
                <p className="truncate text-sm font-semibold">
                    {party.name ?? `Party ${party.id}`}
                </p>
                <p className="truncate text-[11px] text-muted">
                    {party.app.name} · {party.stage.toLowerCase()} · {players(party.users, party.maxUsers)}
                    {party.inGame > 0 ? ` · ${party.inGame} in game` : ''}
                </p>
                {leave.error && (
                    <p className="text-[11px] text-danger">{messageOf(leave.error)}</p>
                )}
            </div>

            <div className="flex -space-x-1.5">
                {party.present.slice(0, 5).map((p) => (
                    <Face key={p.userId} name={p.name} />
                ))}
                {party.present.length > 5 && (
                    <span className="flex size-6 items-center justify-center rounded-full border-2 border-surface bg-surface-2 text-[10px]">
                        +{party.present.length - 5}
                    </span>
                )}
            </div>

            <div className="flex items-center gap-2 whitespace-nowrap">
                {server?.online && (
                    <button
                        type="button"
                        disabled={busy}
                        onClick={() =>
                            onJoin({ serverId: server.id, appId: party.app.id, appName: party.app.name })
                        }
                        className={BTN_PRIMARY}
                    >
                        <FiPlay className="size-3.5" />
                        Join server
                    </button>
                )}
                <Link to="/friends" className={BTN_GHOST} aria-label="Friends">
                    <FiUsers className="size-3.5" />
                    <span className="hidden sm:inline">Friends</span>
                </Link>
                <button
                    type="button"
                    disabled={leave.isPending}
                    onClick={() => leave.mutate()}
                    className={`${BTN_GHOST} hover:border-danger`}
                >
                    <FiLogOut className="size-3.5" />
                    Leave
                </button>
            </div>
        </section>
    )
}

/* ------------------------------------------------------------------------ */
/*  Quick match                                                             */
/* ------------------------------------------------------------------------ */

/**
 * Pick a server for one game, then hand the choice back.
 *
 * The search runs on open and again on "Search again". It joins nothing by
 * itself: the best server is shown first, with the reasons it was ranked
 * there, and one press opens the same launcher a server row does, so the choice
 * of how to start (window, installed copy, connect link) stays the player's.
 */
export function QuickMatchDialog({
    app,
    busy,
    onJoin,
    onClose,
}: {
    app: { id: number; name: string }
    busy: boolean
    onJoin: (target: JoinTarget) => void
    onClose: () => void
}) {
    const [rotation, setRotation] = useState(false)

    const search = useQuery({
        queryKey: ['play', 'quick', app.id, rotation],
        queryFn: () => api.playQuickMatch(app.id, rotation),
        // A search is a moment, not a record: every open asks again.
        staleTime: 0,
        gcTime: 0,
        retry: false,
    })

    const result = search.data
    const best = result?.best ?? null
    const others = (result?.top ?? []).filter((c) => c.id !== best?.id)

    const pick = (serverId: number) =>
        onJoin({ serverId, appId: app.id, appName: result?.appName ?? app.name })

    return (
        <div
            role="dialog"
            aria-modal="true"
            aria-label={`Quick play ${app.name}`}
            className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
        >
            <div className="flex max-h-[90vh] w-full max-w-md flex-col overflow-hidden rounded-2xl border border-border bg-surface shadow-2xl">
                <header className="flex items-center gap-3 border-b border-border px-4 py-3">
                    <span className="flex size-8 items-center justify-center rounded-lg bg-accent/15 text-accent">
                        <FiZap className="size-4" />
                    </span>
                    <div className="min-w-0 flex-1">
                        <h2 className="truncate text-sm font-semibold">Quick play</h2>
                        <p className="truncate text-[11px] text-muted">{app.name}</p>
                    </div>
                    <button
                        type="button"
                        onClick={onClose}
                        aria-label="Close"
                        className="rounded-md p-1.5 text-muted hover:bg-surface-2 hover:text-foreground"
                    >
                        <FiX className="size-4" />
                    </button>
                </header>

                <div className="flex flex-col gap-4 overflow-y-auto p-4">
                    {search.isFetching ? (
                        <div className="flex items-center gap-3 py-6 text-xs text-muted">
                            <FiRefreshCw className="size-4 animate-spin" />
                            Looking for the best server…
                        </div>
                    ) : search.isError ? (
                        <div className="space-y-1 py-2">
                            <p className="flex items-center gap-2 text-xs text-danger">
                                <FiAlertTriangle className="size-3.5" />
                                The search failed.
                            </p>
                            <p className="text-[11px] text-muted">{messageOf(search.error)}</p>
                        </div>
                    ) : result == null ? (
                        <p className="py-4 text-xs text-muted">
                            Quick play is not available for this game right now.
                        </p>
                    ) : !best ? (
                        <div className="space-y-1 py-4">
                            <p className="text-sm font-medium">No open server right now</p>
                            <p className="text-[11px] text-muted">
                                {result.scanned === 0
                                    ? `${result.appName} has no servers listed.`
                                    : `Checked ${result.scanned}: ${result.full} full, ${result.stale} not responding.`}{' '}
                                {!rotation && 'You can include servers that are between maps.'}
                            </p>
                        </div>
                    ) : (
                        <>
                            <div className="rounded-xl border border-accent/40 bg-accent/5 p-3.5">
                                <p className="text-[10px] font-semibold uppercase tracking-wider text-accent">
                                    Best match
                                </p>
                                <p className="mt-1 truncate text-sm font-semibold">
                                    {best.name ?? `Server ${best.id}`}
                                </p>
                                <p className="flex items-center gap-2 truncate text-[11px] text-muted">
                                    {best.map && (
                                        <span className="flex items-center gap-1">
                                            <FiMapPin className="size-2.5" />
                                            {best.map}
                                        </span>
                                    )}
                                    <span className="flex items-center gap-1">
                                        <FiUsers className="size-2.5" />
                                        {players(best.curUsers, best.maxUsers)}
                                    </span>
                                    {result.country && <span>near {result.country.region ?? result.country.code}</span>}
                                </p>
                                {result.top[0] && result.top[0].why.length > 0 && (
                                    <div className="mt-2 flex flex-wrap gap-1">
                                        {result.top[0].why.map((w) => (
                                            <Pill key={w} tone="accent">
                                                {w}
                                            </Pill>
                                        ))}
                                    </div>
                                )}
                                <button
                                    type="button"
                                    disabled={busy}
                                    onClick={() => pick(best.id)}
                                    className={`${BTN_PRIMARY} mt-3 w-full`}
                                >
                                    <FiPlay className="size-3.5" />
                                    {busy ? 'Opening' : 'Join this server'}
                                </button>
                            </div>

                            {others.length > 0 && (
                                <div>
                                    <p className="mb-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted">
                                        Next best
                                    </p>
                                    <ul className="divide-y divide-border overflow-hidden rounded-xl border border-border">
                                        {others.map((c) => (
                                            <li key={c.id} className="flex items-center gap-3 px-3 py-2">
                                                <div className="min-w-0 flex-1">
                                                    <p className="truncate text-xs font-medium">
                                                        {c.name ?? `Server ${c.id}`}
                                                    </p>
                                                    <p className="truncate text-[11px] text-muted">
                                                        {c.map ?? 'No map reported'} · {players(c.curUsers, c.maxUsers)}
                                                    </p>
                                                </div>
                                                <button
                                                    type="button"
                                                    disabled={busy}
                                                    onClick={() => pick(c.id)}
                                                    className={BTN_GHOST}
                                                >
                                                    Join
                                                </button>
                                            </li>
                                        ))}
                                    </ul>
                                </div>
                            )}
                        </>
                    )}

                    {result && (
                        <p className="text-[10px] text-muted">
                            Looked at {result.scanned} servers in {Math.round(result.tookMs)} ms
                            {result.full > 0 ? `, ${result.full} full` : ''}
                            {result.stale > 0 ? `, ${result.stale} not responding` : ''}.
                        </p>
                    )}
                </div>

                <footer className="flex items-center gap-3 border-t border-border px-4 py-3">
                    <label className="flex items-center gap-2 text-[11px] text-muted">
                        <input
                            type="checkbox"
                            checked={rotation}
                            onChange={(e) => setRotation(e.target.checked)}
                        />
                        Include servers changing map
                    </label>
                    <button
                        type="button"
                        disabled={search.isFetching}
                        onClick={() => void search.refetch()}
                        className={`${BTN_GHOST} ml-auto`}
                    >
                        <FiRefreshCw className="size-3.5" />
                        Search again
                    </button>
                </footer>
            </div>
        </div>
    )
}
