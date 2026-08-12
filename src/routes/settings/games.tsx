import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { FiAlertTriangle, FiFolder, FiX } from 'react-icons/fi'

import { api } from '~/lib/api/client'
import { appLabel } from '~/lib/api/labels'
import { isIpcError } from '~/lib/ipc'
import { useSettings } from '~/lib/settings/provider'
import { Section } from '~/components/form'
import { useFolderPicker } from '~/components/folder-picker'

/**
 * Where each game is installed.
 *
 * These paths are the anchor of the plugin sandbox: an installer plugin's
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
                            <div className="min-w-0">
                                <p className="text-sm">{label}</p>
                                <p className="selectable truncate text-xs text-muted">
                                    {current ?? 'Not set'}
                                </p>
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
