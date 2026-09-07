import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import {
    FiAlertTriangle,
    FiCheckCircle,
    FiChevronDown,
    FiChevronRight,
    FiClock,
    FiExternalLink,
    FiFileText,
} from 'react-icons/fi'

import { ipc } from '~/lib/ipc/commands'
import { messageOf } from '~/lib/ipc'
import type { SessionRowT } from '~/lib/ipc/schemas'

/**
 * **What happened the last few times a game was started.**
 *
 * Every mod manager studied for this has some version of it, and the reason is
 * always the same: a modded game that will not start is the single most common
 * thing a user needs help with, and the answer is almost always in the first
 * fifty lines the process printed before it died. Without a captured pipe those
 * lines go to a console nobody attached.
 *
 * So `session::Sessions` writes stdout and stderr to one bounded file per
 * launch, and this is where they are read back.
 *
 * WHAT AN EXIT CODE MEANS HERE, AND WHAT IT DOES NOT
 * -------------------------------------------------
 * A non-zero code is shown as a failure and a zero as a clean exit, and both
 * are only true for a launch the app SUPERVISED. A `steam://` hand-off has no
 * process of ours and therefore no code, so it is labelled as handed off rather
 * than given a green tick it did not earn — the same distinction that keeps its
 * play time out of the totals.
 *
 * A game the user closed themselves is not a failure however it exited, which
 * is why `stoppedByUser` is checked before the code is.
 */

function duration(seconds: number): string {
    if (seconds < 60) return `${seconds}s`
    if (seconds < 3600) return `${Math.round(seconds / 60)}m`

    return `${(seconds / 3600).toFixed(1)}h`
}

function verdict(row: SessionRowT): { text: string; tone: string } {
    if (row.kind === 'handoff')
        return {
            // Not a failure and not a success — the app fired a link and a
            // launcher took over. Saying anything stronger would be inventing
            // an outcome nobody observed.
            text: 'Handed to the game’s own launcher',
            tone: 'text-muted',
        }

    if (row.endedMs === null) return { text: 'Running', tone: 'text-success' }
    if (row.stoppedByUser) return { text: 'Stopped from here', tone: 'text-muted' }

    // A web session is a window that closed. There was never a process to
    // report a code, so saying one is missing describes a thing that could not
    // have existed.
    if (row.kind === 'web') return { text: 'Closed', tone: 'text-muted' }

    if (row.exitCode === null)
        return { text: 'Ended — no exit code', tone: 'text-muted' }

    if (row.exitCode === 0) return { text: 'Exited normally', tone: 'text-success' }

    return { text: `Exited with code ${row.exitCode}`, tone: 'text-danger' }
}

/**
 * The icon, derived from the same rule as the text beside it.
 *
 * It used to test `exitCode === 0 || stoppedByUser` on its own, which called
 * two entirely normal outcomes a crash: a WEB session has no exit code at all,
 * and neither does a process killed with SIGKILL — so both drew a red warning
 * triangle next to a muted "ended, no exit code". Telling somebody their game
 * crashed when they are the one who closed it is worse than saying nothing.
 *
 * A failure is now only what a supervised process reported as one.
 */
function Icon({ row }: { row: SessionRowT }) {
    if (row.kind === 'handoff')
        return <FiExternalLink className="size-3 shrink-0 text-muted" />

    if (row.stoppedByUser || row.exitCode === 0)
        return <FiCheckCircle className="size-3 shrink-0 text-success" />

    // No exit code is not a failure — see above. It is the normal state for a
    // web session and for anything the OS took down without one.
    if (row.exitCode === null)
        return <FiCheckCircle className="size-3 shrink-0 text-muted" />

    return <FiAlertTriangle className="size-3 shrink-0 text-danger" />
}

function Row({ row }: { row: SessionRowT }) {
    const [open, setOpen] = useState(false)
    const [log, setLog] = useState<string | null>(null)
    const [error, setError] = useState<string | null>(null)

    const state = verdict(row)

    const toggle = async () => {
        const next = !open

        setOpen(next)

        if (!next || log !== null) return

        try {
            setLog(await ipc.sessionLog(row.id, 400))
        } catch (err) {
            setError(messageOf(err))
        }
    }

    return (
        <li className="border-b border-border last:border-b-0">
            <button
                type="button"
                onClick={() => void toggle()}
                className="flex w-full items-center gap-2 px-3 py-2 text-left text-xs"
            >
                {open ? (
                    <FiChevronDown className="size-3 shrink-0 text-muted" />
                ) : (
                    <FiChevronRight className="size-3 shrink-0 text-muted" />
                )}

                <Icon row={row} />

                <span className="truncate font-medium">{row.label}</span>

                <span className={`shrink-0 ${state.tone}`}>{state.text}</span>

                <span className="ml-auto flex shrink-0 items-center gap-2 text-muted">
                    {/*
                     * A handoff has no duration to show. Rendering "0s" for one
                     * would put a number on a thing nobody measured.
                     */}
                    {row.kind !== 'handoff' && (
                        <span className="flex items-center gap-1">
                            <FiClock className="size-3" />
                            {duration(row.seconds)}
                        </span>
                    )}
                    <span>{new Date(row.startedMs).toLocaleString()}</span>
                </span>
            </button>

            {open && (
                <div className="px-3 pb-3">
                    {error ? (
                        <p className="text-xs text-danger">{error}</p>
                    ) : log === null ? (
                        <p className="text-xs text-muted">Reading the log…</p>
                    ) : log.trim() === '' ? (
                        <p className="text-xs text-muted">
                            {row.kind === 'handoff'
                                ? 'Nothing to show — the game was started by its own launcher, so the app never saw its output.'
                                : 'The game printed nothing before it exited.'}
                        </p>
                    ) : (
                        <pre className="selectable max-h-64 overflow-auto rounded-lg border border-border bg-surface-2 p-2 text-[11px] leading-relaxed">
                            {log}
                        </pre>
                    )}
                </div>
            )}
        </li>
    )
}

export default function SessionHistory({ appId }: { appId?: number }) {
    const history = useQuery({
        queryKey: ['sessions', 'history', appId ?? null],
        queryFn: () => ipc.sessionHistory(appId, 30),
        staleTime: 15 * 1000,
    })

    if (history.isPending)
        return <p className="text-xs text-muted">Reading recent launches…</p>

    if ((history.data?.length ?? 0) < 1)
        return (
            <p className="text-xs text-muted">
                Nothing has been launched from here yet.
            </p>
        )

    return (
        <div className="rounded-xl border border-border">
            <div className="flex items-center gap-1.5 border-b border-border px-3 py-2 text-[11px] font-medium uppercase tracking-wide text-muted">
                <FiFileText className="size-3" />
                Recent launches
            </div>

            <ul>
                {history.data?.map((row) => (
                    <Row key={row.id} row={row} />
                ))}
            </ul>
        </div>
    )
}
