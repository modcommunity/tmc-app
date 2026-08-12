import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Button } from '@modcommunity/shared'

import { ipc } from '~/lib/ipc/commands'
import type { LogEntryT, LogLevelT } from '~/lib/ipc/schemas'
import { Select } from '~/components/form'

/**
 * The activity log.
 *
 * Everything a plugin did, every file an install wrote, every host it reached,
 * every permission granted — the record a user needs to answer "what did this
 * thing actually do to my machine?".
 *
 * `Security` entries are recorded whatever the app's logging switch says, and
 * the filter defaults to showing them, so opening this pane for the first time
 * shows the decisions rather than a wall of routine chatter.
 */

const LEVEL_STYLES: Record<LogLevelT, string> = {
    debug: 'text-muted',
    info: 'text-foreground',
    security: 'text-accent',
    warn: 'text-warning',
    error: 'text-danger',
}

const LEVEL_OPTIONS: { value: LogLevelT | 'all'; label: string }[] = [
    { value: 'security', label: 'Security and above' },
    { value: 'all', label: 'Everything' },
    { value: 'warn', label: 'Warnings and errors' },
    { value: 'error', label: 'Errors only' },
]

export default function LoggingRoute() {
    const queryClient = useQueryClient()
    const [level, setLevel] = useState<LogLevelT | 'all'>('security')
    const [confirmClear, setConfirmClear] = useState(false)

    const entries = useQuery({
        queryKey: ['log', level],
        queryFn: () => ipc.logRead(500, level === 'all' ? undefined : level),
        // The install running in another pane appends to this file, so a stale
        // view is the common case rather than the exception.
        refetchInterval: 5_000,
    })

    const path = useQuery({ queryKey: ['log-path'], queryFn: () => ipc.logPath() })

    return (
        <div className="flex flex-col gap-4">
            <div className="flex flex-wrap items-center justify-between gap-2">
                <Select
                    label="Log level"
                    value={level}
                    options={LEVEL_OPTIONS}
                    onChange={setLevel}
                />

                {confirmClear ? (
                    <div className="flex items-center gap-2 text-xs">
                        <span className="text-muted">Clear the log?</span>
                        <Button
                            btnType="danger"
                            onClick={() => {
                                void ipc.logClear().then(() => {
                                    setConfirmClear(false)
                                    void queryClient.invalidateQueries({
                                        queryKey: ['log'],
                                    })
                                })
                            }}
                        >
                            Clear
                        </Button>
                        <Button
                            btnType="secondary"
                            onClick={() => setConfirmClear(false)}
                        >
                            Keep
                        </Button>
                    </div>
                ) : (
                    <Button
                        btnType="secondary"
                        onClick={() => setConfirmClear(true)}
                    >
                        Clear log
                    </Button>
                )}
            </div>

            {path.data && (
                <p className="selectable break-all text-xs text-muted">
                    Stored at {path.data}
                </p>
            )}

            {entries.isPending ? (
                <p className="text-sm text-muted">Loading…</p>
            ) : (entries.data?.length ?? 0) === 0 ? (
                <p className="py-8 text-center text-sm text-muted">
                    Nothing recorded at this level yet.
                </p>
            ) : (
                <ul className="flex flex-col divide-y divide-border overflow-hidden rounded-xl border border-border bg-surface">
                    {entries.data?.map((entry, index) => (
                        <LogRow key={`${entry.at}-${index}`} entry={entry} />
                    ))}
                </ul>
            )}
        </div>
    )
}

function LogRow({ entry }: { entry: LogEntryT }) {
    return (
        <li className="selectable flex flex-col gap-0.5 px-3 py-2 text-xs">
            <div className="flex flex-wrap items-center gap-2">
                <span className="tabular-nums text-muted">
                    {new Date(entry.at).toLocaleString()}
                </span>
                <span
                    className={`rounded px-1.5 py-0.5 text-[0.65rem] uppercase ${LEVEL_STYLES[entry.level]}`}
                >
                    {entry.level}
                </span>
                <span className="rounded bg-surface-secondary px-1.5 py-0.5 text-[0.65rem] text-muted">
                    {entry.scope}
                </span>
                {entry.plugin && (
                    <span className="truncate text-[0.65rem] text-accent">
                        {entry.plugin}
                    </span>
                )}
            </div>

            <p className="break-words">{entry.message}</p>
        </li>
    )
}
