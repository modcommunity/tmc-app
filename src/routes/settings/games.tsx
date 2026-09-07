import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { FiAlertTriangle, FiCheck, FiFolder, FiSearch, FiX } from 'react-icons/fi'

import { api } from '~/lib/api/client'
import { appLabel } from '~/lib/api/labels'
import { GameIcon } from '~/components/game-icon'
import { isIpcError } from '~/lib/ipc'
import { useSettings } from '~/lib/settings/provider'
import { ipc } from '~/lib/ipc/commands'
import type { DetectedGameT } from '~/lib/ipc/schemas'
import { Section } from '~/components/form'
import { useFolderPicker } from '~/components/folder-picker'

/**
 * Where each game is installed.
 *
 * These paths are the anchor of the plugin file jail: an installer plugin's
 * `gameDir` root is whatever is set here, and it can never write outside it. So
 * the path is always chosen by BROWSING, never typed — a text field is one a
 * compromised page could pre-fill, and it would then be the jail's own boundary
 * that was attacker-chosen.
 *
 * The screen is NOT what enforces that. `setGameDir` is a dedicated Rust
 * command (`settings_set_game_dir`) which validates the path and refuses to
 * anchor a jail at a drive root, a system directory, or anything containing the
 * app's own data; `settings_patch` refuses the field outright. Both hold
 * whatever this component does, which is the point — the picker is an
 * affordance, not a control.
 */
export default function GamesRoute() {
    const { app, setDownloadDir, setGameDir } = useSettings()
    const { pick, element: picker } = useFolderPicker()

    /*
     * Rust's refusal is shown, not swallowed. "Nothing happened when I chose a
     * folder" is the worst possible reading of a security check firing, and the
     * message names the reason — that the folder encloses the app's own files,
     * or is a system directory — because the fix is to pick a different one.
     */
    const [error, setError] = useState<string | null>(null)

    const guard = async (write: () => Promise<void>) => {
        setError(null)

        try {
            await write()
        } catch (err) {
            setError(isIpcError(err) ? err.message : 'That folder cannot be used.')
        }
    }

    // The game list comes from the facets endpoint, which only lists games that
    // actually have content — a picker of every App row would be mostly empty.
    const games = useQuery({
        queryKey: ['facets', 'mod'],
        queryFn: () => api.facets('mod'),
        staleTime: 10 * 60 * 1000,
    })

    if (!app) return <p className="text-sm text-muted">Loading…</p>

    const choose = async (appId: number, name: string) => {
        const dir = await pick({
            title: `Where is ${name} installed?`,
            hint: 'Installer plugins for this game can only write inside the folder you choose.',
            initial: app.gameDirs[String(appId)] ?? null,
        })

        if (dir === null) return

        await guard(() => setGameDir(appId, dir))
    }

    const clear = async (appId: number) => {
        await guard(() => setGameDir(appId, null))
    }

    return (
        <>
            <DetectPanel onApplied={() => setError(null)} onError={setError} />

            {error && (
                <p className="mb-3 flex items-start gap-2 rounded-lg border border-danger p-3 text-xs text-danger">
                    <FiAlertTriangle className="mt-0.5 size-3.5 shrink-0" />
                    {error}
                </p>
            )}

            <Section
                title="Install folders"
                hint="Installer plugins can only write inside the folder you pick here, and only for the game it belongs to."
            >
                {games.isPending && (
                    <p className="px-3 py-3 text-sm text-muted">Loading games…</p>
                )}

                {games.data?.apps.map((game) => {
                    const current = app.gameDirs[String(game.id)]
                    const label = appLabel(game)

                    return (
                        <div
                            key={game.id}
                            className="flex items-center justify-between gap-3 px-3 py-2.5"
                        >
                            <div className="flex min-w-0 items-center gap-2.5">
                                <GameIcon app={game} />

                                <div className="min-w-0">
                                    <p className="text-sm">{label}</p>
                                    <p className="selectable truncate text-xs text-muted">
                                        {current ?? 'Not set'}
                                    </p>
                                </div>
                            </div>

                            <div className="flex shrink-0 items-center gap-1">
                                <button
                                    type="button"
                                    onClick={() => void choose(game.id, label)}
                                    aria-label={`Choose folder for ${label}`}
                                    className="rounded-lg border border-border p-2 text-muted hover:text-foreground"
                                >
                                    <FiFolder className="size-4" />
                                </button>

                                {current && (
                                    <button
                                        type="button"
                                        onClick={() => void clear(game.id)}
                                        aria-label={`Clear folder for ${label}`}
                                        className="rounded-lg border border-border p-2 text-muted hover:text-danger"
                                    >
                                        <FiX className="size-4" />
                                    </button>
                                )}
                            </div>
                        </div>
                    )
                })}
            </Section>

            <Section
                title="Downloads"
                hint="Where installers put files before unpacking them. Defaults to the app's own cache."
            >
                <div className="flex items-center justify-between gap-3 px-3 py-2.5">
                    <p className="selectable min-w-0 truncate text-xs text-muted">
                        {app.downloadDir ?? 'App cache (cleared on launch)'}
                    </p>

                    <button
                        type="button"
                        onClick={() => {
                            void pick({
                                title: 'Choose a download folder',
                                hint: 'Installers put files here before unpacking them.',
                                initial: app.downloadDir,
                            }).then((dir) => {
                                if (dir !== null)
                                    void guard(() => setDownloadDir(dir))
                            })
                        }}
                        aria-label="Choose download folder"
                        className="shrink-0 rounded-lg border border-border p-2 text-muted hover:text-foreground"
                    >
                        <FiFolder className="size-4" />
                    </button>
                </div>
            </Section>

            {picker}
        </>
    )
}

/**
 * Find installed games instead of asking somebody to type a path.
 *
 * The scan reads what the launchers already wrote down — Steam's manifests,
 * Epic's, GOG Galaxy's database — and shows what it found. **It changes
 * nothing on its own.** Applying a result is a separate click, and it goes
 * through the same validator a hand-picked folder does, because a folder is not
 * more trustworthy for having been found automatically.
 *
 * Unmatched games are still listed, greyed, with their launcher named. A user
 * whose game is plainly installed and does not appear needs to be able to see
 * that the app FOUND it and could not match it, rather than concluding the scan
 * does not work.
 */
function DetectPanel({
    onApplied,
    onError,
}: {
    onApplied: () => void
    onError: (message: string) => void
}) {
    const [found, setFound] = useState<DetectedGameT[] | null>(null)
    const [busy, setBusy] = useState(false)
    const [applied, setApplied] = useState<Set<string>>(new Set())

    const scan = async () => {
        setBusy(true)

        try {
            setFound(await ipc.detectGames())
            onApplied()
        } catch (err) {
            onError(isIpcError(err) ? err.message : 'The scan failed.')
        } finally {
            setBusy(false)
        }
    }

    const apply = async (game: DetectedGameT) => {
        if (!game.slug) return

        try {
            await ipc.detectApply(game.slug, game.path)

            setApplied((was) => new Set(was).add(game.path))
            onApplied()
        } catch (err) {
            onError(isIpcError(err) ? err.message : 'That folder cannot be used.')
        }
    }

    const matched = (found ?? []).filter((g) => g.slug)
    const unmatched = (found ?? []).filter((g) => !g.slug)

    return (
        <Section
            title="Find my games"
            hint="Reads what Steam, Epic, GOG and the rest already know. Nothing is changed until you apply a result."
        >
            <div className="flex items-center justify-between gap-3 px-3 py-2.5">
                <p className="text-xs text-muted">
                    {found === null
                        ? 'Not scanned yet.'
                        : `${matched.length} game${matched.length === 1 ? '' : 's'} the app can manage, ${unmatched.length} other${unmatched.length === 1 ? '' : 's'} found.`}
                </p>

                <button
                    type="button"
                    disabled={busy}
                    onClick={() => void scan()}
                    className="flex shrink-0 items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-xs hover:border-accent disabled:opacity-50"
                >
                    <FiSearch className="size-3" />
                    {busy ? 'Scanning…' : found === null ? 'Scan' : 'Scan again'}
                </button>
            </div>

            {matched.map((game) => (
                <div
                    key={`${game.source}:${game.path}`}
                    className="flex items-center justify-between gap-3 px-3 py-2.5"
                >
                    <div className="min-w-0">
                        <p className="text-sm">
                            {game.name}
                            <span className="ml-2 rounded bg-surface-tertiary px-1.5 py-0.5 text-[0.6rem] uppercase text-muted">
                                {game.source}
                            </span>
                        </p>
                        <p className="selectable truncate text-xs text-muted">
                            {game.path}
                        </p>
                        {game.replaces && (
                            <p className="truncate text-[0.7rem] text-warning">
                                Replaces {game.replaces}
                            </p>
                        )}
                    </div>

                    {game.alreadySet || applied.has(game.path) ? (
                        <span className="flex shrink-0 items-center gap-1 text-xs text-success">
                            <FiCheck className="size-3.5" />
                            In use
                        </span>
                    ) : (
                        <button
                            type="button"
                            onClick={() => void apply(game)}
                            className="shrink-0 rounded-lg bg-accent px-3 py-1.5 text-xs text-accent-foreground"
                        >
                            Use this
                        </button>
                    )}
                </div>
            ))}

            {unmatched.length > 0 && (
                <details className="px-3 py-2.5">
                    <summary className="cursor-pointer text-xs text-muted">
                        {unmatched.length} game
                        {unmatched.length === 1 ? '' : 's'} the app does not support
                        yet
                    </summary>

                    <ul className="mt-2 flex flex-col gap-1">
                        {unmatched.slice(0, 100).map((game) => (
                            <li
                                key={`${game.source}:${game.path}`}
                                className="truncate text-[0.7rem] text-muted"
                            >
                                {game.name}{' '}
                                <span className="opacity-60">({game.source})</span>
                            </li>
                        ))}
                    </ul>
                </details>
            )}
        </Section>
    )
}
