import { useCallback, useEffect, useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { FiTerminal } from 'react-icons/fi'

import type { ContentSummaryT } from '~/lib/api/contract'
import { ipc } from '~/lib/ipc/commands'
import type { RconServerT } from '~/lib/ipc/schemas'
import { ServerForm } from '~/routes/rcon'

/**
 * The server page's tie-in to the local RCON console.
 *
 * A saved RCON server is local and encrypted; this only READS the list on this
 * device. If one is linked to this TMC server (or matches its address and game)
 * it offers the console; otherwise it offers to save one, prefilled from the
 * page. Nothing here reaches the website, and the password is only ever typed
 * into the same form the RCON screen uses.
 */
export default function ServerRcon({ item }: { item: ContentSummaryT }) {
    const navigate = useNavigate()
    const [saved, setSaved] = useState<RconServerT[] | null>(null)
    const [adding, setAdding] = useState(false)

    const load = useCallback(async () => {
        try {
            setSaved(await ipc.rconList())
        } catch {
            // Not the desktop shell, or the library is unavailable: no panel.
            setSaved(null)
        }
    }, [])

    useEffect(() => {
        queueMicrotask(() => void load())
    }, [load])

    if (saved === null) return null

    const id = Number(item.id)
    const host = item.server?.host ?? null
    const appId = item.app?.id ?? null

    const match =
        saved.find((s) => s.serverId === id) ??
        saved.find(
            (s) =>
                host !== null &&
                s.host.toLowerCase() === host.toLowerCase() &&
                (appId === null || s.appId === null || s.appId === appId)
        )

    if (match)
        return (
            <section className="flex flex-wrap items-center gap-3 rounded-xl border border-border bg-surface p-3 text-xs">
                <FiTerminal className="size-4 text-accent" />
                <div className="min-w-0 flex-1">
                    <p className="text-sm font-medium">RCON console</p>
                    <p className="text-muted">
                        Saved on this device as “{match.name}”.
                    </p>
                </div>
                <Link
                    to={`/rcon/${match.id}`}
                    className="rounded-lg bg-accent px-3 py-1.5 text-accent-foreground"
                >
                    Open console
                </Link>
            </section>
        )

    if (adding)
        return (
            <ServerForm
                preset={{
                    name: item.name,
                    host: host ?? '',
                    port: 27015,
                    appId,
                    serverId: Number.isFinite(id) ? id : null,
                }}
                onCancel={() => setAdding(false)}
                onSaved={(newId) => {
                    setAdding(false)
                    void navigate(`/rcon/${newId}`)
                }}
            />
        )

    return (
        <section className="flex flex-wrap items-center gap-3 rounded-xl border border-dashed border-border p-3 text-xs">
            <FiTerminal className="size-4 text-muted" />
            <p className="min-w-0 flex-1 text-muted">
                Administer this server? Save its RCON login to run commands from
                here. It stays encrypted on this device.
            </p>
            <button
                type="button"
                onClick={() => setAdding(true)}
                className="rounded-lg border border-border px-3 py-1.5 hover:border-accent"
            >
                Set up RCON
            </button>
        </section>
    )
}
