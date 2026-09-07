/**
 * The confirmation shown before anything starts.
 *
 * This is the whole reason a declarative launcher is acceptable: the exact
 * program, arguments and working folder are on screen before the process
 * exists. A launcher that cannot show that is one nobody can audit.
 *
 * Shared by the install launcher and the sandbox launcher deliberately. They
 * resolve their plans differently — an install's from its own directory, a
 * sandbox's from its game folder and possibly with a virtual filesystem — but
 * what the user is being asked to approve is the same thing, and two dialogs
 * would eventually show two different amounts of it.
 *
 * `command` is a readable one-liner for DISPLAY. Nothing consumes it, and it
 * must never be fed to a shell — the real launch uses the argv vector.
 */

import { FiAlertTriangle, FiLayers, FiPlay } from 'react-icons/fi'

import type { LaunchPreviewT } from '~/lib/ipc/schemas'

export function LaunchDialog({
    title,
    preview,
    error,
    busy,
    onCancel,
    onConfirm,
}: {
    title: string
    preview: LaunchPreviewT | null
    error: string | null
    busy?: boolean
    onCancel: () => void
    onConfirm: () => void
}) {
    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
            <div className="w-full max-w-lg rounded-xl border border-border bg-surface p-4">
                <h2 className="text-sm font-semibold">{title}</h2>

                {error ? (
                    <p className="mt-3 flex items-start gap-2 rounded-lg border border-danger/40 bg-danger/10 p-2 text-xs text-danger">
                        <FiAlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                        {error}
                    </p>
                ) : null}

                {preview ? (
                    <>
                        <p className="mt-3 text-xs text-muted">
                            This is exactly what will run. Nothing goes through a
                            shell — every argument is passed separately.
                        </p>

                        <pre className="selectable mt-2 max-h-48 overflow-auto rounded-lg border border-border bg-surface-2 p-2 text-[11px]">
                            {preview.command}
                        </pre>

                        {preview.plan.cwd ? (
                            <p className="selectable mt-2 text-[11px] text-muted">
                                Working folder: {preview.plan.cwd}
                            </p>
                        ) : null}

                        {preview.plan.vfs ? (
                            /*
                             * Worth saying out loud, because it is the one case
                             * where the game folder on disk is NOT what the game
                             * will see — and somebody who goes looking for their
                             * mods in it and finds nothing deserves to have been
                             * told why.
                             */
                            <p className="mt-2 flex items-start gap-1.5 rounded-lg border border-border bg-surface-2 p-2 text-[11px] text-muted">
                                <FiLayers className="mt-0.5 h-3 w-3 shrink-0" />
                                <span>
                                    The mods are loaded into the game as it starts,
                                    so the game folder itself stays untouched. They
                                    will not appear in it.
                                </span>
                            </p>
                        ) : null}

                        <p className="mt-1 text-[11px] text-muted">
                            Rule: {preview.plan.rule}
                        </p>
                    </>
                ) : error ? null : (
                    <p className="mt-3 text-xs text-muted">Resolving…</p>
                )}

                <div className="mt-4 flex justify-end gap-2">
                    <button
                        type="button"
                        onClick={onCancel}
                        className="rounded-lg border border-border px-3 py-1.5 text-xs"
                    >
                        Cancel
                    </button>

                    <button
                        type="button"
                        disabled={!preview || busy}
                        onClick={onConfirm}
                        className="flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-xs font-semibold text-accent-foreground disabled:opacity-50"
                    >
                        <FiPlay className="h-3.5 w-3.5" />
                        {busy ? 'Starting…' : 'Launch'}
                    </button>
                </div>
            </div>
        </div>
    )
}
