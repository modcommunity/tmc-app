import { useMemo, useState } from 'react'
import { Link } from 'react-router-dom'
import {
    FiAlertTriangle,
    FiArrowDownCircle,
    FiCheckCircle,
    FiExternalLink,
    FiPauseCircle,
    FiRefreshCw,
    FiTrash2,
} from 'react-icons/fi'

import { useLibrary } from '~/lib/library/provider'
import { useAuth } from '~/lib/auth/provider'
import type { LibraryRowT } from '~/lib/ipc/schemas'

/**
 * **Library** — everything this account subscribed to, and what this machine
 * has done about it.
 *
 * The screen answers one question per row and does it in the subtitle rather
 * than behind a hover: *is this installed, is there an update, and if neither,
 * why not?* An item that cannot install here — no rule for its game, no game
 * folder set, the team turned subscriptions off — says so on the row, because
 * the alternative is a library that silently does nothing and a user with no
 * way to find out.
 */

const KIND_LABELS: Record<LibraryRowT['kind'], string> = {
    mod: 'Mod',
    asset: 'Asset',
    collection: 'Collection',
}

type Filter = 'all' | 'installed' | 'updates' | 'problems'

/** The one-line status under a row's name. */
function statusOf(row: LibraryRowT): { text: string; tone: string } {
    if (row.state === 'installing')
        return { text: 'Installing…', tone: 'text-accent' }
    if (row.state === 'removing') return { text: 'Removing…', tone: 'text-accent' }

    if (row.state === 'failed' && row.lastError)
        return { text: row.lastError, tone: 'text-danger' }

    if (!row.installable)
        return {
            text: 'No longer installable — its team turned subscriptions off, or the game lost app support.',
            tone: 'text-warning',
        }

    /*
     * A collection has no files of its own: subscribing to it subscribed you to
     * each of its members, and those are the rows that install. Saying so beats
     * the alternative — "no install rule for this game", which is an error
     * about something that was never going to happen.
     */
    if (row.isContainer)
        return {
            text: 'Its items are subscribed individually and appear below.',
            tone: 'text-muted',
        }

    if (!row.hasRule)
        return {
            text: 'No install rule for this game on this device.',
            tone: 'text-warning',
        }

    if (row.paused)
        return { text: 'Paused — not installed or updated.', tone: 'text-muted' }

    if (row.updateAvailable)
        return {
            text: `Update available: ${row.latestVersion ?? 'new release'} (installed ${row.installedVersion ?? 'unknown'})`,
            tone: 'text-accent',
        }

    if (row.installedReleaseId !== null)
        return {
            text: `Installed${row.installedVersion ? ` · ${row.installedVersion}` : ''}`,
            tone: 'text-success',
        }

    if (row.fileUrl === null)
        return { text: 'No release file yet.', tone: 'text-muted' }

    return { text: 'Not installed yet.', tone: 'text-muted' }
}

function LibraryRow({ row }: { row: LibraryRowT }) {
    const { install, uninstall, busy } = useLibrary()

    const status = statusOf(row)
    const working =
        busy.has(row.id) || row.state === 'installing' || row.state === 'removing'

    const canInstall =
        row.installable &&
        row.hasRule &&
        !row.isContainer &&
        !row.paused &&
        row.fileUrl !== null

    return (
        <div className="flex items-start gap-3 border-b border-border px-3 py-3 last:border-b-0">
            <div className="h-12 w-12 shrink-0 overflow-hidden rounded-lg bg-surface-2">
                {row.image ? (
                    <img
                        src={row.image}
                        alt=""
                        loading="lazy"
                        className="h-full w-full object-cover"
                    />
                ) : null}
            </div>

            <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-2">
                    <Link
                        to={`/view/${row.kind}/${row.itemId}`}
                        className="truncate text-sm font-medium hover:text-accent"
                    >
                        {row.name}
                    </Link>

                    <span className="rounded border border-border px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-muted">
                        {KIND_LABELS[row.kind]}
                    </span>

                    {row.appName ? (
                        <span className="text-[11px] text-muted">
                            {row.appName}
                        </span>
                    ) : null}

                    {row.viaCollectionId !== null ? (
                        <span
                            title="Added by a collection you subscribed to"
                            className="text-[11px] text-muted"
                        >
                            via collection
                        </span>
                    ) : null}
                </div>

                <p className={`mt-0.5 text-xs ${status.tone}`}>{status.text}</p>
            </div>

            <div className="flex shrink-0 items-center gap-1">
                {canInstall ? (
                    <button
                        type="button"
                        disabled={working}
                        onClick={() => void install(row.id)}
                        aria-label={
                            row.updateAvailable ? 'Update now' : 'Install now'
                        }
                        title={row.updateAvailable ? 'Update now' : 'Install now'}
                        className="rounded-lg border border-border p-2 text-muted hover:text-foreground disabled:opacity-50"
                    >
                        {row.updateAvailable ? (
                            <FiArrowDownCircle className="h-4 w-4" />
                        ) : (
                            <FiCheckCircle className="h-4 w-4" />
                        )}
                    </button>
                ) : null}

                {row.installedReleaseId !== null ? (
                    <button
                        type="button"
                        disabled={working}
                        onClick={() => void uninstall(row.id)}
                        aria-label="Remove from this device"
                        title="Remove from this device"
                        className="rounded-lg border border-border p-2 text-muted hover:text-danger disabled:opacity-50"
                    >
                        <FiTrash2 className="h-4 w-4" />
                    </button>
                ) : null}

                <a
                    href={row.webUrl}
                    target="_blank"
                    rel="noreferrer"
                    aria-label="Open on the website"
                    title="Open on the website"
                    className="rounded-lg border border-border p-2 text-muted hover:text-foreground"
                >
                    <FiExternalLink className="h-4 w-4" />
                </a>
            </div>
        </div>
    )
}

export default function LibraryRoute() {
    const { status } = useAuth()
    const { rows, loading, lastSync, error, sync } = useLibrary()
    const [filter, setFilter] = useState<Filter>('all')

    const counts = useMemo(() => {
        const installed = rows.filter((r) => r.installedReleaseId !== null).length
        const updates = rows.filter((r) => r.updateAvailable).length
        // A container with no rule is not a problem — it was never going to
        // have one. Counting it here would put a permanent badge on the filter.
        const problems = rows.filter(
            (r) =>
                r.state === 'failed' ||
                !r.installable ||
                (!r.hasRule && !r.isContainer)
        ).length

        return { all: rows.length, installed, updates, problems }
    }, [rows])

    const shown = useMemo(() => {
        switch (filter) {
            case 'installed':
                return rows.filter((r) => r.installedReleaseId !== null)
            case 'updates':
                return rows.filter((r) => r.updateAvailable)
            case 'problems':
                return rows.filter(
                    (r) =>
                        r.state === 'failed' ||
                        !r.installable ||
                        (!r.hasRule && !r.isContainer)
                )
            default:
                return rows
        }
    }, [rows, filter])

    if (status !== 'signedIn') {
        return (
            <div className="p-6 text-center">
                <p className="text-sm text-muted">
                    Sign in to see the mods and assets you subscribed to.
                </p>
                <Link
                    to="/account"
                    className="mt-3 inline-block rounded-lg bg-accent px-4 py-2 text-sm text-accent-foreground"
                >
                    Sign in
                </Link>
            </div>
        )
    }

    const FILTERS: { key: Filter; label: string; count: number }[] = [
        { key: 'all', label: 'All', count: counts.all },
        { key: 'installed', label: 'Installed', count: counts.installed },
        { key: 'updates', label: 'Updates', count: counts.updates },
        { key: 'problems', label: 'Needs attention', count: counts.problems },
    ]

    return (
        <div className="flex flex-col gap-3 p-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
                <div>
                    <h1 className="text-base font-semibold">Library</h1>
                    <p className="text-xs text-muted">
                        {lastSync
                            ? `Last synced ${new Date(lastSync.at).toLocaleTimeString()}`
                            : 'Not synced yet'}
                    </p>
                </div>

                <button
                    type="button"
                    disabled={loading}
                    onClick={() => void sync(true)}
                    className="flex items-center gap-2 rounded-lg border border-border px-3 py-1.5 text-xs disabled:opacity-50"
                >
                    <FiRefreshCw
                        className={
                            loading ? 'h-3.5 w-3.5 animate-spin' : 'h-3.5 w-3.5'
                        }
                    />
                    Sync now
                </button>
            </div>

            {error ? (
                <p className="flex items-start gap-2 rounded-lg border border-danger/40 bg-danger/10 p-2 text-xs text-danger">
                    <FiAlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                    {error}
                </p>
            ) : null}

            <div className="flex flex-wrap gap-1.5">
                {FILTERS.map((f) => (
                    <button
                        key={f.key}
                        type="button"
                        onClick={() => setFilter(f.key)}
                        aria-pressed={filter === f.key}
                        className={`rounded-lg border px-2.5 py-1 text-xs ${
                            filter === f.key
                                ? 'border-accent bg-accent/10 text-accent'
                                : 'border-border text-muted'
                        }`}
                    >
                        {f.label}
                        <span className="ml-1.5 opacity-70">{f.count}</span>
                    </button>
                ))}
            </div>

            <div className="rounded-xl border border-border">
                {shown.length === 0 ? (
                    <div className="flex flex-col items-center gap-2 p-8 text-center">
                        <FiPauseCircle className="h-6 w-6 text-muted" />
                        <p className="text-sm">
                            {rows.length === 0
                                ? 'Nothing subscribed yet.'
                                : 'Nothing matches this filter.'}
                        </p>
                        {rows.length === 0 ? (
                            <p className="max-w-sm text-xs text-muted">
                                Subscribe to a mod, asset or collection — here or on
                                the website — and it will install itself on every
                                device you are signed in on.
                            </p>
                        ) : null}
                    </div>
                ) : (
                    shown.map((row) => <LibraryRow key={row.id} row={row} />)
                )}
            </div>
        </div>
    )
}
