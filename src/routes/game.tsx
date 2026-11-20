import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Link, useParams, useSearchParams } from 'react-router-dom'
import {
    FiArrowLeft,
    FiAward,
    FiBox,
    FiExternalLink,
    FiPackage,
    FiPlay,
    FiServer,
    FiUsers,
} from 'react-icons/fi'

import { api } from '~/lib/api/client'
import { formatStat, isStatKey, type StatsTopT } from '~/lib/api/social'
import { useAuth } from '~/lib/auth/provider'
import { openExternally } from '~/lib/external'
import { messageOf } from '~/lib/ipc'
import { GameIcon } from '~/components/game-icon'
import PlayDialog, { type PlayTargetT } from '~/components/play-dialog'

/**
 * **One game** — `/apps/:id`. What the Apps card is a summary of.
 *
 * Three things the card has no room for, all of which a Dot game grew in the
 * platform's September work:
 *
 *   * **Its busiest servers**, from `/browse` — the same rows, filters and
 *     live query as the server browser, capped to a short list with a link to
 *     the full one.
 *   * **Leaderboards**, from `/stats/top` — any stat a game's servers file
 *     through `dot-stats`, ranked by the site, with the signed-in member's own
 *     row beside it.
 *   * **Your figures**, from `/stats/me`, shown beside each stat.
 *   * **The picker**, from `/stats/defs`: every visible stat the game
 *     declared, so a member new to a game still has something to pick. A site
 *     without that route falls back to the keys this member holds, and a typed
 *     key reaches the rest either way.
 *
 * The stat in view lives in the URL (`?stat=`), like every durable view choice.
 */
export default function GameRoute() {
    const params = useParams<{ id: string }>()
    const id = Number(params.id)
    const valid = Number.isInteger(id) && id > 0
    const [target, setTarget] = useState<PlayTargetT | null>(null)

    const looked = useQuery({
        queryKey: ['apps', { ids: [id] }],
        queryFn: () => api.apps({ ids: [id], limit: 1 }),
        enabled: valid,
    })

    const app = looked.data?.apps[0] ?? null

    if (!valid) return <Centered>That is not a game.</Centered>
    if (looked.isPending) return <Centered>Loading…</Centered>
    if (looked.error) return <Centered>{messageOf(looked.error)}</Centered>
    if (!app) return <Centered>No such game.</Centered>

    const canPlay = app.play !== null || app.install !== null

    return (
        <div className="flex flex-col gap-4 p-3">
            <Link
                to="/apps"
                className="flex w-fit items-center gap-1 text-xs text-muted hover:text-foreground"
            >
                <FiArrowLeft className="size-3" />
                All games
            </Link>

            <header className="flex flex-wrap items-start gap-3">
                <GameIcon
                    app={{ id: app.id, name: app.name, icon: app.images.icon }}
                    size="lg"
                />
                <div className="min-w-0 flex-1">
                    <h1 className="text-xl font-bold">{app.name}</h1>
                    {app.engine && (
                        <p className="text-xs text-muted">
                            Built on {app.engine.name}
                        </p>
                    )}
                    {app.description && (
                        <p className="mt-1 max-w-prose text-sm text-muted">
                            {app.description}
                        </p>
                    )}
                    <div className="mt-2 flex flex-wrap items-center gap-3 text-xs text-muted">
                        <span className="flex items-center gap-1">
                            <FiUsers className="size-3" />
                            {app.counts.players.toLocaleString()} playing
                        </span>
                        <span className="flex items-center gap-1">
                            <FiServer className="size-3" />
                            {app.counts.servers.toLocaleString()} servers
                        </span>
                        <span className="flex items-center gap-1">
                            <FiPackage className="size-3" />
                            {app.counts.mods.toLocaleString()} mods
                        </span>
                        <span className="flex items-center gap-1">
                            <FiBox className="size-3" />
                            {app.counts.assets.toLocaleString()} assets
                        </span>
                    </div>
                </div>

                <div className="flex flex-wrap items-center gap-1.5">
                    {canPlay && (
                        <button
                            type="button"
                            onClick={() => setTarget({ appId: app.id, app })}
                            className="flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-sm font-semibold text-accent-foreground"
                        >
                            <FiPlay className="size-3.5" />
                            Play
                        </button>
                    )}
                    <button
                        type="button"
                        onClick={() => void openExternally(app)}
                        className="flex items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-sm"
                    >
                        <FiExternalLink className="size-3.5" />
                        Website
                    </button>
                </div>
            </header>

            {app.hasServers && <TopServers appId={app.id} />}

            <Leaderboards appId={app.id} />


            {target && (
                <PlayDialog target={target} onClose={() => setTarget(null)} />
            )}
        </div>
    )
}

function Centered({ children }: { children: React.ReactNode }) {
    return <div className="p-6 text-center text-sm text-muted">{children}</div>
}

function Section({
    title,
    icon: Icon,
    action,
    children,
}: {
    title: string
    icon: typeof FiServer
    action?: React.ReactNode
    children: React.ReactNode
}) {
    return (
        <section className="flex flex-col gap-2">
            <div className="flex items-center justify-between gap-2">
                <h2 className="flex items-center gap-1.5 text-sm font-semibold">
                    <Icon className="size-3.5" />
                    {title}
                </h2>
                {action}
            </div>
            {children}
        </section>
    )
}

const SERVER_ROWS = 8

function TopServers({ appId }: { appId: number }) {
    const servers = useQuery({
        queryKey: ['browse', 'game-servers', appId],
        queryFn: () =>
            api.browse({
                kind: 'server',
                apps: [appId],
                onlineOnly: true,
                sort: 'curUsers',
                sortDir: 'desc',
                limit: SERVER_ROWS,
            }),
    })

    return (
        <Section
            title="Servers"
            icon={FiServer}
            action={
                <Link
                    to={`/browse/server?apps=${appId}`}
                    className="text-xs text-muted hover:text-foreground"
                >
                    Server browser →
                </Link>
            }
        >
            {servers.isPending && <p className="text-xs text-muted">Loading…</p>}
            {servers.error && (
                <p className="text-xs text-danger">{messageOf(servers.error)}</p>
            )}
            {servers.data && servers.data.items.length === 0 && (
                <p className="text-xs text-muted">
                    Nobody is hosting this game right now.
                </p>
            )}
            {servers.data && servers.data.items.length > 0 && (
                <div className="flex flex-col divide-y divide-border rounded-xl border border-border">
                    {servers.data.items.map((row) => (
                        <Link
                            key={row.id}
                            to={`/view/server/${row.id}`}
                            className="flex items-center gap-3 px-3 py-2 hover:bg-surface"
                        >
                            <span className="min-w-0 flex-1 truncate text-sm">
                                {row.name}
                            </span>
                            {row.server?.map && (
                                <span className="hidden truncate text-xs text-muted sm:block">
                                    {row.server.map}
                                </span>
                            )}
                            <span className="text-xs tabular-nums text-muted">
                                {row.server
                                    ? `${row.server.curUsers}/${row.server.maxUsers}`
                                    : '—'}
                            </span>
                        </Link>
                    ))}
                </div>
            )}
        </Section>
    )
}

function Leaderboards({ appId }: { appId: number }) {
    const { status } = useAuth()
    const [params, setParams] = useSearchParams()
    const [typed, setTyped] = useState('')
    const signedIn = status === 'signedIn'

    const mine = useQuery({
        queryKey: ['stats', 'me', appId],
        queryFn: () => api.statsMe(appId),
        enabled: signedIn,
    })

    const defs = useQuery({
        queryKey: ['stats', 'defs', appId],
        queryFn: () => api.statsDefs(appId),
        enabled: signedIn,
        retry: false,
    })

    // Every declared stat, with this member's figure where they hold one; the
    // held keys alone when the site cannot list a game's stats.
    const held = new Map((mine.data?.stats ?? []).map((s) => [s.key, s]))
    const keys: {
        key: string
        name: string
        value: number | null
        decimals: number
        unit: string
    }[] = defs.data
        ? defs.data.map((d) => ({
              key: d.key,
              name: d.name,
              value: held.get(d.key)?.value ?? null,
              decimals: d.decimals,
              unit: d.unit,
          }))
        : (mine.data?.stats ?? [])
    const stat = params.get('stat') ?? keys[0]?.key ?? null

    const top = useQuery({
        queryKey: ['stats', 'top', appId, stat],
        queryFn: () => api.statsTop(appId, stat ?? ''),
        enabled: signedIn && stat !== null && isStatKey(stat),
    })

    const choose = (key: string) => {
        const next = new URLSearchParams(params)
        next.set('stat', key)
        setParams(next, { replace: true })
    }

    return (
        <Section title="Leaderboards" icon={FiAward}>
            {!signedIn ? (
                <p className="text-xs text-muted">
                    <Link to="/account" className="underline">
                        Sign in
                    </Link>{' '}
                    to see this game's leaderboards and your own figures.
                </p>
            ) : (
                <>
                    <div className="flex flex-wrap items-center gap-1.5">
                        {keys.map((s) => (
                            <button
                                key={s.key}
                                type="button"
                                onClick={() => choose(s.key)}
                                aria-pressed={s.key === stat}
                                className={`rounded-lg border px-2.5 py-1 text-xs ${
                                    s.key === stat
                                        ? 'border-accent bg-accent text-accent-foreground'
                                        : 'border-border'
                                }`}
                            >
                                {s.name}
                                {s.value !== null && (
                                    <span className="ml-1.5 opacity-70">
                                        {formatStat(s.value, s.decimals, s.unit)}
                                    </span>
                                )}
                            </button>
                        ))}
                        <form
                            onSubmit={(e) => {
                                e.preventDefault()
                                const key = typed.trim()
                                if (isStatKey(key)) choose(key)
                            }}
                            className="flex items-center gap-1"
                        >
                            <input
                                value={typed}
                                onChange={(e) => setTyped(e.target.value)}
                                placeholder="stat key, e.g. kills"
                                aria-label="Stat key"
                                className="w-36 rounded-lg border border-border bg-transparent px-2 py-1 text-xs"
                            />
                            <button
                                type="submit"
                                disabled={!isStatKey(typed.trim())}
                                className="rounded-lg border border-border px-2 py-1 text-xs disabled:opacity-50"
                            >
                                Show
                            </button>
                        </form>
                    </div>

                    {mine.error && (
                        <p className="text-xs text-danger">
                            {messageOf(mine.error)}
                        </p>
                    )}
                    {mine.data && keys.length === 0 && !params.get('stat') && (
                        <p className="text-xs text-muted">
                            {defs.data
                                ? 'This game has no stats yet.'
                                : 'You have no figures in this game yet. Type a stat key to see its ranking anyway.'}
                        </p>
                    )}

                    {top.isFetching && !top.data && (
                        <p className="text-xs text-muted">Loading…</p>
                    )}
                    {top.error && (
                        <p className="text-xs text-danger">
                            {messageOf(top.error)}
                        </p>
                    )}
                    {top.data && <Ranking board={top.data} />}
                </>
            )}
        </Section>
    )
}

function Ranking({ board }: { board: StatsTopT }) {
    const { stat, rows, self } = board
    const selfShown = self !== null && rows.some((r) => r.player === self.player)

    if (rows.length === 0)
        return (
            <p className="text-xs text-muted">
                Nobody has a {stat.name} figure yet.
            </p>
        )

    return (
        <div className="overflow-hidden rounded-xl border border-border">
            <p className="border-b border-border px-3 py-1.5 text-[11px] text-muted">
                {stat.name} · {stat.players.toLocaleString()} players
            </p>
            <table className="w-full text-sm">
                <tbody className="divide-y divide-border">
                    {rows.map((r) => (
                        <tr
                            key={r.player}
                            className={
                                self?.player === r.player ? 'bg-surface-2' : ''
                            }
                        >
                            <td className="w-12 px-3 py-1.5 tabular-nums text-muted">
                                #{r.rank}
                            </td>
                            <td className="truncate px-3 py-1.5">{r.name}</td>
                            <td className="px-3 py-1.5 text-right tabular-nums">
                                {formatStat(r.value, stat.decimals, stat.unit)}
                            </td>
                        </tr>
                    ))}
                    {self && !selfShown && (
                        <tr className="bg-surface-2">
                            <td className="w-12 px-3 py-1.5 tabular-nums text-muted">
                                #{self.rank}
                            </td>
                            <td className="truncate px-3 py-1.5">
                                {self.name} (you)
                            </td>
                            <td className="px-3 py-1.5 text-right tabular-nums">
                                {formatStat(self.value, stat.decimals, stat.unit)}
                            </td>
                        </tr>
                    )}
                </tbody>
            </table>
        </div>
    )
}
