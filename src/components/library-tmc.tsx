import { useCallback, useEffect, useMemo, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import {
    FiAlertTriangle,
    FiCheck,
    FiDownload,
    FiFolder,
    FiGlobe,
    FiPlay,
    FiRefreshCw,
    FiSquare,
    FiTrash2,
} from 'react-icons/fi'

import { api } from '~/lib/api/client'
import { ipc } from '~/lib/ipc/commands'
import { messageOf } from '~/lib/ipc'
import { GameIcon } from '~/components/game-icon'
import PlayDialog, { type PlayTargetT } from '~/components/play-dialog'
import type { GameStatusT, InstalledGameT, SessionT } from '~/lib/ipc/schemas'

/**
 * **Games TMC publishes**, installed on this machine.
 *
 * The third Library view, and it is a genuinely different noun from the two
 * beside it. "Games" is what somebody ELSE's launcher installed and this app
 * mods; "Subscribed" is what an account subscribed to. This is the one case
 * where the app is the distributor: it downloads a build, unpacks it under its
 * own data directory, keeps it current and starts it.
 *
 * WHY THE LIST IS A UNION AND NOT A FILTER
 * ---------------------------------------
 * Two sources, merged on the app id: the catalogue (`/apps`), which says what
 * is PUBLISHED for this machine, and the device (`games_list`), which says what
 * is here. Each without the other is a real state and both have to be visible:
 *
 *   * published and not installed is the Install button, which is the point of
 *     the screen;
 *   * installed and no longer published is a game whose build was withdrawn —
 *     it still runs, and hiding it would leave somebody with a folder they
 *     cannot uninstall from the app that put it there.
 *
 * WHY NOTHING HERE SHOWS A PROGRESS BAR
 * ------------------------------------
 * The build goes through the ordinary download queue, so the Downloads screen
 * shows it, pauses it, resumes it and rate-limits it — with the resume and the
 * checksum that queue already guarantees. A second progress bar here would be a
 * second source of truth about one transfer, and the one that drifted would be
 * the one somebody was watching.
 */

function bytes(n: number): string {
    if (n < 1024) return `${n} B`
    if (n < 1024 ** 2) return `${(n / 1024).toFixed(0)} KB`
    if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(0)} MB`

    return `${(n / 1024 ** 3).toFixed(1)} GB`
}

/** One row: a published game, an installed one, or both. */
type Row = {
    appId: number
    name: string
    slug: string | null
    icon: string | null
    /** What the catalogue says is available for this machine, if anything. */
    publishedVersion: string | null
    publishedSize: number | null
    installed: InstalledGameT | null
    /** Only known after a check — see `gamesCheckUpdates`. */
    status: GameStatusT | null
}

export default function LibraryTmcGames() {
    const [busy, setBusy] = useState<number | null>(null)
    const [error, setError] = useState<string | null>(null)
    const [play, setPlay] = useState<PlayTargetT | null>(null)
    const [running, setRunning] = useState<SessionT[]>([])
    const [installed, setInstalled] = useState<InstalledGameT[]>([])
    const [statuses, setStatuses] = useState<Record<number, GameStatusT>>({})
    const [checking, setChecking] = useState(false)

    /*
     * What this machine can do, asked once. It is a compile-time fact in Rust —
     * the binary that is asking IS the evidence of its own architecture — so it
     * cannot change while the app is open.
     */
    const platform = useQuery({
        queryKey: ['games', 'platform'],
        queryFn: () => ipc.gamesPlatform(),
        staleTime: Infinity,
    })

    const refresh = useCallback(async () => {
        try {
            setInstalled(await ipc.gamesList())
            setRunning(await ipc.sessionRunning())
        } catch (err) {
            setError(messageOf(err))
        }
    }, [])

    useEffect(() => {
        void refresh()
    }, [refresh])

    /*
     * The catalogue, narrowed SERVER-side to apps with a build. `playable` is
     * not the filter: a game can be perfectly playable in the browser window
     * and have nothing to install, and vice versa.
     *
     * Narrowed there rather than here because the alternative — filtering a
     * page of the whole catalogue — makes an installable game past the last row
     * of that page invisible, with nothing on screen to suggest it exists. The
     * server narrows to a build on ANY platform and the merge below drops the
     * ones with none for this machine, which is the safe direction of that
     * asymmetry.
     */
    const catalogue = useQuery({
        queryKey: ['apps', { installable: true }],
        queryFn: () => api.apps({ installable: true, limit: 100, sort: 'name' }),
        staleTime: 5 * 60 * 1000,
    })

    const target = platform.data?.platform ?? null

    const rows = useMemo<Row[]>(() => {
        const byId = new Map<number, Row>()

        for (const app of catalogue.data?.apps ?? []) {
            const build = target
                ? app.install?.builds.find((b) => b.platform === target)
                : undefined

            if (!build) continue

            byId.set(app.id, {
                appId: app.id,
                name: app.name,
                slug: app.slug,
                icon: app.images.icon,
                publishedVersion: build.version,
                publishedSize: build.sizeBytes,
                installed: null,
                status: statuses[app.id] ?? null,
            })
        }

        for (const game of installed) {
            const existing = byId.get(game.appId)

            if (existing) {
                existing.installed = game
                continue
            }

            // Installed, and no longer published for this machine. It still
            // runs and it still has to be removable from here.
            byId.set(game.appId, {
                appId: game.appId,
                name: game.name,
                slug: game.slug ?? null,
                icon: null,
                publishedVersion: null,
                publishedSize: null,
                installed: game,
                status: statuses[game.appId] ?? null,
            })
        }

        return [...byId.values()].sort((a, b) => {
            // Installed first — this is a Library, so what somebody HAS comes
            // before what they could have.
            if (Boolean(a.installed) !== Boolean(b.installed))
                return a.installed ? -1 : 1

            return a.name.localeCompare(b.name)
        })
    }, [catalogue.data, installed, statuses, target])

    const act = async (appId: number, run: () => Promise<unknown>) => {
        setBusy(appId)
        setError(null)

        try {
            await run()
            await refresh()
        } catch (err) {
            setError(messageOf(err))
        } finally {
            setBusy(null)
        }
    }

    const check = async () => {
        setChecking(true)
        setError(null)

        try {
            const found = await ipc.gamesCheckUpdates()

            setStatuses(Object.fromEntries(found.map((s) => [s.appId, s])))
        } catch (err) {
            setError(messageOf(err))
        } finally {
            setChecking(false)
        }
    }

    if (platform.data && !platform.data.installable)
        return (
            <Notice>
                {platform.data.platform
                    ? 'Games for this platform are installed through its own store, not by the app. You can still play them in a window from the Apps tab.'
                    : 'The app does not publish games for this kind of machine. You can still play them in a window from the Apps tab.'}
            </Notice>
        )

    return (
        <div className="flex flex-col gap-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
                <p className="text-[11px] text-muted">
                    Games TMC publishes, installed on this machine and kept up to
                    date.
                </p>

                <button
                    type="button"
                    onClick={() => void check()}
                    disabled={checking || installed.length === 0}
                    className="flex items-center gap-1.5 rounded-lg border border-border px-2.5 py-1.5 text-xs disabled:opacity-50"
                >
                    <FiRefreshCw
                        className={`size-3 ${checking ? 'animate-spin' : ''}`}
                    />
                    Check for updates
                </button>
            </div>

            {error && (
                <div className="flex items-start gap-2 rounded-lg border border-danger/40 bg-danger/10 p-2.5 text-[11px] text-danger">
                    <FiAlertTriangle className="mt-0.5 size-3 shrink-0" />
                    <span>{error}</span>
                </div>
            )}

            {rows.length === 0 ? (
                <Notice>
                    {catalogue.isLoading
                        ? 'Looking for games…'
                        : 'No games are published for this machine yet.'}
                </Notice>
            ) : (
                <div className="flex flex-col gap-2">
                    {rows.map((row) => (
                        <GameRow
                            key={row.appId}
                            row={row}
                            busy={busy === row.appId}
                            running={
                                running.find((s) => s.appId === row.appId) ?? null
                            }
                            onInstall={() =>
                                void act(row.appId, () =>
                                    ipc.gameInstall({
                                        appId: row.appId,
                                        name: row.name,
                                        slug: row.slug ?? undefined,
                                    })
                                )
                            }
                            onUninstall={() =>
                                void act(row.appId, () =>
                                    ipc.gameUninstall(row.appId)
                                )
                            }
                            onAutoUpdate={(on) =>
                                void act(row.appId, () =>
                                    ipc.gameSetAutoUpdate(row.appId, on)
                                )
                            }
                            onPlay={() =>
                                setPlay({
                                    appId: row.appId,
                                    fallback: {
                                        name: row.name,
                                        slug: row.slug,
                                        icon: row.icon,
                                    },
                                })
                            }
                            onStop={(session) =>
                                void act(row.appId, () =>
                                    ipc.sessionStop(session.id)
                                )
                            }
                        />
                    ))}
                </div>
            )}

            {play && (
                <PlayDialog
                    target={play}
                    onClose={() => {
                        setPlay(null)
                        void refresh()
                    }}
                />
            )}
        </div>
    )
}

function Notice({ children }: { children: React.ReactNode }) {
    return (
        <div className="rounded-xl border border-border p-6 text-center">
            <p className="text-sm text-muted">{children}</p>
        </div>
    )
}

function GameRow({
    row,
    busy,
    running,
    onInstall,
    onUninstall,
    onAutoUpdate,
    onPlay,
    onStop,
}: {
    row: Row
    busy: boolean
    running: SessionT | null
    onInstall: () => void
    onUninstall: () => void
    onAutoUpdate: (on: boolean) => void
    onPlay: () => void
    onStop: (session: SessionT) => void
}) {
    const installed = row.installed

    /*
     * Whether an update exists is RUST's answer, never a comparison made here.
     * `1.10.0` sorts before `1.9.0` as a string, and an updater built on a
     * string compare either never fires or never stops — so the whole app has
     * one implementation of this, in `tmc_core::version`.
     */
    const updatable = row.status?.updateAvailable ?? false

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

                        {updatable && (
                            <span className="rounded-full bg-accent/15 px-2 py-0.5 text-[10px] font-medium text-accent">
                                Update to {row.status?.available}
                            </span>
                        )}
                    </div>

                    <p className="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[11px] text-muted">
                        {installed ? (
                            <span className="flex items-center gap-1">
                                <FiCheck className="size-3" />
                                Installed {installed.version}
                            </span>
                        ) : (
                            <span className="flex items-center gap-1">
                                <FiGlobe className="size-3" />
                                {row.publishedVersion}
                                {row.publishedSize
                                    ? ` · ${bytes(row.publishedSize)}`
                                    : ''}
                            </span>
                        )}

                        {installed && !row.publishedVersion && (
                            <span
                                className="flex items-center gap-1"
                                title="This build is no longer published for this machine. It still runs, and you can still remove it from here."
                            >
                                <FiAlertTriangle className="size-3" />
                                No longer published
                            </span>
                        )}
                    </p>

                    {installed && (
                        <p className="mt-0.5 flex items-center gap-1 truncate text-[11px] text-muted">
                            <FiFolder className="size-3 shrink-0" />
                            <span className="selectable truncate">
                                {installed.dir}
                            </span>
                        </p>
                    )}

                    {row.status?.notes && updatable && (
                        <p className="mt-1 text-[11px] text-muted">
                            {row.status.notes}
                        </p>
                    )}
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
                    ) : installed ? (
                        <button
                            type="button"
                            onClick={onPlay}
                            disabled={busy}
                            className="flex items-center gap-1.5 rounded-lg bg-accent px-2.5 py-1 text-[11px] font-semibold text-accent-foreground disabled:opacity-50"
                        >
                            <FiPlay className="size-3" />
                            Play
                        </button>
                    ) : null}

                    {(!installed || updatable) && row.publishedVersion && (
                        <button
                            type="button"
                            onClick={onInstall}
                            disabled={busy || Boolean(running)}
                            className={`flex items-center gap-1.5 rounded-lg px-2.5 py-1 text-[11px] font-semibold disabled:opacity-50 ${
                                installed
                                    ? 'border border-border'
                                    : 'bg-accent text-accent-foreground'
                            }`}
                        >
                            <FiDownload className="size-3" />
                            {busy ? 'Working…' : installed ? 'Update' : 'Install'}
                        </button>
                    )}

                    {installed && (
                        <>
                            <label className="flex cursor-pointer items-center gap-1.5 text-[11px] text-muted">
                                <input
                                    type="checkbox"
                                    checked={installed.autoUpdate}
                                    onChange={(e) => onAutoUpdate(e.target.checked)}
                                    className="size-3 accent-[var(--color-accent)]"
                                />
                                Auto-update
                            </label>

                            <button
                                type="button"
                                onClick={onUninstall}
                                disabled={busy || Boolean(running)}
                                className="flex items-center gap-1.5 rounded-lg border border-border px-2.5 py-1 text-[11px] text-danger disabled:opacity-50"
                            >
                                <FiTrash2 className="size-3" />
                                Remove
                            </button>
                        </>
                    )}
                </div>
            </div>
        </article>
    )
}
