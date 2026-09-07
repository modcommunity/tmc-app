import { useMemo, useState } from 'react'
import {
    FiAlertCircle,
    FiCheck,
    FiChevronDown,
    FiChevronUp,
    FiPause,
    FiPlay,
    FiTrash2,
    FiX,
} from 'react-icons/fi'

import {
    formatBytes,
    formatEta,
    formatSpeed,
    useDownloads,
} from '~/lib/downloads/provider'
import { type DownloadT } from '~/lib/ipc/schemas'
import Select from '~/components/select'
import { ItemThumb } from '~/components/item-thumb'
import { useLibrary } from '~/lib/library/provider'
import SpeedGraph from '~/components/speed-graph'

/**
 * The download queue.
 *
 * WHAT THIS SCREEN IS FOR
 * -----------------------
 * Not "is it done yet" — a badge answers that. It is for the moment something
 * has gone wrong or is taking too long: which file is stuck, why it failed,
 * what to do about it. So every row carries its error text, its attempt count
 * and its own speed history, and every action that could help is one click
 * away rather than behind a menu.
 *
 * THE GLOBAL LIMIT IS AT THE TOP
 * ------------------------------
 * Because the reason somebody opens this screen mid-download is usually that
 * the app is eating the connection. Putting the throttle in Settings would mean
 * navigating away from the thing they are trying to slow down.
 */

/** The presets the limit dropdown offers, in bytes per second. */
const LIMITS: { value: string; label: string }[] = [
    { value: '0', label: 'No limit' },
    { value: String(256 * 1024), label: '256 KB/s' },
    { value: String(512 * 1024), label: '512 KB/s' },
    { value: String(1024 * 1024), label: '1 MB/s' },
    { value: String(2 * 1024 * 1024), label: '2 MB/s' },
    { value: String(5 * 1024 * 1024), label: '5 MB/s' },
    { value: String(10 * 1024 * 1024), label: '10 MB/s' },
    { value: String(25 * 1024 * 1024), label: '25 MB/s' },
    { value: String(50 * 1024 * 1024), label: '50 MB/s' },
]

export default function DownloadsRoute() {
    const queue = useDownloads()
    const [showFinished, setShowFinished] = useState(false)

    const { active, finished } = useMemo(() => {
        const active: DownloadT[] = []
        const finished: DownloadT[] = []

        for (const download of queue.downloads) {
            if (download.status === 'done' || download.status === 'cancelled')
                finished.push(download)
            else active.push(download)
        }

        return { active, finished }
    }, [queue.downloads])

    const remaining = active.reduce(
        (sum, d) => sum + Math.max(0, (d.total ?? 0) - d.done),
        0
    )

    return (
        <div className="flex flex-col gap-4 p-4">
            <header className="flex flex-wrap items-center justify-between gap-3">
                <div>
                    <h1 className="text-lg font-bold">Downloads</h1>
                    <p className="text-xs text-muted">
                        {active.length === 0
                            ? 'Nothing in the queue.'
                            : `${active.length} in the queue · ${formatSpeed(
                                  queue.speedBps
                              )}${
                                  remaining > 0
                                      ? ` · ${formatBytes(remaining)} to go`
                                      : ''
                              }`}
                    </p>
                </div>

                <div className="flex items-center gap-2">
                    <span className="text-xs text-muted">Speed limit</span>
                    <Select
                        label="Global download speed limit"
                        value={String(queue.limitBps ?? 0)}
                        onChange={(next) =>
                            void queue.setGlobalLimit(
                                Number(next) > 0 ? Number(next) : null
                            )
                        }
                        options={LIMITS}
                    />
                </div>
            </header>

            {active.length === 0 && finished.length === 0 && (
                <div className="rounded-xl border border-dashed border-border p-8 text-center">
                    <p className="text-sm">Nothing has been downloaded yet.</p>
                    <p className="mt-1 text-xs text-muted">
                        Files queue up here when you install a mod or stage a
                        sandbox. You can pause, reorder and throttle them without
                        leaving this screen.
                    </p>
                </div>
            )}

            {active.length > 0 && (
                <section className="flex flex-col gap-2">
                    {active.map((download) => (
                        <DownloadCard key={download.id} download={download} />
                    ))}
                </section>
            )}

            {finished.length > 0 && (
                <section className="flex flex-col gap-2">
                    <div className="flex items-center justify-between">
                        <button
                            type="button"
                            onClick={() => setShowFinished((was) => !was)}
                            className="flex items-center gap-1.5 text-xs font-medium text-muted"
                        >
                            {showFinished ? (
                                <FiChevronUp className="size-3.5" />
                            ) : (
                                <FiChevronDown className="size-3.5" />
                            )}
                            Finished ({finished.length})
                        </button>

                        <button
                            type="button"
                            onClick={() => void queue.clearFinished()}
                            className="flex items-center gap-1.5 rounded-lg border border-border px-2 py-1 text-xs hover:border-accent"
                        >
                            <FiTrash2 className="size-3" />
                            Clear
                        </button>
                    </div>

                    {showFinished &&
                        finished.map((download) => (
                            <DownloadCard key={download.id} download={download} />
                        ))}
                </section>
            )}
        </div>
    )
}

function DownloadCard({ download }: { download: DownloadT }) {
    const queue = useDownloads()
    const library = useLibrary()
    const [open, setOpen] = useState(false)

    const percent =
        download.total && download.total > 0
            ? Math.min(100, (download.done / download.total) * 100)
            : null

    const failed = download.status === 'failed'
    const done = download.status === 'done'
    const running = download.status === 'running'

    return (
        <article
            className={`rounded-xl border bg-surface p-3 ${
                failed ? 'border-danger/50' : 'border-border'
            }`}
        >
            <div className="flex items-start gap-3">
                <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                        {/*
                         * The item's cover, found in the LOCAL library by the
                         * key Rust put in `meta.item`. A queue row is always
                         * something the user subscribed to, so the picture is
                         * already on this device — no request per row, in a
                         * list that redraws every second.
                         */}
                        <ItemThumb
                            image={
                                library.rows.find(
                                    (r) => r.id === download.meta.item
                                )?.image
                            }
                            kind={download.meta.kind}
                            size="sm"
                        />
                        {done && (
                            <FiCheck
                                aria-hidden
                                className="size-3.5 shrink-0 text-success"
                            />
                        )}
                        {failed && (
                            <FiAlertCircle
                                aria-hidden
                                className="size-3.5 shrink-0 text-danger"
                            />
                        )}
                        <p className="truncate text-sm font-medium">
                            {download.label}
                        </p>
                    </div>

                    <p className="mt-0.5 text-[0.7rem] text-muted">
                        <StatusText download={download} />
                    </p>
                </div>

                <div className="flex shrink-0 items-center gap-1">
                    {(running || download.status === 'queued') && (
                        <IconButton
                            label="Pause"
                            onClick={() => void queue.pause(download.id)}
                        >
                            <FiPause className="size-3.5" />
                        </IconButton>
                    )}

                    {(download.status === 'paused' || failed) && (
                        <IconButton
                            label={failed ? 'Retry' : 'Resume'}
                            onClick={() => void queue.resume(download.id)}
                        >
                            <FiPlay className="size-3.5" />
                        </IconButton>
                    )}

                    {!done && download.status !== 'cancelled' && (
                        <IconButton
                            label="Cancel"
                            onClick={() => void queue.cancel(download.id)}
                        >
                            <FiX className="size-3.5" />
                        </IconButton>
                    )}

                    <IconButton
                        label={open ? 'Hide details' : 'Show details'}
                        onClick={() => setOpen((was) => !was)}
                    >
                        {open ? (
                            <FiChevronUp className="size-3.5" />
                        ) : (
                            <FiChevronDown className="size-3.5" />
                        )}
                    </IconButton>
                </div>
            </div>

            {/*
             * An indeterminate bar when the server sent no length. A bar at 0%
             * for a download that is plainly moving reads as stuck, which is
             * the one thing it must not do.
             */}
            {!done && (
                <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-surface-tertiary">
                    {percent == null ? (
                        <div
                            className={`h-full w-1/3 rounded-full ${
                                running ? 'animate-pulse bg-accent' : 'bg-muted'
                            }`}
                        />
                    ) : (
                        <div
                            className={`h-full rounded-full transition-[width] ${
                                failed ? 'bg-danger' : 'bg-accent'
                            }`}
                            style={{ width: `${percent}%` }}
                        />
                    )}
                </div>
            )}

            {download.error && (
                <p className="selectable mt-2 text-[0.7rem] text-danger">
                    {download.error}
                    {download.attempts > 1 && ` (attempt ${download.attempts})`}
                </p>
            )}

            {open && (
                <div className="mt-3 flex flex-col gap-3 border-t border-border pt-3">
                    {download.samples.length > 0 && (
                        <div className="text-accent">
                            <SpeedGraph
                                samples={download.samples}
                                showLabels
                                height={44}
                            />
                        </div>
                    )}

                    <dl className="grid grid-cols-2 gap-x-4 gap-y-1 text-[0.7rem]">
                        <Detail label="Saving to" value={download.dest} wrap />
                        <Detail
                            label="Size"
                            value={
                                download.total
                                    ? `${formatBytes(download.done)} of ${formatBytes(download.total)}`
                                    : formatBytes(download.done)
                            }
                        />
                        <Detail
                            label="Checksum"
                            value={
                                download.sha256
                                    ? 'Verified on completion'
                                    : 'None published'
                            }
                        />
                        <Detail label="Queued" value={download.queuedAt} />
                    </dl>

                    <div className="flex flex-wrap items-center gap-3">
                        <label className="flex items-center gap-2 text-[0.7rem]">
                            <span className="text-muted">This download</span>
                            <Select
                                label={`Speed limit for ${download.label}`}
                                value={String(download.limitBps ?? 0)}
                                onChange={(next) =>
                                    void queue.setLimit(
                                        download.id,
                                        Number(next) > 0 ? Number(next) : null
                                    )
                                }
                                options={LIMITS}
                            />
                        </label>

                        {download.status === 'queued' && (
                            <button
                                type="button"
                                onClick={() =>
                                    void queue.setPriority(download.id, 50)
                                }
                                className="rounded-lg border border-border px-2 py-1 text-[0.7rem] hover:border-accent"
                            >
                                Move to the front
                            </button>
                        )}
                    </div>
                </div>
            )}
        </article>
    )
}

function StatusText({ download }: { download: DownloadT }) {
    switch (download.status) {
        case 'running':
            return (
                <>
                    {formatBytes(download.done)}
                    {download.total
                        ? ` / ${formatBytes(download.total)}`
                        : ''} · {formatSpeed(download.speedBps)}
                    {download.etaSecs
                        ? ` · ${formatEta(download.etaSecs)} left`
                        : ''}
                </>
            )
        case 'queued':
            return <>Waiting its turn</>
        case 'paused':
            return (
                <>
                    Paused at {formatBytes(download.done)}
                    {download.total ? ` of ${formatBytes(download.total)}` : ''}
                </>
            )
        case 'done':
            return <>Finished · {formatBytes(download.done)}</>
        case 'failed':
            return <>Failed</>
        case 'cancelled':
            return <>Cancelled</>
    }
}

function Detail({
    label,
    value,
    wrap,
}: {
    label: string
    value: string
    wrap?: boolean
}) {
    return (
        <div className="min-w-0">
            <dt className="text-muted">{label}</dt>
            <dd
                className={`selectable ${wrap ? 'break-all' : 'truncate'}`}
                title={value}
            >
                {value}
            </dd>
        </div>
    )
}

function IconButton({
    label,
    onClick,
    children,
}: {
    label: string
    onClick: () => void
    children: React.ReactNode
}) {
    return (
        <button
            type="button"
            aria-label={label}
            title={label}
            onClick={onClick}
            className="rounded-lg border border-border p-1.5 text-muted transition-colors hover:border-accent hover:text-foreground"
        >
            {children}
        </button>
    )
}
