import { useCallback, useEffect, useMemo, useState } from 'react'
import {
    FiAlertTriangle,
    FiArchive,
    FiCheck,
    FiFile,
    FiFolder,
    FiLink,
    FiX,
} from 'react-icons/fi'

import { ipc } from '~/lib/ipc/commands'
import { messageOf } from '~/lib/ipc'
import Select from '~/components/select'
import {
    type ImportOutcomeT,
    type ImportPayloadT,
    type ImportPreviewT,
    type SandboxRowT,
} from '~/lib/ipc/schemas'

/**
 * **What is about to be imported, and where it will land.**
 *
 * The two decisions an import makes are guesses about somebody else's archive:
 * whether to unpack it, and where its contents belong under the game folder.
 * Rust suggests both from the game's own install rules — a Minecraft `.jar` is
 * a file, a `.zip` of loose DLLs is unpacked, `mods` is where mods go — and it
 * is right most of the time, which is exactly why the wrong times need to be
 * visible before anything is written rather than diagnosed afterwards from a
 * game that will not start.
 *
 * So this dialog exists to be *boring*: it shows what would happen, lets the
 * three fields that matter be edited, and imports. Nothing is deployed here.
 * The files go into the app's own store, and putting them in front of a game is
 * the same separate, explicit deploy every subscribed mod goes through.
 *
 * WHY THE NAME IS EDITABLE AND PRE-FILLED
 * ---------------------------------------
 * `handed-to-me.zip` is a filename, not a mod name, and it is what the user
 * will see in their load order for the next year. Pre-filling from the archive
 * (its `tmc.json`, or failing that its file name) and letting them fix it is
 * the whole feature — the alternative is a list of filenames nobody can read.
 */

/** Everything this dialog can be opened with, reduced to one shape. */
export type ImportTarget = {
    /** Tokens Rust minted for the paths it found. Never paths. */
    tokens: string[]
    /** Fallback labels, for the moment before the preview resolves. */
    labels?: Record<string, string>
    /** Pre-set install paths, from a manager descriptor or an adopt scan. */
    relPaths?: Record<string, string>
    appId?: number
    sandbox?: SandboxRowT | null
    origin?: 'dropped' | 'adopted' | 'manager'
    /** What to call the source, for the heading. */
    from?: string
}

type Row = {
    token: string
    name: string
    relPath: string
    payload: ImportPayloadT
    /** From Rust; not editable, only shown. */
    preview: ImportPreviewT | null
    include: boolean
}

const PAYLOADS: { value: ImportPayloadT; label: string; hint: string }[] = [
    {
        value: 'file',
        label: 'Keep the file as it is',
        hint: 'For a .jar, a .dll, or a pack the game reads as an archive.',
    },
    {
        value: 'unpack',
        label: 'Unpack it',
        hint: 'For an archive holding the loose files a mod is made of.',
    },
    {
        value: 'folder',
        label: 'Copy the folder',
        hint: 'Everything inside it, as it is.',
    },
]

function bytes(n: number): string {
    if (n < 1024) return `${n} B`
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`
    if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`

    return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`
}

export default function ImportDialog({
    target,
    onClose,
    onImported,
}: {
    target: ImportTarget
    onClose: () => void
    onImported?: (report: ImportOutcomeT) => void
}) {
    const [rows, setRows] = useState<Row[]>(() =>
        target.tokens.map((token) => ({
            token,
            name: target.labels?.[token] ?? '',
            relPath: target.relPaths?.[token] ?? '',
            // Replaced by whatever Rust suggests the moment the preview lands.
            payload: 'file',
            preview: null,
            include: true,
        }))
    )

    const [loading, setLoading] = useState(true)
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const [report, setReport] = useState<ImportOutcomeT | null>(null)

    const appId = target.sandbox?.appId ?? target.appId

    useEffect(() => {
        let live = true

        void ipc
            .importPreview(target.tokens, appId)
            .then((previews) => {
                if (!live) return

                setRows((prev) =>
                    prev.map((row) => {
                        const found = previews.find((p) => p.token === row.token)

                        if (!found) return row

                        return {
                            ...row,
                            preview: found,
                            name: row.name || found.name,
                            /*
                             * A pre-set path wins over the suggestion. It came
                             * from a manager descriptor that knows where that
                             * manager's mods belong, or from the folder the
                             * adopt scan literally found it in — both of which
                             * are better answers than "the game's first mod
                             * target".
                             */
                            relPath: row.relPath || found.relPath,
                            payload: found.payload,
                        }
                    })
                )
            })
            .catch((err: unknown) => {
                if (live) setError(messageOf(err))
            })
            .finally(() => {
                if (live) setLoading(false)
            })

        return () => {
            live = false
        }
        // The token list is fixed for the life of this dialog.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [])

    const patch = useCallback((token: string, next: Partial<Row>) => {
        setRows((prev) =>
            prev.map((row) => (row.token === token ? { ...row, ...next } : row))
        )
    }, [])

    const chosen = useMemo(() => rows.filter((r) => r.include), [rows])

    const run = useCallback(async () => {
        if (chosen.length < 1) return

        setBusy(true)
        setError(null)

        try {
            const result = await ipc.importPaths(
                chosen.map((r) => r.token),
                {
                    appId,
                    sandboxId: target.sandbox?.id,
                    origin: target.origin,
                    overrides: chosen.map((r) => ({
                        token: r.token,
                        name: r.name,
                        relPath: r.relPath,
                        payload: r.payload,
                    })),
                }
            )

            setReport(result)
            onImported?.(result)
        } catch (err) {
            setError(messageOf(err))
        } finally {
            setBusy(false)
        }
    }, [chosen, appId, target.sandbox?.id, target.origin, onImported])

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
            <div className="flex max-h-[90vh] w-full max-w-2xl flex-col overflow-hidden rounded-xl border border-border bg-surface">
                <header className="flex items-center justify-between border-b border-border p-4">
                    <div>
                        <h2 className="text-sm font-semibold">
                            Import{' '}
                            {rows.length === 1 ? 'a mod' : `${rows.length} mods`}
                        </h2>
                        <p className="text-[11px] text-muted">
                            {target.from ?? 'Dropped on the window'}
                            {target.sandbox
                                ? ` · into ${target.sandbox.name}`
                                : ' · into this device'}
                        </p>
                    </div>

                    <button
                        type="button"
                        onClick={onClose}
                        aria-label="Close"
                        className="rounded-lg p-1.5 text-muted hover:bg-surface-2"
                    >
                        <FiX className="size-4" />
                    </button>
                </header>

                <div className="flex-1 overflow-y-auto p-4">
                    {report ? (
                        <Outcome report={report} />
                    ) : (
                        <>
                            {!appId && (
                                <p className="mb-3 flex items-start gap-2 rounded-lg border border-warning/40 bg-warning/10 p-2.5 text-[11px] text-warning">
                                    <FiAlertTriangle className="mt-0.5 size-3.5 shrink-0" />
                                    <span>
                                        No game is selected, so the app cannot guess
                                        where these belong. They will be imported
                                        unattached — set the game and the install
                                        path on each afterwards.
                                    </span>
                                </p>
                            )}

                            <div className="flex flex-col gap-2">
                                {rows.map((row) => (
                                    <RowEditor
                                        key={row.token}
                                        row={row}
                                        loading={loading}
                                        onPatch={patch}
                                    />
                                ))}
                            </div>
                        </>
                    )}

                    {error && (
                        <p className="mt-3 rounded-lg border border-danger/40 bg-danger/10 p-2.5 text-[11px] text-danger">
                            {error}
                        </p>
                    )}
                </div>

                <footer className="flex items-center justify-between gap-3 border-t border-border p-4">
                    <p className="text-[11px] text-muted">
                        {report
                            ? 'Nothing has been put in the game folder yet — deploy the sandbox when you are ready.'
                            : 'Files are copied into the app. The originals are left exactly where they are.'}
                    </p>

                    <div className="flex gap-2">
                        <button
                            type="button"
                            onClick={onClose}
                            className="rounded-lg border border-border px-3 py-1.5 text-xs"
                        >
                            {report ? 'Done' : 'Cancel'}
                        </button>

                        {!report && (
                            <button
                                type="button"
                                disabled={busy || loading || chosen.length < 1}
                                onClick={() => void run()}
                                className="rounded-lg bg-accent px-3 py-1.5 text-xs text-accent-foreground disabled:opacity-50"
                            >
                                {busy ? 'Importing…' : `Import ${chosen.length}`}
                            </button>
                        )}
                    </div>
                </footer>
            </div>
        </div>
    )
}

function RowEditor({
    row,
    loading,
    onPatch,
}: {
    row: Row
    loading: boolean
    onPatch: (token: string, next: Partial<Row>) => void
}) {
    const Icon = row.preview?.isDir
        ? FiFolder
        : row.payload === 'unpack'
          ? FiArchive
          : FiFile

    return (
        <div className="rounded-lg border border-border bg-surface-2 p-3">
            <div className="flex items-start gap-2.5">
                <input
                    type="checkbox"
                    checked={row.include}
                    onChange={(e) =>
                        onPatch(row.token, { include: e.target.checked })
                    }
                    className="mt-1.5"
                    aria-label={`Import ${row.name}`}
                />

                <Icon className="mt-1.5 size-4 shrink-0 text-muted" />

                <div className="min-w-0 flex-1">
                    <input
                        value={row.name}
                        onChange={(e) =>
                            onPatch(row.token, { name: e.target.value })
                        }
                        placeholder={loading ? 'Reading…' : 'Name this mod'}
                        className="w-full rounded-md border border-border bg-surface px-2 py-1 text-xs"
                        aria-label="Mod name"
                    />

                    <div className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-muted">
                        <span>{row.preview?.name ?? '…'}</span>

                        {row.preview && !row.preview.isDir && (
                            <span>{bytes(row.preview.bytes)}</span>
                        )}

                        {row.preview?.version && (
                            <span>v{row.preview.version}</span>
                        )}

                        {/*
                         * The one thing a `tmc.json` buys that nothing else
                         * does: this file is a release of a known item, so the
                         * user can be told to subscribe instead and get updates
                         * rather than a snapshot.
                         */}
                        {row.preview?.source && (
                            <span className="flex items-center gap-1 text-accent">
                                <FiLink className="size-3" />
                                From this site — subscribing instead keeps it
                                updated
                            </span>
                        )}
                    </div>

                    <div className="mt-2 grid gap-2 sm:grid-cols-2">
                        <label className="flex flex-col gap-1">
                            <span className="text-[10px] uppercase tracking-wide text-muted">
                                Installs to
                            </span>
                            <input
                                value={row.relPath}
                                onChange={(e) =>
                                    onPatch(row.token, { relPath: e.target.value })
                                }
                                placeholder="the game's folder"
                                className="rounded-md border border-border bg-surface px-2 py-1 font-mono text-[11px]"
                                aria-label="Install path inside the game folder"
                            />
                        </label>

                        {!row.preview?.isDir && (
                            <label className="flex flex-col gap-1">
                                <span className="text-[10px] uppercase tracking-wide text-muted">
                                    Treat it as
                                </span>
                                <Select
                                    label="How to treat this file"
                                    value={row.payload}
                                    fullWidth
                                    onChange={(value) =>
                                        onPatch(row.token, { payload: value })
                                    }
                                    options={PAYLOADS.filter(
                                        (p) => p.value !== 'folder'
                                    ).map((p) => ({
                                        value: p.value,
                                        label: p.label,
                                        hint: p.hint,
                                    }))}
                                />
                            </label>
                        )}
                    </div>
                </div>
            </div>
        </div>
    )
}

function Outcome({ report }: { report: ImportOutcomeT }) {
    return (
        <div className="flex flex-col gap-2">
            {report.imported.map((mod) => (
                <div
                    key={mod.id}
                    className="flex items-center gap-2 rounded-lg border border-border bg-surface-2 p-2.5 text-xs"
                >
                    <FiCheck className="size-3.5 shrink-0 text-success" />
                    <span className="min-w-0 flex-1 truncate">{mod.name}</span>
                    <span className="font-mono text-[11px] text-muted">
                        {mod.relPath || 'game folder'}
                    </span>
                </div>
            ))}

            {report.failed.map((line, i) => (
                <div
                    key={i}
                    className="flex items-start gap-2 rounded-lg border border-danger/40 bg-danger/10 p-2.5 text-[11px] text-danger"
                >
                    <FiAlertTriangle className="mt-0.5 size-3.5 shrink-0" />
                    <span>{line}</span>
                </div>
            ))}

            {report.imported.length < 1 && report.failed.length < 1 && (
                <p className="text-xs text-muted">Nothing was imported.</p>
            )}
        </div>
    )
}
