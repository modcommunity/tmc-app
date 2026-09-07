import { useCallback, useEffect, useMemo, useState } from 'react'
import {
    FiAlertTriangle,
    FiBox,
    FiChevronDown,
    FiChevronRight,
    FiEdit2,
    FiExternalLink,
    FiPackage,
    FiPlus,
    FiRefreshCw,
    FiSearch,
    FiTrash2,
    FiX,
} from 'react-icons/fi'

import { ipc } from '~/lib/ipc/commands'
import { messageOf } from '~/lib/ipc'
import ImportDialog, { type ImportTarget } from '~/components/import-dialog'
import {
    type LocalModT,
    type ManagerInfoT,
    type SandboxRowT,
} from '~/lib/ipc/schemas'

/**
 * **Mods on this device that no account put here.**
 *
 * Three ways in, and they are three different situations rather than three
 * buttons for one:
 *
 *   * **Dropped** — a file or folder dragged onto the window. Handled by
 *     `components/drop-import`; the results land in this list.
 *   * **Already in the game folder** — somebody who has been modding for years
 *     has thirty DLLs in `BepInEx/plugins` that this app knows nothing about.
 *     The scan lists what the deployment ledger, the installed subscriptions
 *     and previous adoptions cannot account for, and adopting one COPIES it
 *     into the app's store while leaving the original alone.
 *   * **Another mod manager** — Vortex, r2modman, CurseForge, Prism. A
 *     descriptor per manager says where it keeps things; the app reads it and
 *     copies. That manager is not touched, then or ever.
 *
 * WHY ADOPTING COPIES RATHER THAN CLAIMING IN PLACE
 * -------------------------------------------------
 * Claiming a file where it sits would mean the ledger asserting ownership of
 * something the app did not put there — and the ledger is the only record that
 * makes an undeploy safe. A copy keeps that record honest: the original stays
 * exactly where the user left it, so backing out is deleting a row, and the
 * game keeps working either way.
 */

function bytes(n: number): string {
    if (n < 1024) return `${n} B`
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`
    if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`

    return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`
}

const ORIGIN_LABEL: Record<LocalModT['origin'], string> = {
    dropped: 'Dropped in',
    folder: 'Folder',
    adopted: 'Found in the game folder',
    manager: 'From another manager',
}

export default function LocalMods({
    sandbox,
    appId,
    onChanged,
}: {
    /** When present, imports go straight into it and it can be scanned. */
    sandbox?: SandboxRowT | null
    appId?: number
    onChanged?: () => void
}) {
    const game = sandbox?.appId ?? appId

    const [rows, setRows] = useState<LocalModT[] | null>(null)
    const [error, setError] = useState<string | null>(null)
    const [busy, setBusy] = useState<string | null>(null)
    const [editing, setEditing] = useState<number | null>(null)
    const [target, setTarget] = useState<ImportTarget | null>(null)
    const [managers, setManagers] = useState<ManagerInfoT[] | null>(null)
    const [showManagers, setShowManagers] = useState(false)

    const load = useCallback(() => {
        void ipc
            .localList(game)
            .then(setRows)
            .catch((err: unknown) => setError(messageOf(err)))
    }, [game])

    useEffect(load, [load])

    // ------------------------------------------------------ Finding things

    const scanGameFolder = useCallback(async () => {
        if (!sandbox) return

        setBusy('unmanaged')
        setError(null)

        try {
            const found = await ipc.importUnmanaged(sandbox.id)

            if (found.length < 1) {
                setError(
                    'Nothing unaccounted for — everything in this game’s mod folders is already known to the app.'
                )

                return
            }

            const labels: Record<string, string> = {}
            const relPaths: Record<string, string> = {}

            for (const item of found) {
                labels[item.token] = item.name
                /*
                 * The folder it was FOUND in is where it goes back to. Anything
                 * else would move somebody's working mod to a different place
                 * the first time they deploy, which is the one thing an adopt
                 * must never do.
                 */
                relPaths[item.token] = item.relPath
                    .split('/')
                    .slice(0, -1)
                    .join('/')
            }

            setTarget({
                tokens: found.map((f) => f.token),
                labels,
                relPaths,
                sandbox,
                origin: 'adopted',
                from: `Found in ${sandbox.name}’s game folder`,
            })
        } catch (err) {
            setError(messageOf(err))
        } finally {
            setBusy(null)
        }
    }, [sandbox])

    const openManagers = useCallback(async () => {
        setShowManagers((prev) => !prev)

        if (managers) return

        try {
            setManagers(await ipc.importManagers())
        } catch (err) {
            setError(messageOf(err))
        }
    }, [managers])

    const scanManager = useCallback(
        async (manager: ManagerInfoT) => {
            setBusy(manager.id)
            setError(null)

            try {
                const found = await ipc.importManagerScan(manager.id)

                const usable = sandbox
                    ? found.filter((f) => f.slug === sandbox.appSlug)
                    : found

                if (usable.length < 1) {
                    setError(
                        `Nothing found in ${manager.label}. If it is installed somewhere other than its default folder, a corrected descriptor in the app’s plugins/manager folder is the whole fix.`
                    )

                    return
                }

                const labels: Record<string, string> = {}
                const relPaths: Record<string, string> = {}

                for (const item of usable) {
                    labels[item.token] = item.group
                        ? `${item.name} (${item.group})`
                        : item.name

                    if (item.relPath) relPaths[item.token] = item.relPath
                }

                setTarget({
                    tokens: usable.map((f) => f.token),
                    labels,
                    relPaths,
                    sandbox,
                    appId: game,
                    origin: 'manager',
                    from: `Read from ${manager.label}`,
                })
            } catch (err) {
                setError(messageOf(err))
            } finally {
                setBusy(null)
            }
        },
        [sandbox, game]
    )

    // ------------------------------------------------------------- Actions

    const addToSandbox = useCallback(
        async (mod: LocalModT) => {
            if (!sandbox) return

            setBusy(`add-${mod.id}`)

            try {
                await ipc.localAddToSandbox(sandbox.id, mod.id)
                onChanged?.()
            } catch (err) {
                setError(messageOf(err))
            } finally {
                setBusy(null)
            }
        },
        [sandbox, onChanged]
    )

    const remove = useCallback(
        async (mod: LocalModT) => {
            setBusy(`del-${mod.id}`)

            try {
                const affected = await ipc.localDelete(mod.id)

                load()
                onChanged?.()

                if (affected.length > 0) {
                    setError(
                        `${mod.name} was in ${affected.length} sandbox${affected.length === 1 ? '' : 'es'}. Its files are gone from the app, but a deployed copy stays in the game folder until you deploy again.`
                    )
                }
            } catch (err) {
                setError(messageOf(err))
            } finally {
                setBusy(null)
            }
        },
        [load, onChanged]
    )

    const inSandbox = useMemo(
        () => new Set((sandbox?.mods ?? []).map((m) => m.modKey)),
        [sandbox]
    )

    return (
        <section className="rounded-xl border border-border bg-surface">
            <header className="flex flex-wrap items-center justify-between gap-2 border-b border-border p-3">
                <div>
                    <h3 className="flex items-center gap-1.5 text-xs font-semibold">
                        <FiBox className="size-3.5" />
                        Imported mods
                    </h3>
                    <p className="text-[11px] text-muted">
                        Anything on this device that no subscription put here. Drag
                        a file onto the window to add one.
                    </p>
                </div>

                <div className="flex flex-wrap gap-2">
                    {sandbox && (
                        <button
                            type="button"
                            onClick={() => void scanGameFolder()}
                            disabled={busy === 'unmanaged'}
                            className="flex items-center gap-1.5 rounded-lg border border-border px-2.5 py-1.5 text-[11px] disabled:opacity-50"
                        >
                            <FiSearch className="size-3" />
                            {busy === 'unmanaged'
                                ? 'Looking…'
                                : 'Find mods already in the game folder'}
                        </button>
                    )}

                    <button
                        type="button"
                        onClick={() => void openManagers()}
                        className="flex items-center gap-1.5 rounded-lg border border-border px-2.5 py-1.5 text-[11px]"
                    >
                        <FiPackage className="size-3" />
                        Import from another manager
                        {showManagers ? (
                            <FiChevronDown className="size-3" />
                        ) : (
                            <FiChevronRight className="size-3" />
                        )}
                    </button>
                </div>
            </header>

            {showManagers && (
                <div className="border-b border-border p-3">
                    {managers === null ? (
                        <p className="text-[11px] text-muted">Loading…</p>
                    ) : (
                        <div className="flex flex-col gap-1.5">
                            {managers.map((manager) => (
                                <ManagerRow
                                    key={manager.id}
                                    manager={manager}
                                    busy={busy === manager.id}
                                    onScan={() => void scanManager(manager)}
                                />
                            ))}

                            <p className="mt-1 text-[11px] text-muted">
                                Each of these reads what the manager already keeps
                                on this machine. Nothing there is moved or changed —
                                the mods are copied, so going back to that manager
                                tomorrow finds everything as it was.
                            </p>
                        </div>
                    )}
                </div>
            )}

            {error && (
                <div className="flex items-start gap-2 border-b border-border bg-warning/10 p-3 text-[11px] text-warning">
                    <FiAlertTriangle className="mt-0.5 size-3.5 shrink-0" />
                    <span className="flex-1">{error}</span>
                    <button
                        type="button"
                        onClick={() => setError(null)}
                        aria-label="Dismiss"
                        className="shrink-0"
                    >
                        <FiX className="size-3.5" />
                    </button>
                </div>
            )}

            <div className="p-3">
                {rows === null ? (
                    <p className="text-[11px] text-muted">Loading…</p>
                ) : rows.length < 1 ? (
                    <p className="text-[11px] text-muted">
                        Nothing imported yet. Drag a mod file, an archive or a
                        folder onto the window
                        {sandbox
                            ? ', or look for what is already in the game folder.'
                            : '.'}
                    </p>
                ) : (
                    <div className="flex flex-col gap-1.5">
                        {rows.map((mod) => (
                            <div
                                key={mod.id}
                                className="rounded-lg border border-border bg-surface-2 p-2.5"
                            >
                                <div className="flex items-start gap-2">
                                    <div className="min-w-0 flex-1">
                                        <p className="truncate text-xs font-medium">
                                            {mod.name}
                                            {mod.version && (
                                                <span className="ml-1.5 font-normal text-muted">
                                                    v{mod.version}
                                                </span>
                                            )}
                                        </p>

                                        <p className="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px] text-muted">
                                            <span>
                                                {ORIGIN_LABEL[mod.origin]}
                                                {mod.originLabel
                                                    ? ` · ${mod.originLabel}`
                                                    : ''}
                                            </span>
                                            <span className="font-mono">
                                                {mod.relPath || 'game folder'}
                                            </span>
                                            <span>
                                                {mod.files} file
                                                {mod.files === 1 ? '' : 's'} ·{' '}
                                                {bytes(mod.bytes)}
                                            </span>
                                        </p>

                                        {mod.source && (
                                            <p className="mt-1 flex items-center gap-1 text-[11px] text-accent">
                                                <FiExternalLink className="size-3" />
                                                This is a release of an item on the
                                                site — subscribing instead keeps it
                                                updated.
                                            </p>
                                        )}
                                    </div>

                                    <div className="flex shrink-0 gap-1">
                                        {sandbox &&
                                            !inSandbox.has(`local:${mod.id}`) && (
                                                <button
                                                    type="button"
                                                    onClick={() =>
                                                        void addToSandbox(mod)
                                                    }
                                                    disabled={
                                                        busy === `add-${mod.id}`
                                                    }
                                                    title="Add to this sandbox"
                                                    className="rounded-md border border-border p-1.5 disabled:opacity-50"
                                                >
                                                    <FiPlus className="size-3" />
                                                </button>
                                            )}

                                        <button
                                            type="button"
                                            onClick={() =>
                                                setEditing(
                                                    editing === mod.id
                                                        ? null
                                                        : mod.id
                                                )
                                            }
                                            title="Edit"
                                            className="rounded-md border border-border p-1.5"
                                        >
                                            <FiEdit2 className="size-3" />
                                        </button>

                                        <button
                                            type="button"
                                            onClick={() => void remove(mod)}
                                            disabled={busy === `del-${mod.id}`}
                                            title="Delete from this device"
                                            className="rounded-md border border-border p-1.5 text-danger disabled:opacity-50"
                                        >
                                            <FiTrash2 className="size-3" />
                                        </button>
                                    </div>
                                </div>

                                {editing === mod.id && (
                                    <Editor
                                        mod={mod}
                                        onSaved={(next) => {
                                            setRows(
                                                (prev) =>
                                                    prev?.map((r) =>
                                                        r.id === next.id ? next : r
                                                    ) ?? null
                                            )
                                            setEditing(null)
                                            onChanged?.()
                                        }}
                                        onError={setError}
                                    />
                                )}
                            </div>
                        ))}
                    </div>
                )}
            </div>

            {target && (
                <ImportDialog
                    target={target}
                    onClose={() => setTarget(null)}
                    onImported={() => {
                        load()
                        onChanged?.()
                    }}
                />
            )}
        </section>
    )
}

function ManagerRow({
    manager,
    busy,
    onScan,
}: {
    manager: ManagerInfoT
    busy: boolean
    onScan: () => void
}) {
    return (
        <div className="rounded-lg border border-border bg-surface-2 p-2.5">
            <div className="flex items-start gap-2">
                <div className="min-w-0 flex-1">
                    <p className="text-xs font-medium">{manager.label}</p>

                    {manager.notes.map((note, i) => (
                        <p key={i} className="mt-0.5 text-[11px] text-muted">
                            {note}
                        </p>
                    ))}
                </div>

                <button
                    type="button"
                    onClick={onScan}
                    disabled={busy}
                    className="flex shrink-0 items-center gap-1.5 rounded-md border border-border px-2 py-1 text-[11px] disabled:opacity-50"
                >
                    <FiRefreshCw
                        className={busy ? 'size-3 animate-spin' : 'size-3'}
                    />
                    {busy ? 'Reading…' : 'Look'}
                </button>
            </div>
        </div>
    )
}

/**
 * The editable half of an imported mod.
 *
 * `Installs to` is the one field with teeth: saving it MOVES the files inside
 * the app's store, so a row that says `BepInEx/plugins` and files that sit
 * under `mods` can never drift apart. Every sandbox holding this mod needs a
 * redeploy afterwards, which is why the hint says so rather than the app
 * silently redeploying somebody's game folder from a text box.
 */
function Editor({
    mod,
    onSaved,
    onError,
}: {
    mod: LocalModT
    onSaved: (next: LocalModT) => void
    onError: (message: string) => void
}) {
    const [name, setName] = useState(mod.name)
    const [version, setVersion] = useState(mod.version ?? '')
    const [author, setAuthor] = useState(mod.author ?? '')
    const [relPath, setRelPath] = useState(mod.relPath)
    const [notes, setNotes] = useState(mod.notes ?? '')
    const [busy, setBusy] = useState(false)

    const save = async () => {
        setBusy(true)

        try {
            onSaved(
                await ipc.localPatch(mod.id, {
                    name,
                    version,
                    author,
                    relPath,
                    notes,
                })
            )
        } catch (err) {
            onError(messageOf(err))
        } finally {
            setBusy(false)
        }
    }

    return (
        <div className="mt-2.5 border-t border-border pt-2.5">
            <div className="grid gap-2 sm:grid-cols-2">
                <Field label="Name" value={name} onChange={setName} />
                <Field label="Version" value={version} onChange={setVersion} />
                <Field label="Author" value={author} onChange={setAuthor} />
                <Field
                    label="Installs to"
                    value={relPath}
                    onChange={setRelPath}
                    placeholder="the game's folder"
                    mono
                />
            </div>

            <label className="mt-2 flex flex-col gap-1">
                <span className="text-[10px] uppercase tracking-wide text-muted">
                    Note
                </span>
                <textarea
                    value={notes}
                    onChange={(e) => setNotes(e.target.value)}
                    rows={2}
                    className="rounded-md border border-border bg-surface px-2 py-1 text-[11px]"
                />
            </label>

            <div className="mt-2 flex items-center justify-between gap-2">
                <p className="text-[11px] text-muted">
                    {relPath === mod.relPath
                        ? 'Only this app’s own record changes.'
                        : 'Changing where it installs moves its files — deploy any sandbox holding it afterwards.'}
                </p>

                <button
                    type="button"
                    onClick={() => void save()}
                    disabled={busy}
                    className="shrink-0 rounded-lg bg-accent px-3 py-1.5 text-[11px] text-accent-foreground disabled:opacity-50"
                >
                    {busy ? 'Saving…' : 'Save'}
                </button>
            </div>
        </div>
    )
}

function Field({
    label,
    value,
    onChange,
    placeholder,
    mono,
}: {
    label: string
    value: string
    onChange: (next: string) => void
    placeholder?: string
    mono?: boolean
}) {
    return (
        <label className="flex flex-col gap-1">
            <span className="text-[10px] uppercase tracking-wide text-muted">
                {label}
            </span>
            <input
                value={value}
                placeholder={placeholder}
                onChange={(e) => onChange(e.target.value)}
                className={`rounded-md border border-border bg-surface px-2 py-1 text-[11px] ${mono ? 'font-mono' : ''}`}
            />
        </label>
    )
}
