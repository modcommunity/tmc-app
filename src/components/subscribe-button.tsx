import { useCallback, useMemo, useState } from 'react'
import { Link } from 'react-router-dom'
import {
    FiCheckCircle,
    FiCloudOff,
    FiDownloadCloud,
    FiLoader,
} from 'react-icons/fi'
import { Button } from '@modcommunity/shared'

import { api } from '~/lib/api/client'
import { useAuth } from '~/lib/auth/provider'
import { useLibrary } from '~/lib/library/provider'
import type { ContentSummaryT, SubKindT } from '~/lib/api/contract'

/**
 * **Subscribe** — the app's primary action on a mod, asset or collection.
 *
 * The counterpart to the website's split button, and deliberately simpler: a
 * user reading this screen has the app, so "install it and keep it updated" is
 * the obvious thing to want. The manual download stays beside it (the view
 * route renders both), for the same reason it does on the website — a real
 * fraction of people want the file and nothing else, and hiding it is what
 * makes a mod site feel like it is holding files hostage.
 *
 * WHY THE STATE COMES FROM THE LIBRARY AND NOT A QUERY
 * ---------------------------------------------------
 * The library is already loaded and already kept fresh by the sync loop, so
 * "am I subscribed?" is a lookup rather than a request. It also means the
 * button is correct offline, and that subscribing on the website updates it
 * within one poll without this component knowing anything about polling.
 */
export default function SubscribeButton({ summary }: { summary: ContentSummaryT }) {
    const { status } = useAuth()
    const { rows, sync, install, busy } = useLibrary()

    const [working, setWorking] = useState(false)
    const [error, setError] = useState<string | null>(null)

    const kind = summary.kind as SubKindT
    const itemId = Number(summary.id)

    const row = useMemo(
        () => rows.find((r) => r.kind === kind && r.itemId === itemId),
        [rows, kind, itemId]
    )

    const subscribed = row !== undefined
    const pending = working || (row ? busy.has(row.id) : false)

    const toggle = useCallback(async () => {
        setWorking(true)
        setError(null)

        try {
            await api.subscribe(kind, itemId, !subscribed)

            /*
             * A FULL sync rather than a delta. An unsubscribe has to be
             * reconciled — the item leaves the list rather than changing in it —
             * and a delta cannot express that. It is also the pass that queues
             * the install, so the button's click is what makes the files
             * appear.
             */
            await sync(true)
        } catch (err) {
            setError(err instanceof Error ? err.message : 'Could not do that.')
        } finally {
            setWorking(false)
        }
    }, [kind, itemId, subscribed, sync])

    // Only three kinds can be subscribed to.
    if (kind !== 'mod' && kind !== 'asset' && kind !== 'collection') return null

    if (status !== 'signedIn')
        return (
            <Link
                to="/account"
                className="flex items-center gap-2 rounded-lg border border-border px-3 py-2 text-xs text-muted"
            >
                <FiDownloadCloud className="size-3.5" />
                Sign in to subscribe
            </Link>
        )

    return (
        <div className="flex flex-col gap-1">
            <div className="flex flex-wrap items-center gap-2">
                <Button
                    btnType={subscribed ? 'secondary' : 'primary'}
                    onClick={() => void toggle()}
                    disabled={pending}
                >
                    <span className="flex items-center gap-2">
                        {pending ? (
                            <FiLoader className="size-4 animate-spin" />
                        ) : subscribed ? (
                            <FiCheckCircle className="size-4" />
                        ) : (
                            <FiDownloadCloud className="size-4" />
                        )}
                        {subscribed ? 'Subscribed' : 'Subscribe'}
                    </span>
                </Button>

                {/*
                    An update the user's own `autoUpdate` setting declined to
                    apply. The sync loop reports it and deliberately does not
                    act, so this is where it becomes actionable.
                */}
                {row?.updateAvailable ? (
                    <Button
                        btnType="secondary"
                        onClick={() => void install(row.id)}
                        disabled={pending}
                    >
                        <span className="flex items-center gap-2">
                            <FiDownloadCloud className="size-4" />
                            Update to {row.latestVersion ?? 'the new release'}
                        </span>
                    </Button>
                ) : null}
            </div>

            {row && !row.hasRule && !row.isContainer ? (
                <p className="flex items-center gap-1.5 text-[11px] text-warning">
                    <FiCloudOff className="size-3" />
                    No install rule for {summary.app?.name ?? 'this game'} on this
                    device — subscribed, but nothing will be written to disk.
                </p>
            ) : null}

            {row?.lastError ? (
                <p className="selectable text-[11px] text-danger">
                    {row.lastError}
                </p>
            ) : null}

            {error ? (
                <p className="selectable text-[11px] text-danger">{error}</p>
            ) : null}
        </div>
    )
}
