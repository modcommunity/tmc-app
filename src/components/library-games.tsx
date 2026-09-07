import { useCallback, useEffect, useMemo, useState } from 'react'
import { Link } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import {
    FiAlertTriangle,
    FiClock,
    FiFolder,
    FiLayers,
    FiPlay,
    FiPlus,
    FiSearch,
    FiShare2,
    FiSquare,
    FiTerminal,
} from 'react-icons/fi'

import { api } from '~/lib/api/client'
import { ipc } from '~/lib/ipc/commands'
import { messageOf } from '~/lib/ipc'
import { useSettings } from '~/lib/settings/provider'
import { useAppRefs } from '~/lib/hooks/use-app-icons'
import { GameIcon } from '~/components/game-icon'
import SandboxEditor from '~/components/sandbox-editor'
import ScanDialog from '~/components/scan-dialog'
import PlayDialog, { type PlayTargetT } from '~/components/play-dialog'
import SessionHistory from '~/components/session-history'
import { ExportSandbox, ImportSandbox } from '~/components/sandbox-share'
import type { SandboxRowT, SessionT } from '~/lib/ipc/schemas'

/**
 * **The games on this machine**, and the sandboxes each one has.
 *
 * The Library used to be a list of subscriptions, which answered "what did I
 * subscribe to" and not "what have I got installed" — and the second is the
 * question somebody opens a mod manager to answer. Both are here now, on two
 * views, and this is the one that opens.
 *
 * WHAT COUNTS AS "ON THIS MACHINE"
 * -------------------------------
 * A game with a folder configured, or a game with a sandbox. The union rather
 * than either alone, because they are different halves of the same fact and
 * each without the other is a real state:
 *
 *   * a folder and no sandbox is a game that was just found by a scan, and the
 *     row is where somebody adds their first profile to it;
 *   * a sandbox and no folder is a profile restored from the account on a new
 *     machine, and the row is where they point it at the game.
 *
 * Showing only one of the two would make one of those invisible, and the user
 * would be left with a screen that does not mention the thing they are looking
 * for.
 *
 * PLAY TIME IS MEASURED, NOT GUESSED
 * ---------------------------------
 * The totals come from `game_session`, which only records a duration for a
 * launch the app actually supervised. A `steam://` hand-off contributes zero —
 * see `tmc_core::session` — so a game played entirely through Steam shows no
 * hours here. That is the honest answer, and it is why the row says "not
 * measured" rather than "0h" for one.
 */

function duration(seconds: number): string {
    if (seconds < 60) return `${seconds}s`
    if (seconds < 3600) return `${Math.round(seconds / 60)}m`

    return `${(seconds / 3600).toFixed(1)}h`
}

function since(ms: number): string {
    const days = Math.floor((Date.now() - ms) / 86_400_000)

    if (days < 1) return 'today'
    if (days === 1) return 'yesterday'
    if (days < 30) return `${days} days ago`

    return new Date(ms).toLocaleDateString()
}

type GameRow = {
    appId: number
    name: string
    icon: string | null
    slug: string | null
    dir: string | null
    sandboxes: SandboxRowT[]
}

function GameCard({
    row,
    running,
    playtime,
    onEdit,
    onCreate,
    onPlay,
    onStop,
}: {
    row: GameRow
    running: SessionT | null
    playtime: {
        seconds: number
        launches: number
        lastPlayedMs: number | null
    } | null
    onEdit: (sandbox: SandboxRowT) => void
    onCreate: () => void
    onPlay: () => void
    onStop: (session: SessionT) => void
}) {
    return (
        <article className="rounded-xl border border-border bg-surface">
            <div className="flex items-start gap-3 p-3">
                <GameIcon
                    app={{ id: row.appId, name: row.name, icon: row.icon }}
                    size="lg"
                />

                <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                        <h3 className="truncate text-sm font-semibold">
                            {row.name}
                        </h3>

                        {running && (
                            <span className="flex items-center gap-1 rounded-full bg-success/15 px-2 py-0.5 text-[10px] font-medium text-success">
                                <span className="size-1.5 rounded-full bg-success" />
                                Running
                            </span>
                        )}
                    </div>

                    <p className="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[11px] text-muted">
                        <span className="flex items-center gap-1">
                            <FiLayers className="size-3" />
                            {row.sandboxes.length}{' '}
                            {row.sandboxes.length === 1 ? 'sandbox' : 'sandboxes'}
                        </span>

                        {playtime && playtime.seconds > 0 ? (
                            <span className="flex items-center gap-1">
                                <FiClock className="size-3" />
                                {duration(playtime.seconds)} played
                                {playtime.lastPlayedMs
                                    ? ` · ${since(playtime.lastPlayedMs)}`
                                    : ''}
                            </span>
                        ) : (
                            <span
                                className="flex items-center gap-1"
                                title="Play time is only measured for launches the app supervised. A game started through Steam's own link is started by Steam."
                            >
                                <FiClock className="size-3" />
                                Not measured
                            </span>
                        )}
                    </p>

                    <p className="mt-0.5 flex items-center gap-1 truncate text-[11px] text-muted">
                        <FiFolder className="size-3 shrink-0" />
                        <span className="selectable truncate">
                            {row.dir ?? (
                                <Link to="/settings/games" className="underline">
                                    No folder set for this game
                                </Link>
                            )}
                        </span>
                    </p>
                </div>

                <div className="flex shrink-0 flex-col items-end gap-1.5">
                    {running ? (
                        <button
                            type="button"
                            onClick={() => onStop(running)}
                            className="flex items-center gap-1.5 rounded-lg border border-border px-2.5 py-1 text-[11px] text-danger"
                        >
                            <FiSquare className="size-3" />
                            Stop
                        </button>
                    ) : (
                        <button
                            type="button"
                            onClick={onPlay}
                            className="flex items-center gap-1.5 rounded-lg bg-accent px-2.5 py-1 text-[11px] font-semibold text-accent-foreground"
                        >
                            <FiPlay className="size-3" />
                            Play
                        </button>
                    )}

                    <button
                        type="button"
                        onClick={onCreate}
                        className="flex items-center gap-1.5 rounded-lg border border-border px-2.5 py-1 text-[11px]"
                    >
                        <FiPlus className="size-3" />
                        Sandbox
                    </button>
                </div>
            </div>

            {row.sandboxes.length > 0 && (
                <ul className="border-t border-border">
                    {row.sandboxes.map((sandbox) => (
                        <li
                            key={sandbox.id}
                            className="flex items-center gap-2 border-b border-border px-3 py-2 text-xs last:border-b-0"
                        >
                            <FiLayers className="size-3 shrink-0 text-muted" />

                            <Link
                                to={`/sandboxes/${sandbox.id}`}
                                className="truncate font-medium hover:text-accent"
                            >
                                {sandbox.name}
                            </Link>

                            {sandbox.isDefault && (
                                <span className="shrink-0 rounded bg-surface-2 px-1 text-[10px] text-muted">
                                    default
                                </span>
                            )}

                            <span className="shrink-0 text-[11px] text-muted">
                                {sandbox.mods.length}{' '}
                                {sandbox.mods.length === 1 ? 'mod' : 'mods'}
                            </span>

                            {sandbox.needsDeploy && (
                                <span
                                    className="shrink-0 text-[11px] text-warning"
                                    title="Its staged files are not in the game folder yet."
                                >
                                    needs deploying
                                </span>
                            )}

                            {!sandbox.cloudSync && (
                                <span
                                    className="shrink-0 text-[11px] text-muted"
                                    title="Kept off the account. It exists only on this device."
                                >
                                    local only
                                </span>
                            )}

                            <div className="ml-auto flex shrink-0 items-center gap-1">
                                <ExportSandbox
                                    id={sandbox.id}
                                    name={sandbox.name}
                                />

                                <button
                                    type="button"
                                    onClick={() => onEdit(sandbox)}
                                    className="rounded-lg border border-border px-2 py-0.5 text-[11px]"
                                >
                                    Edit
                                </button>
                            </div>
                        </li>
                    ))}
                </ul>
            )}
        </article>
    )
}

export default function LibraryGames() {
    const { app, reloadApp } = useSettings()
    const refFor = useAppRefs()

    const [search, setSearch] = useState('')
    const [scanning, setScanning] = useState(false)
    const [importing, setImporting] = useState(false)
    const [editing, setEditing] = useState<{
        appId: number
        appSlug: string | null
        appName: string | null
        existing: SandboxRowT | null
    } | null>(null)
    const [target, setTarget] = useState<PlayTargetT | null>(null)
    const [error, setError] = useState<string | null>(null)

    const sandboxes = useQuery({
        queryKey: ['sandboxes'],
        queryFn: () => ipc.sandboxList(),
        staleTime: 10 * 1000,
    })

    const playtime = useQuery({
        queryKey: ['playtime'],
        queryFn: () => ipc.playtimeSummary(),
        staleTime: 30 * 1000,
    })

    /*
     * Polled rather than pushed. A game exits on its own schedule and the app
     * has no event for it — the supervisor closes the session on a thread with
     * no webview to notify. Five seconds is well inside the time it takes
     * somebody to look back at this screen, and one poll of an in-memory list
     * costs nothing.
     */
    const running = useQuery({
        queryKey: ['sessions', 'running'],
        queryFn: () => ipc.sessionRunning(),
        refetchInterval: 5_000,
    })

    const catalogue = useQuery({
        queryKey: ['apps', { limit: 100, sort: 'players' }],
        queryFn: () => api.apps({ limit: 100 }),
        staleTime: 5 * 60 * 1000,
    })

    /*
     * Play totals move only when a session ends, and the running poll is the
     * only thing that notices — the supervisor closes a session on a thread
     * with no webview to notify, so there is no event to listen for. Keying off
     * the COUNT catches both ends of a launch and costs one query per change
     * rather than one per poll.
     */
    const runningCount = running.data?.length ?? 0

    useEffect(() => {
        void playtime.refetch()
        // `playtime` is a stable query object; depending on it would re-run
        // this on its own result.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [runningCount])

    const refresh = useCallback(() => {
        void sandboxes.refetch()
        void playtime.refetch()

        /*
         * And the SETTINGS, because applying a scan result writes `gameDirs` in
         * Rust rather than through the settings provider — it has to, since
         * each folder is validated as a jail anchor on the way in. Without this
         * the rows below are still built from the `gameDirs` this screen loaded
         * at mount, so a fresh machine that just found a dozen games sees none
         * of them until the app is restarted.
         */
        void reloadApp()
    }, [sandboxes, playtime, reloadApp])

    const rows = useMemo<GameRow[]>(() => {
        const byApp = new Map<number, GameRow>()

        const ensure = (
            appId: number,
            slug: string | null,
            name?: string | null
        ) => {
            const existing = byApp.get(appId)

            if (existing) {
                if (!existing.slug && slug) existing.slug = slug

                return existing
            }

            const ref = refFor(appId)

            const created: GameRow = {
                appId,
                name: name ?? ref?.name ?? `Game ${appId}`,
                icon: ref?.icon ?? null,
                // A game known only by its configured folder has no local row
                // to carry a slug, and a sandbox created without one gets no
                // game rules at all — no preset, no install rule, no launch.
                slug: slug ?? ref?.slug ?? null,
                dir: app?.gameDirs[String(appId)] ?? null,
                sandboxes: [],
            }

            byApp.set(appId, created)

            return created
        }

        for (const sandbox of sandboxes.data ?? []) {
            ensure(sandbox.appId, sandbox.appSlug, sandbox.appName).sandboxes.push(
                sandbox
            )
        }

        // A configured folder is a game on this machine even with no sandbox —
        // it is what a fresh scan produces, and this row is where the first
        // profile gets added to it.
        for (const key of Object.keys(app?.gameDirs ?? {})) {
            const appId = Number(key)

            if (Number.isFinite(appId)) ensure(appId, null)
        }

        const term = search.trim().toLowerCase()

        return [...byApp.values()]
            .filter((row) => !term || row.name.toLowerCase().includes(term))
            .sort((a, b) => {
                // Most recently played first, then alphabetical. Somebody
                // opening this screen is usually going back to what they were
                // last doing.
                const aLast =
                    playtime.data?.byApp[String(a.appId)]?.lastPlayedMs ?? 0
                const bLast =
                    playtime.data?.byApp[String(b.appId)]?.lastPlayedMs ?? 0

                if (aLast !== bLast) return bLast - aLast

                return a.name.localeCompare(b.name)
            })
    }, [sandboxes.data, app?.gameDirs, refFor, search, playtime.data])

    /**
     * End a running session, by whichever means actually ends it.
     *
     * A web session has no process to signal — `Sessions::stop` would close the
     * session record and leave the player WINDOW open, which is a game still
     * running with the app convinced it is not. The window is the thing to
     * close, and Rust ends the session when it is destroyed.
     */
    const stop = async (session: SessionT) => {
        setError(null)

        try {
            if (session.kind === 'web') await ipc.playClose()
            else await ipc.sessionStop(session.id)

            await running.refetch()
        } catch (err) {
            setError(messageOf(err))
        }
    }

    /*
     * The catalogue row when this page's own fetch happened to include it, and
     * an id plus what is known locally otherwise — the dialog looks the rest up
     * itself. Passing the row when we have it saves a request; passing the id
     * when we do not is what makes a game outside the first page of the
     * catalogue, or a device that is offline, still launchable.
     */
    const play = (row: GameRow) =>
        setTarget({
            appId: row.appId,
            app: catalogue.data?.apps.find((a) => a.id === row.appId),
            fallback: { name: row.name, icon: row.icon, slug: row.slug },
        })

    return (
        <div className="flex flex-col gap-3">
            <div className="flex flex-wrap items-center gap-2">
                <div className="relative min-w-40 flex-1">
                    <FiSearch className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted" />
                    <input
                        type="search"
                        value={search}
                        onChange={(e) => setSearch(e.target.value)}
                        placeholder="Search your games"
                        className="w-full rounded-lg border border-border bg-surface-2 py-1.5 pl-8 pr-2 text-xs"
                    />
                </div>

                <button
                    type="button"
                    onClick={() => setScanning(true)}
                    className="flex items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-xs"
                >
                    <FiSearch className="size-3.5" />
                    Find games
                </button>

                <button
                    type="button"
                    onClick={() => setImporting(true)}
                    className="flex items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-xs"
                >
                    <FiShare2 className="size-3.5" />
                    Import a code
                </button>

                <Link
                    to="/rcon"
                    className="flex items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-xs"
                >
                    <FiTerminal className="size-3.5" />
                    Console
                </Link>
            </div>

            {error && (
                <p className="flex items-start gap-2 rounded-lg border border-danger/40 bg-danger/10 p-2 text-xs text-danger">
                    <FiAlertTriangle className="mt-0.5 size-3.5 shrink-0" />
                    {error}
                </p>
            )}

            {sandboxes.isPending ? (
                <p className="text-xs text-muted">Reading your library…</p>
            ) : rows.length < 1 ? (
                <div className="flex flex-col items-center gap-2 rounded-xl border border-border p-8 text-center">
                    <FiLayers className="size-6 text-muted" />
                    <p className="text-sm">No games set up on this device yet.</p>
                    <p className="max-w-sm text-xs text-muted">
                        Let the app read your launchers, or point it at the folders
                        your games are in. Nothing is changed until you pick what to
                        set up.
                    </p>
                    <button
                        type="button"
                        onClick={() => setScanning(true)}
                        className="mt-1 rounded-lg bg-accent px-4 py-2 text-xs font-semibold text-accent-foreground"
                    >
                        Find my games
                    </button>
                </div>
            ) : (
                <div className="flex flex-col gap-3">
                    {rows.map((row) => (
                        <GameCard
                            key={row.appId}
                            row={row}
                            running={
                                running.data?.find((s) => s.appId === row.appId) ??
                                null
                            }
                            playtime={
                                playtime.data?.byApp[String(row.appId)] ?? null
                            }
                            onEdit={(sandbox) =>
                                setEditing({
                                    appId: row.appId,
                                    appSlug: row.slug,
                                    appName: row.name,
                                    existing: sandbox,
                                })
                            }
                            onCreate={() =>
                                setEditing({
                                    appId: row.appId,
                                    appSlug: row.slug,
                                    appName: row.name,
                                    existing: null,
                                })
                            }
                            onPlay={() => play(row)}
                            onStop={(session) => void stop(session)}
                        />
                    ))}
                </div>
            )}

            {rows.length > 0 && (
                /*
                 * Below the games rather than on its own screen: a modded game
                 * that will not start is what somebody comes here to work out,
                 * and the launch that failed is the row above this one.
                 */
                <SessionHistory />
            )}

            {importing && (
                <ImportSandbox
                    onClose={() => setImporting(false)}
                    onImported={refresh}
                />
            )}

            {scanning && (
                <ScanDialog
                    onClose={() => setScanning(false)}
                    onApplied={refresh}
                />
            )}

            {editing && (
                <SandboxEditor
                    appId={editing.appId}
                    appSlug={editing.appSlug}
                    appName={editing.appName}
                    existing={editing.existing}
                    onClose={() => setEditing(null)}
                    onSaved={refresh}
                />
            )}

            {target && (
                <PlayDialog target={target} onClose={() => setTarget(null)} />
            )}
        </div>
    )
}
