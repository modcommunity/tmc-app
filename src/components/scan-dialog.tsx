import { useCallback, useEffect, useMemo, useState } from 'react'
import {
    FiAlertTriangle,
    FiCheck,
    FiFolder,
    FiFolderPlus,
    FiHardDrive,
    FiSearch,
    FiX,
} from 'react-icons/fi'

import { ipc } from '~/lib/ipc/commands'
import { messageOf, subscribe } from '~/lib/ipc'
import { useFolderPicker } from '~/components/folder-picker'
import {
    ScanProgressSchema,
    type ApplyOutcomeT,
    type DetectedGameT,
    type ScanReportT,
    type WalkLimitsT,
} from '~/lib/ipc/schemas'

/**
 * **Finding games**, in the two ways this app can look.
 *
 * *Check the launchers* reads what Steam, Epic and GOG already wrote down. It
 * is instant, exact, and covers most machines, so it runs the moment this
 * opens and needs no input at all.
 *
 * *Scan folders* walks directories the user has ticked. It is for everything a
 * launcher does not know about — a game copied from another PC, a dedicated
 * server unpacked by hand, a drive that was moved — and it is the reason this
 * dialog has a folder list in it rather than a single button.
 *
 * WHY THE FOLDERS ARE TICKED RATHER THAN DISCOVERED
 * -----------------------------------------------
 * "Scan my computer" is a scan of somebody's documents, their photos and every
 * network share the machine has mounted. The app has no business enumerating
 * any of that, so the roots come from a person one tick at a time and there is
 * no button that skips the choosing. Rust bounds depth, directory count and
 * wall clock on top of that — see `tmc_core::detect::walk`.
 *
 * NOTHING IS APPLIED WITHOUT BEING SHOWN
 * -------------------------------------
 * A scan produces candidates and a second, explicit step configures them. That
 * split is the same one `detect_games` has always had, and it is what makes the
 * result auditable: a game directory is a plugin jail anchor, so pointing one
 * somewhere is a security decision and not a search result.
 */

type Phase = 'idle' | 'scanning' | 'done'

/** The presets, so nobody has to reason about a directory budget. */
const DEPTHS: { value: number; label: string; hint: string }[] = [
    { value: 3, label: 'Shallow', hint: 'Fast. Finds games near the top.' },
    {
        value: 5,
        label: 'Normal',
        hint: 'Reaches a Steam library inside a drive.',
    },
    { value: 8, label: 'Deep', hint: 'Slow. For unusual folder layouts.' },
]

export default function ScanDialog({
    onClose,
    onApplied,
}: {
    onClose: () => void
    onApplied: () => void
}) {
    const { pick, element: picker } = useFolderPicker()

    const [roots, setRoots] = useState<{ label: string; path: string }[]>([])
    const [ticked, setTicked] = useState<Set<string>>(new Set())
    const [depth, setDepth] = useState(5)
    const [hidden, setHidden] = useState(false)

    const [phase, setPhase] = useState<Phase>('idle')
    const [progress, setProgress] = useState<{ dirs: number; path: string } | null>(
        null
    )
    const [report, setReport] = useState<ScanReportT | null>(null)
    const [launcher, setLauncher] = useState<DetectedGameT[] | null>(null)
    const [chosen, setChosen] = useState<Set<string>>(new Set())
    const [outcomes, setOutcomes] = useState<ApplyOutcomeT[] | null>(null)
    const [error, setError] = useState<string | null>(null)
    const [busy, setBusy] = useState(false)

    // The launcher read costs nothing and needs no input, so it happens on open
    // rather than behind a button somebody has to know to press.
    useEffect(() => {
        void ipc
            .detectGames()
            .then(setLauncher)
            .catch((err: unknown) => setError(messageOf(err)))

        void ipc
            .fsRoots()
            .then(setRoots)
            .catch(() => undefined)
    }, [])

    useEffect(() => {
        let live = true
        let stop: (() => void) | null = null

        void subscribe('tmc://scan-progress', ScanProgressSchema, (payload) => {
            setProgress(payload)
        })
            .then((unlisten) => {
                if (live) stop = unlisten
                else unlisten()
            })
            .catch(() => undefined)

        return () => {
            live = false
            stop?.()
        }
    }, [])

    const addFolder = async () => {
        const dir = await pick({
            title: 'Choose a folder to scan',
            hint: 'Only this folder and what is inside it will be looked at.',
        })

        if (dir === null) return

        setRoots((prev) =>
            prev.some((r) => r.path === dir)
                ? prev
                : [...prev, { label: dir, path: dir }]
        )

        setTicked((prev) => new Set(prev).add(dir))
    }

    const scan = useCallback(async () => {
        const selected = [...ticked]

        if (selected.length < 1) return

        setPhase('scanning')
        setError(null)
        setReport(null)
        setProgress(null)

        const limits: WalkLimitsT = {
            depth,
            maxDirs: 60_000,
            budgetSecs: 90,
            hidden,
        }

        try {
            const result = await ipc.detectScan(selected, limits)

            setReport(result)
            /*
             * Pre-ticked, except what is already pointed there. The button says
             * "automatically integrate what it finds" — making somebody tick
             * twelve rows they just asked for would be a worse version of the
             * same thing — but a row that would REPLACE a folder they chose
             * earlier is left alone, because silently repointing a jail anchor
             * is exactly the decision that has to be theirs.
             */
            setChosen(
                new Set(
                    result.games
                        .filter((g) => g.slug && !g.alreadySet && !g.replaces)
                        .map((g) => g.path)
                )
            )
        } catch (err) {
            setError(messageOf(err))
        } finally {
            setPhase('done')
        }
    }, [ticked, depth, hidden])

    const apply = useCallback(
        async (games: DetectedGameT[]) => {
            const pairs = games
                .filter((g) => g.slug && chosen.has(g.path))
                .map((g) => [g.slug as string, g.path] as [string, string])

            if (pairs.length < 1) return

            setBusy(true)
            setError(null)

            try {
                setOutcomes(await ipc.detectApplyMany(pairs))
                onApplied()
            } catch (err) {
                setError(messageOf(err))
            } finally {
                setBusy(false)
            }
        },
        [chosen, onApplied]
    )

    /*
     * One list, from both sources.
     *
     * A user does not care whether their copy of a game was found in a Steam
     * manifest or by walking a folder — they care that it is here and whether
     * it is set up. Keeping two lists would make them check both.
     */
    const found = useMemo(() => {
        const rows = [...(launcher ?? []), ...(report?.games ?? [])]
        const seen = new Set<string>()

        return rows.filter((row) => {
            if (!row.slug) return false
            if (seen.has(row.path)) return false

            seen.add(row.path)

            return true
        })
    }, [launcher, report])

    const applicable = found.filter((g) => !g.alreadySet)

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
            <div className="flex max-h-[90vh] w-full max-w-2xl flex-col overflow-hidden rounded-xl border border-border bg-surface">
                <header className="flex items-center justify-between border-b border-border p-4">
                    <div>
                        <h2 className="text-sm font-semibold">Find games</h2>
                        <p className="text-[11px] text-muted">
                            Read from the launchers, or look through folders you
                            choose.
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
                    <section className="mb-4">
                        <h3 className="mb-2 flex items-center gap-1.5 text-xs font-semibold">
                            <FiHardDrive className="size-3.5" />
                            Folders to scan
                        </h3>

                        <div className="flex flex-col gap-1">
                            {roots.map((root) => (
                                <label
                                    key={root.path}
                                    className="flex items-center gap-2 rounded-lg border border-border px-2.5 py-1.5 text-xs"
                                >
                                    <input
                                        type="checkbox"
                                        checked={ticked.has(root.path)}
                                        onChange={(e) =>
                                            setTicked((prev) => {
                                                const next = new Set(prev)

                                                if (e.target.checked)
                                                    next.add(root.path)
                                                else next.delete(root.path)

                                                return next
                                            })
                                        }
                                    />
                                    <FiFolder className="size-3 shrink-0 text-muted" />
                                    <span className="truncate">{root.label}</span>
                                    <span className="selectable ml-auto truncate text-[10px] text-muted">
                                        {root.path}
                                    </span>
                                </label>
                            ))}

                            <button
                                type="button"
                                onClick={() => void addFolder()}
                                className="flex items-center gap-1.5 self-start rounded-lg border border-dashed border-border px-2.5 py-1.5 text-[11px] text-muted"
                            >
                                <FiFolderPlus className="size-3" />
                                Add another folder
                            </button>
                        </div>

                        <div className="mt-3 flex flex-wrap items-center gap-3">
                            <div className="flex items-center gap-1.5">
                                {DEPTHS.map((preset) => (
                                    <button
                                        key={preset.value}
                                        type="button"
                                        title={preset.hint}
                                        onClick={() => setDepth(preset.value)}
                                        aria-pressed={depth === preset.value}
                                        className={`rounded-full border px-2.5 py-1 text-[11px] ${
                                            depth === preset.value
                                                ? 'border-accent bg-accent/10 text-accent'
                                                : 'border-border text-muted'
                                        }`}
                                    >
                                        {preset.label}
                                    </button>
                                ))}
                            </div>

                            <label className="flex items-center gap-1.5 text-[11px] text-muted">
                                <input
                                    type="checkbox"
                                    checked={hidden}
                                    onChange={(e) => setHidden(e.target.checked)}
                                />
                                {/* Worth offering rather than hard-coding:
                                    `~/.minecraft` and `~/.local/share/Steam`
                                    are real install locations, so a scan of a
                                    Linux home with this off finds nothing. */}
                                Look inside hidden folders
                            </label>

                            <button
                                type="button"
                                disabled={ticked.size < 1 || phase === 'scanning'}
                                onClick={() => void scan()}
                                className="ml-auto flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-xs font-semibold text-accent-foreground disabled:opacity-50"
                            >
                                <FiSearch className="size-3.5" />
                                {phase === 'scanning' ? 'Scanning…' : 'Scan'}
                            </button>
                        </div>

                        {phase === 'scanning' && (
                            <div className="mt-3 rounded-lg border border-border bg-surface-2 p-2.5">
                                <p className="truncate text-[11px] text-muted">
                                    {progress
                                        ? `${progress.dirs.toLocaleString()} folders · ${progress.path}`
                                        : 'Starting…'}
                                </p>
                                <button
                                    type="button"
                                    onClick={() => void ipc.detectScanCancel()}
                                    className="mt-1.5 text-[11px] underline"
                                >
                                    Stop
                                </button>
                            </div>
                        )}

                        {report && report.stop !== 'completed' && (
                            <p className="mt-2 flex items-start gap-1.5 rounded-lg border border-warning/40 bg-warning/10 p-2 text-[11px] text-warning">
                                <FiAlertTriangle className="mt-0.5 size-3 shrink-0" />
                                {report.stop === 'cancelled'
                                    ? 'Stopped before it finished, so there may be more to find.'
                                    : report.stop === 'timeLimit'
                                      ? 'Ran out of time before it finished. Scan a narrower folder to reach the rest.'
                                      : 'Reached its folder limit before it finished. Scan a narrower folder to reach the rest.'}
                            </p>
                        )}

                        {report?.unreadable.map(([path, why]) => (
                            <p
                                key={path}
                                className="mt-1 truncate text-[11px] text-muted"
                            >
                                Could not read {path} — {why}
                            </p>
                        ))}
                    </section>

                    <section>
                        <h3 className="mb-2 text-xs font-semibold">
                            Found{' '}
                            <span className="text-muted">({found.length})</span>
                        </h3>

                        {found.length < 1 ? (
                            <p className="text-[11px] text-muted">
                                {launcher === null
                                    ? 'Checking the launchers…'
                                    : 'Nothing the app recognises yet. Tick a folder above and scan it.'}
                            </p>
                        ) : (
                            <div className="flex flex-col gap-1">
                                {found.map((game) => {
                                    const outcome = outcomes?.find(
                                        (o) => o.path === game.path
                                    )

                                    return (
                                        <label
                                            key={game.path}
                                            className="flex items-start gap-2 rounded-lg border border-border px-2.5 py-2 text-xs"
                                        >
                                            <input
                                                type="checkbox"
                                                className="mt-0.5"
                                                disabled={game.alreadySet}
                                                checked={
                                                    game.alreadySet ||
                                                    chosen.has(game.path)
                                                }
                                                onChange={(e) =>
                                                    setChosen((prev) => {
                                                        const next = new Set(prev)

                                                        if (e.target.checked)
                                                            next.add(game.path)
                                                        else next.delete(game.path)

                                                        return next
                                                    })
                                                }
                                            />

                                            <span className="min-w-0 flex-1">
                                                <span className="flex items-center gap-1.5">
                                                    <span className="truncate font-medium">
                                                        {game.name}
                                                    </span>
                                                    <span className="shrink-0 rounded bg-surface-2 px-1 text-[10px] text-muted">
                                                        {game.source}
                                                    </span>
                                                </span>

                                                <span className="selectable block truncate text-[11px] text-muted">
                                                    {game.path}
                                                </span>

                                                {game.alreadySet && (
                                                    <span className="flex items-center gap-1 text-[11px] text-success">
                                                        <FiCheck className="size-3" />
                                                        Already set up
                                                    </span>
                                                )}

                                                {game.replaces && (
                                                    <span className="block text-[11px] text-warning">
                                                        Would replace{' '}
                                                        {game.replaces}
                                                    </span>
                                                )}

                                                {outcome && !outcome.ok && (
                                                    <span className="block text-[11px] text-danger">
                                                        {outcome.error}
                                                    </span>
                                                )}
                                            </span>
                                        </label>
                                    )
                                })}
                            </div>
                        )}
                    </section>

                    {error && (
                        <p className="mt-3 flex items-start gap-2 rounded-lg border border-danger/40 bg-danger/10 p-2 text-xs text-danger">
                            <FiAlertTriangle className="mt-0.5 size-3.5 shrink-0" />
                            {error}
                        </p>
                    )}
                </div>

                <footer className="flex items-center justify-end gap-2 border-t border-border p-4">
                    {outcomes && (
                        <p className="mr-auto text-[11px] text-muted">
                            Set up {outcomes.filter((o) => o.ok).length} of{' '}
                            {outcomes.length}.
                        </p>
                    )}

                    <button
                        type="button"
                        onClick={onClose}
                        className="rounded-lg border border-border px-3 py-1.5 text-xs"
                    >
                        Close
                    </button>

                    <button
                        type="button"
                        disabled={busy || chosen.size < 1}
                        onClick={() => void apply(applicable)}
                        className="rounded-lg bg-accent px-3 py-1.5 text-xs font-semibold text-accent-foreground disabled:opacity-50"
                    >
                        {busy
                            ? 'Setting up…'
                            : `Set up ${chosen.size || ''}`.trim()}
                    </button>
                </footer>
            </div>

            {picker}
        </div>
    )
}
