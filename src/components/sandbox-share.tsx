import { useState } from 'react'
import {
    FiAlertTriangle,
    FiCheck,
    FiCopy,
    FiDownloadCloud,
    FiShare2,
} from 'react-icons/fi'

import { ipc } from '~/lib/ipc/commands'
import { messageOf } from '~/lib/ipc'
import type { ImportReportT, SharedSandboxT } from '~/lib/ipc/schemas'

/**
 * **Sharing a sandbox** — export to a code, import from one.
 *
 * "Send me your modpack" and "I am setting up my second machine" are the two
 * moments, and every mod manager studied for this has some version of it.
 *
 * A CODE RATHER THAN A FILE
 * ------------------------
 * A file export needs somewhere to write and a file import needs something to
 * read, and the webview names neither path — that is the guarantee the whole
 * architecture rests on, and a save dialog is not worth trading it for. A
 * pasteable string needs no filesystem access at all, and it is what people do
 * with an exported profile anyway: put it in a message.
 *
 * IMPORTING SHOWS BEFORE IT DOES
 * -----------------------------
 * A code is a string from somebody else, and pressing Import on one can make
 * two hundred subscriptions on the account. So the preview is a separate step
 * and lists what is in it first — the same split every other consequential
 * action in this app has.
 */

export function ExportSandbox({ id, name }: { id: number; name: string }) {
    const [code, setCode] = useState<string | null>(null)
    const [copied, setCopied] = useState(false)
    const [error, setError] = useState<string | null>(null)

    const build = async () => {
        setError(null)

        try {
            setCode(await ipc.sandboxExport(id))
        } catch (err) {
            setError(messageOf(err))
        }
    }

    const copy = async () => {
        if (!code) return

        try {
            await navigator.clipboard.writeText(code)

            setCopied(true)
            window.setTimeout(() => setCopied(false), 2_000)
        } catch {
            // Clipboard access can be refused. The code is on screen and
            // selectable, which is the fallback that always works.
        }
    }

    if (!code)
        return (
            <button
                type="button"
                onClick={() => void build()}
                className="flex items-center gap-1.5 rounded-lg border border-border px-2.5 py-1 text-[11px]"
            >
                <FiShare2 className="size-3" />
                Share
                {error && <span className="text-danger">— {error}</span>}
            </button>
        )

    return (
        <div className="flex flex-col gap-2 rounded-lg border border-border p-3">
            <p className="text-[11px] text-muted">
                Anyone with this code can rebuild “{name}”. It lists the mods, not
                the files — they subscribe to their own copies.
            </p>

            <textarea
                readOnly
                value={code}
                rows={3}
                onFocus={(e) => e.currentTarget.select()}
                className="selectable w-full resize-none rounded-lg border border-border bg-surface-2 p-2 font-mono text-[10px]"
            />

            <div className="flex gap-2">
                <button
                    type="button"
                    onClick={() => void copy()}
                    className="flex items-center gap-1.5 rounded-lg bg-accent px-2.5 py-1 text-[11px] font-semibold text-accent-foreground"
                >
                    {copied ? (
                        <FiCheck className="size-3" />
                    ) : (
                        <FiCopy className="size-3" />
                    )}
                    {copied ? 'Copied' : 'Copy'}
                </button>

                <button
                    type="button"
                    onClick={() => setCode(null)}
                    className="rounded-lg border border-border px-2.5 py-1 text-[11px]"
                >
                    Done
                </button>
            </div>
        </div>
    )
}

export function ImportSandbox({
    onClose,
    onImported,
}: {
    onClose: () => void
    onImported: () => void
}) {
    const [code, setCode] = useState('')
    const [preview, setPreview] = useState<SharedSandboxT | null>(null)
    const [name, setName] = useState('')
    const [report, setReport] = useState<ImportReportT | null>(null)
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<string | null>(null)

    const look = async () => {
        setBusy(true)
        setError(null)

        try {
            const found = await ipc.sandboxImportPreview(code)

            setPreview(found)
            setName(found.name)
        } catch (err) {
            setError(messageOf(err))
        } finally {
            setBusy(false)
        }
    }

    const run = async () => {
        setBusy(true)
        setError(null)

        try {
            setReport(await ipc.sandboxImport(code, name))
            onImported()
        } catch (err) {
            setError(messageOf(err))
        } finally {
            setBusy(false)
        }
    }

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
            <div className="flex max-h-[90vh] w-full max-w-md flex-col overflow-hidden rounded-xl border border-border bg-surface">
                <header className="border-b border-border p-4">
                    <h2 className="text-sm font-semibold">Import a sandbox</h2>
                    <p className="text-[11px] text-muted">
                        Paste a code somebody shared. Nothing happens until you have
                        seen what is in it.
                    </p>
                </header>

                <div className="flex flex-1 flex-col gap-3 overflow-y-auto p-4">
                    {report ? (
                        <div className="flex flex-col gap-2 text-xs">
                            <p className="flex items-center gap-2 text-success">
                                <FiCheck className="size-3.5" />
                                Created “{report.sandbox?.name}” with {report.added}{' '}
                                {report.added === 1 ? 'mod' : 'mods'}.
                            </p>

                            {/*
                             * Not staged, and said out loud. The sandbox exists
                             * and is empty on disk until it is deployed, and
                             * somebody who launches the game now and finds
                             * nothing deserves to have been told why.
                             */}
                            <p className="text-muted">
                                Its files are not downloaded yet — open the sandbox
                                and press Deploy.
                            </p>

                            {report.skipped.length > 0 && (
                                <div className="rounded-lg border border-warning/40 bg-warning/10 p-2">
                                    <p className="font-medium text-warning">
                                        {report.skipped.length} could not be added
                                    </p>
                                    <ul className="mt-1 flex flex-col gap-0.5 text-[11px] text-muted">
                                        {report.skipped.map((line) => (
                                            <li key={line}>{line}</li>
                                        ))}
                                    </ul>
                                </div>
                            )}
                        </div>
                    ) : preview ? (
                        <>
                            <label className="flex flex-col gap-1">
                                <span className="text-[11px] font-medium uppercase tracking-wide text-muted">
                                    Name
                                </span>
                                <input
                                    type="text"
                                    value={name}
                                    maxLength={64}
                                    onChange={(e) => setName(e.target.value)}
                                    className="rounded-lg border border-border bg-surface-2 px-2 py-1.5 text-xs"
                                />
                            </label>

                            <div className="rounded-lg border border-border p-3 text-xs">
                                <p className="font-medium">
                                    {preview.appName ?? `Game ${preview.appId}`}
                                </p>
                                <p className="text-[11px] text-muted">
                                    {preview.loader ?? 'no loader'} ·{' '}
                                    {preview.gameVersion ?? 'any version'} ·{' '}
                                    {preview.environment}
                                </p>

                                <ul className="mt-2 flex max-h-48 flex-col gap-0.5 overflow-y-auto text-[11px]">
                                    {preview.mods.map((mod) => (
                                        <li
                                            key={`${mod.kind}:${mod.itemId}`}
                                            className={
                                                mod.enabled
                                                    ? ''
                                                    : 'text-muted line-through'
                                            }
                                        >
                                            {mod.name ||
                                                `${mod.kind} ${mod.itemId}`}
                                        </li>
                                    ))}
                                </ul>
                            </div>

                            <p className="text-[11px] text-muted">
                                Importing subscribes your account to{' '}
                                {preview.mods.length}{' '}
                                {preview.mods.length === 1 ? 'item' : 'items'}, so
                                they stay updated on your devices.
                            </p>
                        </>
                    ) : (
                        <label className="flex flex-col gap-1">
                            <span className="text-[11px] font-medium uppercase tracking-wide text-muted">
                                Code
                            </span>
                            <textarea
                                value={code}
                                rows={4}
                                onChange={(e) => setCode(e.target.value)}
                                placeholder="TMC1-…"
                                className="w-full resize-none rounded-lg border border-border bg-surface-2 p-2 font-mono text-[10px]"
                            />
                        </label>
                    )}

                    {error && (
                        <p className="flex items-start gap-2 rounded-lg border border-danger/40 bg-danger/10 p-2 text-xs text-danger">
                            <FiAlertTriangle className="mt-0.5 size-3.5 shrink-0" />
                            {error}
                        </p>
                    )}
                </div>

                <footer className="flex justify-end gap-2 border-t border-border p-4">
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
                            disabled={busy || (!preview && !code.trim())}
                            onClick={() => (preview ? void run() : void look())}
                            className="flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-xs font-semibold text-accent-foreground disabled:opacity-50"
                        >
                            <FiDownloadCloud className="size-3.5" />
                            {busy
                                ? 'Working…'
                                : preview
                                  ? 'Import'
                                  : 'Read the code'}
                        </button>
                    )}
                </footer>
            </div>
        </div>
    )
}
