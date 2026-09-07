import { useCallback, useEffect, useMemo, useState } from 'react'
import {
    FiAlertTriangle,
    FiCheck,
    FiFileText,
    FiRotateCcw,
    FiSave,
    FiSearch,
} from 'react-icons/fi'

import { ipc } from '~/lib/ipc/commands'
import { messageOf } from '~/lib/ipc'
import type { ConfigFileT } from '~/lib/ipc/schemas'

/**
 * **Editing a game's own settings files.**
 *
 * The feature every studied mod manager has and this app was missing. A loader
 * writes `BepInEx/config/com.author.mod.cfg` the first time the game runs, one
 * value in it is wrong, and fixing it meant leaving the app.
 *
 * WHAT IS OFFERED, AND WHY IT IS SO LITTLE
 * ---------------------------------------
 * Only what the GAME declared in `plugins/app/<slug>/config.json` — the same
 * arrangement as "where do this game's mods go". A game that declares nothing
 * has no editor here, and the tab is absent rather than empty: sweeping a game
 * folder for `*.cfg` would offer somebody their save file, and a wrong guess
 * about which files are safe to edit is worse than no editor.
 *
 * A PLAIN TEXT BOX, DELIBERATELY
 * -----------------------------
 * Not a schema-driven form with typed fields and sliders. Gale has one and it
 * is genuinely nicer where it works — but it works by understanding BepInEx's
 * own config dialect, and the moment a game uses TOML, an INI, a properties
 * file or something bespoke, a form either cannot render it or renders it
 * wrongly and writes back something the loader will not parse.
 *
 * A text box round-trips every format exactly. It is the version that is never
 * wrong, and it is what somebody would have opened the file in anyway.
 *
 * The safety is elsewhere and is real: the previous contents are copied into
 * the app's backup folder on every save, the write goes to a temporary file and
 * is renamed, and a file whose bytes are not UTF-8 is refused rather than
 * mangled through a text box.
 */

function bytes(n: number): string {
    if (n < 1024) return `${n} B`
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`

    return `${(n / (1024 * 1024)).toFixed(1)} MB`
}

export default function ConfigEditor({ sandboxId }: { sandboxId: number }) {
    const [files, setFiles] = useState<ConfigFileT[] | null>(null)
    const [search, setSearch] = useState('')
    const [open, setOpen] = useState<ConfigFileT | null>(null)

    const [original, setOriginal] = useState('')
    const [draft, setDraft] = useState('')
    const [loading, setLoading] = useState(false)
    const [saving, setSaving] = useState(false)
    const [saved, setSaved] = useState(false)
    const [error, setError] = useState<string | null>(null)

    useEffect(() => {
        let live = true

        setFiles(null)

        void ipc
            .configList(sandboxId)
            .then((rows) => {
                if (live) setFiles(rows)
            })
            .catch((err: unknown) => {
                if (!live) return

                setFiles([])
                setError(messageOf(err))
            })

        return () => {
            live = false
        }
    }, [sandboxId])

    const load = useCallback(
        async (file: ConfigFileT) => {
            setOpen(file)
            setError(null)
            setSaved(false)

            if (!file.editable) {
                setOriginal('')
                setDraft('')

                return
            }

            setLoading(true)

            try {
                const text = await ipc.configRead(sandboxId, file.root, file.path)

                setOriginal(text)
                setDraft(text)
            } catch (err) {
                setError(messageOf(err))
                setOriginal('')
                setDraft('')
            } finally {
                setLoading(false)
            }
        },
        [sandboxId]
    )

    const save = useCallback(async () => {
        if (!open) return

        setSaving(true)
        setError(null)

        try {
            await ipc.configWrite(sandboxId, open.root, open.path, draft)

            setOriginal(draft)
            setSaved(true)
            window.setTimeout(() => setSaved(false), 2_000)
        } catch (err) {
            setError(messageOf(err))
        } finally {
            setSaving(false)
        }
    }, [open, draft, sandboxId])

    const groups = useMemo(() => {
        const term = search.trim().toLowerCase()

        const shown = (files ?? []).filter(
            (f) => !term || f.name.toLowerCase().includes(term)
        )

        const out = new Map<string, ConfigFileT[]>()

        for (const file of shown) {
            const list = out.get(file.group) ?? []

            list.push(file)
            out.set(file.group, list)
        }

        return [...out.entries()]
    }, [files, search])

    const dirty = draft !== original

    if (files === null)
        return <p className="p-3 text-xs text-muted">Looking for settings files…</p>

    if (files.length < 1)
        return (
            <div className="p-3">
                <p className="text-xs text-muted">
                    {error ??
                        'No settings files yet. Most games write theirs the first time they run — start the game once, then come back.'}
                </p>
            </div>
        )

    return (
        <div className="flex min-h-0 flex-1 flex-col gap-2 md:flex-row">
            <div className="flex w-full shrink-0 flex-col gap-2 md:w-64">
                <div className="relative">
                    <FiSearch className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted" />
                    <input
                        type="search"
                        value={search}
                        onChange={(e) => setSearch(e.target.value)}
                        placeholder="Search files"
                        className="w-full rounded-lg border border-border bg-surface-2 py-1.5 pl-8 pr-2 text-xs"
                    />
                </div>

                <div className="max-h-72 overflow-y-auto rounded-lg border border-border md:max-h-none">
                    {groups.map(([group, rows]) => (
                        <div key={group}>
                            <p className="sticky top-0 border-b border-border bg-surface px-2.5 py-1 text-[10px] font-medium uppercase tracking-wide text-muted">
                                {group}
                            </p>

                            {rows.map((file) => (
                                <button
                                    key={`${file.root}:${file.path}`}
                                    type="button"
                                    onClick={() => void load(file)}
                                    className={`flex w-full items-center gap-2 border-b border-border px-2.5 py-1.5 text-left text-xs last:border-b-0 ${
                                        open?.path === file.path
                                            ? 'bg-accent/10 text-accent'
                                            : ''
                                    }`}
                                >
                                    <FiFileText className="size-3 shrink-0 text-muted" />
                                    <span className="truncate">{file.name}</span>
                                    <span className="ml-auto shrink-0 text-[10px] text-muted">
                                        {bytes(file.size)}
                                    </span>
                                </button>
                            ))}
                        </div>
                    ))}
                </div>
            </div>

            <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-2">
                {!open ? (
                    <p className="text-xs text-muted">
                        Pick a file to edit it. Its previous contents are kept every
                        time you save.
                    </p>
                ) : !open.editable ? (
                    <p className="flex items-start gap-2 rounded-lg border border-border bg-surface-2 p-2 text-xs text-muted">
                        <FiAlertTriangle className="mt-0.5 size-3.5 shrink-0 text-warning" />
                        {open.reason ?? 'This file cannot be edited here.'}
                    </p>
                ) : (
                    <>
                        <div className="flex flex-wrap items-center gap-2">
                            <span className="selectable min-w-0 flex-1 truncate text-[11px] text-muted">
                                {open.path}
                            </span>

                            {dirty && (
                                <button
                                    type="button"
                                    onClick={() => setDraft(original)}
                                    className="flex items-center gap-1.5 rounded-lg border border-border px-2.5 py-1 text-[11px]"
                                >
                                    <FiRotateCcw className="size-3" />
                                    Revert
                                </button>
                            )}

                            <button
                                type="button"
                                disabled={!dirty || saving || loading}
                                onClick={() => void save()}
                                className="flex items-center gap-1.5 rounded-lg bg-accent px-2.5 py-1 text-[11px] font-semibold text-accent-foreground disabled:opacity-50"
                            >
                                {saved ? (
                                    <FiCheck className="size-3" />
                                ) : (
                                    <FiSave className="size-3" />
                                )}
                                {saving ? 'Saving…' : saved ? 'Saved' : 'Save'}
                            </button>
                        </div>

                        <textarea
                            value={loading ? '' : draft}
                            readOnly={loading}
                            onChange={(e) => setDraft(e.target.value)}
                            spellCheck={false}
                            /*
                             * `off` on all four, because a settings file is not
                             * prose: a mobile keyboard capitalising a key name
                             * or "correcting" an enum value produces a file the
                             * loader silently ignores.
                             */
                            autoCapitalize="off"
                            autoCorrect="off"
                            autoComplete="off"
                            className="selectable min-h-64 flex-1 resize-none rounded-lg border border-border bg-surface-2 p-2 font-mono text-[11px] leading-relaxed"
                        />

                        <p className="text-[10px] text-muted">
                            Saving keeps a copy of what was there before. The game
                            reads this file when it starts, so restart it to see a
                            change.
                        </p>
                    </>
                )}

                {error && (
                    <p className="flex items-start gap-2 rounded-lg border border-danger/40 bg-danger/10 p-2 text-xs text-danger">
                        <FiAlertTriangle className="mt-0.5 size-3.5 shrink-0" />
                        {error}
                    </p>
                )}
            </div>
        </div>
    )
}
