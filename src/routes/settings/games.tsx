import { useQuery } from '@tanstack/react-query'
import { open } from '@tauri-apps/plugin-dialog'
import { FiFolder, FiX } from 'react-icons/fi'

import { api } from '~/lib/api/client'
import { useSettings } from '~/lib/settings/provider'
import { Section } from '~/components/form'

/**
 * Where each game is installed.
 *
 * These paths are the anchor of the plugin sandbox: an installer plugin's
 * `gameDir` root is whatever is set here, and it can never write outside it.
 * So the path has to come from the OS folder picker, never from a text field —
 * a typed path is one a compromised page could pre-fill, and it would then be
 * the jail's own boundary that was attacker-chosen.
 */
export default function GamesRoute() {
    const { app, setApp } = useSettings()

    // The game list comes from the facets endpoint, which only lists games that
    // actually have content — a picker of every App row would be mostly empty.
    const games = useQuery({
        queryKey: ['facets', 'mod'],
        queryFn: () => api.facets('mod'),
        staleTime: 10 * 60 * 1000,
    })

    if (!app) return <p className="text-sm text-muted">Loading…</p>

    const choose = async (appId: number) => {
        const dir = await open({
            directory: true,
            multiple: false,
            title: 'Choose the game install folder',
        })

        if (typeof dir !== 'string') return

        await setApp({ gameDirs: { ...app.gameDirs, [String(appId)]: dir } })
    }

    const clear = async (appId: number) => {
        const next = { ...app.gameDirs }
        delete next[String(appId)]

        await setApp({ gameDirs: next })
    }

    return (
        <>
            <Section
                title="Install folders"
                hint="Installer plugins can only write inside the folder you pick here, and only for the game it belongs to."
            >
                {games.isPending && (
                    <p className="px-3 py-3 text-sm text-muted">Loading games…</p>
                )}

                {games.data?.apps.map((game) => {
                    const current = app.gameDirs[String(game.id)]

                    return (
                        <div
                            key={game.id}
                            className="flex items-center justify-between gap-3 px-3 py-2.5"
                        >
                            <div className="min-w-0">
                                <p className="text-sm">{game.name}</p>
                                <p className="selectable truncate text-xs text-muted">
                                    {current ?? 'Not set'}
                                </p>
                            </div>

                            <div className="flex shrink-0 items-center gap-1">
                                <button
                                    type="button"
                                    onClick={() => void choose(game.id)}
                                    aria-label={`Choose folder for ${game.name}`}
                                    className="rounded-lg border border-border p-2 text-muted hover:text-foreground"
                                >
                                    <FiFolder className="size-4" />
                                </button>

                                {current && (
                                    <button
                                        type="button"
                                        onClick={() => void clear(game.id)}
                                        aria-label={`Clear folder for ${game.name}`}
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
                            void open({
                                directory: true,
                                multiple: false,
                                title: 'Choose a download folder',
                            }).then((dir) => {
                                if (typeof dir === 'string')
                                    void setApp({ downloadDir: dir })
                            })
                        }}
                        aria-label="Choose download folder"
                        className="shrink-0 rounded-lg border border-border p-2 text-muted hover:text-foreground"
                    >
                        <FiFolder className="size-4" />
                    </button>
                </div>
            </Section>
        </>
    )
}
