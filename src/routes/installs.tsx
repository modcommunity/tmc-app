import { useCallback, useEffect, useMemo, useState } from 'react'
import { Link } from 'react-router-dom'
import {
    FiAlertTriangle,
    FiFolder,
    FiPlay,
    FiPlus,
    FiRefreshCw,
    FiStar,
    FiTrash2,
    FiX,
} from 'react-icons/fi'

import { api } from '~/lib/api/client'
import { ipc } from '~/lib/ipc/commands'
import { messageOf } from '~/lib/ipc'
import { useAuth } from '~/lib/auth/provider'
import { useLibrary } from '~/lib/library/provider'
import { InstallSchema, type InstallT } from '~/lib/api/contract'
import { appLabel } from '~/lib/api/labels'
import { GameIcon } from '~/components/game-icon'
import Select from '~/components/select'
import { LaunchDialog } from '~/components/launch-dialog'
import type { LaunchPreviewT } from '~/lib/ipc/schemas'
import { useFolderPicker } from '~/components/folder-picker'

/**
 * **Installs** — the sandboxes a game is played in.
 *
 * An install is the app's answer to "I want two different sets of mods for the
 * same game": a named profile holding its own item list and its own launch
 * settings. The DEFINITION lives on the account, so it follows the user to
 * another machine; the DIRECTORY it materialises into is this machine's answer
 * and lives only here.
 *
 * The launch button never runs anything the user has not seen. `launchPreview`
 * resolves the exact program, arguments and working directory first and the
 * dialog shows them — which is the whole reason a declarative launcher is
 * acceptable at all.
 */

function InstallCard({
    install,
    localDir,
    onChanged,
    onLaunch,
}: {
    install: InstallT
    localDir: string | null
    onChanged: () => void
    onLaunch: (install: InstallT) => void
}) {
    const [busy, setBusy] = useState(false)
    const [launchable, setLaunchable] = useState(false)

    const { pick, element: picker } = useFolderPicker()

    useEffect(() => {
        let live = true

        const slug = install.app.slug

        if (!slug) return

        void ipc
            .launchAvailable(slug)
            .then((ok) => {
                if (live) setLaunchable(ok)
            })
            .catch(() => undefined)

        return () => {
            live = false
        }
    }, [install.app.slug])

    const chooseDir = async () => {
        const dir = await pick({
            title: `Choose the folder for “${install.name}”`,
            hint: 'Files for this install are written here and nowhere else.',
            initial: localDir,
        })

        if (dir === null) return

        setBusy(true)

        try {
            await ipc.librarySetInstallDir(install.id, dir)
            onChanged()
        } finally {
            setBusy(false)
        }
    }

    const clearDir = async () => {
        setBusy(true)

        try {
            await ipc.librarySetInstallDir(install.id, null)
            onChanged()
        } finally {
            setBusy(false)
        }
    }

    const makeDefault = async () => {
        setBusy(true)

        try {
            await api.installUpdate({ id: install.id, isDefault: true })
            onChanged()
        } finally {
            setBusy(false)
        }
    }

    const remove = async () => {
        if (
            !window.confirm(
                `Delete “${install.name}”? The definition is removed from your account. Files already on this machine are left alone.`
            )
        )
            return

        setBusy(true)

        try {
            await api.installDelete(install.id)
            onChanged()
        } finally {
            setBusy(false)
        }
    }

    const missing = install.items.filter((i) => !i.subscribed).length

    return (
        <div className="rounded-xl border border-border p-3">
            <div className="flex flex-wrap items-start justify-between gap-2">
                <GameIcon app={install.app} size="lg" className="mt-0.5" />

                <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                        <h2 className="truncate text-sm font-semibold">
                            {install.name}
                        </h2>

                        {install.isDefault ? (
                            <span className="rounded border border-accent px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-accent">
                                Default
                            </span>
                        ) : null}

                        <span className="text-[11px] text-muted">
                            {appLabel(install.app)}
                        </span>
                    </div>

                    <p className="mt-0.5 text-xs text-muted">
                        {[
                            `${install.items.length} item${install.items.length === 1 ? '' : 's'}`,
                            install.gameVersion,
                            install.loader,
                        ]
                            .filter(Boolean)
                            .join(' · ')}
                    </p>
                </div>

                <div className="flex shrink-0 items-center gap-1">
                    {launchable ? (
                        <button
                            type="button"
                            disabled={busy}
                            onClick={() => onLaunch(install)}
                            className="flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-xs font-semibold text-accent-foreground disabled:opacity-50"
                        >
                            <FiPlay className="h-3.5 w-3.5" />
                            Play
                        </button>
                    ) : null}

                    {!install.isDefault ? (
                        <button
                            type="button"
                            disabled={busy}
                            onClick={() => void makeDefault()}
                            aria-label="Make default"
                            title="Make this the default install for this game"
                            className="rounded-lg border border-border p-2 text-muted hover:text-foreground disabled:opacity-50"
                        >
                            <FiStar className="h-4 w-4" />
                        </button>
                    ) : null}

                    <button
                        type="button"
                        disabled={busy}
                        onClick={() => void remove()}
                        aria-label="Delete install"
                        className="rounded-lg border border-border p-2 text-muted hover:text-danger disabled:opacity-50"
                    >
                        <FiTrash2 className="h-4 w-4" />
                    </button>
                </div>
            </div>

            <div className="mt-3 flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
                <div className="min-w-0">
                    <p className="text-xs">Folder on this device</p>
                    <p className="selectable truncate text-[11px] text-muted">
                        {localDir ?? 'Using the game folder from Settings → Games'}
                    </p>
                </div>

                <div className="flex shrink-0 items-center gap-1">
                    <button
                        type="button"
                        disabled={busy}
                        onClick={() => void chooseDir()}
                        aria-label="Choose folder"
                        className="rounded-lg border border-border p-2 text-muted hover:text-foreground disabled:opacity-50"
                    >
                        <FiFolder className="h-4 w-4" />
                    </button>

                    {localDir ? (
                        <button
                            type="button"
                            disabled={busy}
                            onClick={() => void clearDir()}
                            aria-label="Clear folder"
                            className="rounded-lg border border-border p-2 text-muted hover:text-foreground disabled:opacity-50"
                        >
                            <FiX className="h-4 w-4" />
                        </button>
                    ) : null}
                </div>
            </div>

            {install.items.length > 0 ? (
                <ul className="mt-2 flex flex-col gap-1">
                    {install.items.map((item) => (
                        <li
                            key={item.id}
                            className="flex items-center justify-between gap-2 text-xs"
                        >
                            <Link
                                to={`/view/${item.kind}/${item.itemId}`}
                                className={`truncate ${item.enabled ? '' : 'text-muted line-through'}`}
                            >
                                {item.name}
                            </Link>

                            {!item.subscribed ? (
                                <span
                                    title="You are not subscribed to this, so the app will skip it"
                                    className="shrink-0 text-warning"
                                >
                                    not subscribed
                                </span>
                            ) : null}
                        </li>
                    ))}
                </ul>
            ) : null}

            {missing > 0 ? (
                <p className="mt-2 flex items-start gap-1.5 text-[11px] text-warning">
                    <FiAlertTriangle className="mt-0.5 h-3 w-3 shrink-0" />
                    {missing} item{missing === 1 ? '' : 's'} in this install are not
                    subscribed, so nothing will be installed for them.
                </p>
            ) : null}

            {picker}
        </div>
    )
}

/**
 * Creating an install, in the app.
 *
 * The game list comes from the facets endpoint and is narrowed to the games
 * this BUILD can actually manage (`librarySupportedApps`, which reads
 * `plugins/app/<slug>/`). Offering all eighty would let a user create an
 * install for a game nothing will ever materialise — a row that looks like it
 * works and silently does not, which is the failure this whole screen is trying
 * to make impossible.
 */
function NewInstallDialog({
    onClose,
    onCreated,
}: {
    onClose: () => void
    onCreated: () => void
}) {
    const [name, setName] = useState('')
    const [appId, setAppId] = useState<number | null>(null)
    const [gameVersion, setGameVersion] = useState('')
    const [loader, setLoader] = useState('')
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const [apps, setApps] = useState<
        { id: number; name: string; icon: string | null }[]
    >([])

    useEffect(() => {
        let live = true

        void (async () => {
            try {
                const [facets, supported] = await Promise.all([
                    api.facets('mod'),
                    ipc.librarySupportedApps(),
                ])

                if (!live) return

                const slugs = new Set(supported)

                setApps(
                    facets.apps
                        .filter((a) => a.url && slugs.has(a.url.toLowerCase()))
                        .map((a) => ({
                            id: a.id,
                            name: appLabel(a),
                            icon: a.icon,
                        }))
                )
            } catch (err) {
                if (live) setError(messageOf(err))
            }
        })()

        return () => {
            live = false
        }
    }, [])

    const create = async () => {
        if (!appId || name.trim().length === 0) return

        setBusy(true)
        setError(null)

        try {
            await api.installCreate({
                appId,
                name: name.trim(),
                gameVersion: gameVersion.trim() || undefined,
                loader: loader.trim() || undefined,
            })

            onCreated()
        } catch (err) {
            setError(messageOf(err))
        } finally {
            setBusy(false)
        }
    }

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
            <div className="w-full max-w-md rounded-xl border border-border bg-surface p-4">
                <h2 className="text-sm font-semibold">New install</h2>

                {error ? (
                    <p className="mt-3 rounded-lg border border-danger/40 bg-danger/10 p-2 text-xs text-danger">
                        {error}
                    </p>
                ) : null}

                {apps.length === 0 && !error ? (
                    <p className="mt-3 text-xs text-muted">
                        No game on this device has install rules yet. Add one under
                        Settings → Plugins.
                    </p>
                ) : null}

                <div className="mt-3 text-xs">
                    <span className="mb-1 block">Game</span>
                    <Select
                        label="Game"
                        fullWidth
                        placeholder="Choose a game…"
                        value={appId ? String(appId) : ''}
                        onChange={(next) => setAppId(next ? Number(next) : null)}
                        options={apps.map((a) => ({
                            value: String(a.id),
                            label: a.name,
                            icon: a.icon ? (
                                <img
                                    src={a.icon}
                                    alt=""
                                    className="size-4 rounded-sm object-cover"
                                />
                            ) : undefined,
                        }))}
                    />
                </div>

                <label className="mt-3 block text-xs">
                    Name
                    <input
                        value={name}
                        maxLength={64}
                        onChange={(e) => setName(e.target.value)}
                        placeholder="Kitchen sink 1.20"
                        className="mt-1 w-full rounded-lg border border-border bg-surface-2 px-2 py-1.5 text-xs"
                    />
                </label>

                <div className="mt-3 grid grid-cols-2 gap-2">
                    <label className="block text-xs">
                        Game version
                        <input
                            value={gameVersion}
                            maxLength={64}
                            onChange={(e) => setGameVersion(e.target.value)}
                            placeholder="1.20.1"
                            className="mt-1 w-full rounded-lg border border-border bg-surface-2 px-2 py-1.5 text-xs"
                        />
                    </label>

                    <label className="block text-xs">
                        Mod loader
                        <input
                            value={loader}
                            maxLength={64}
                            onChange={(e) => setLoader(e.target.value)}
                            placeholder="fabric"
                            className="mt-1 w-full rounded-lg border border-border bg-surface-2 px-2 py-1.5 text-xs"
                        />
                    </label>
                </div>

                <p className="mt-2 text-[11px] text-muted">
                    The version and loader are hints a launch rule may use. You can
                    pick this install&rsquo;s folder on this device once it exists.
                </p>

                <div className="mt-4 flex justify-end gap-2">
                    <button
                        type="button"
                        onClick={onClose}
                        className="rounded-lg border border-border px-3 py-1.5 text-xs"
                    >
                        Cancel
                    </button>

                    <button
                        type="button"
                        disabled={busy || !appId || name.trim().length === 0}
                        onClick={() => void create()}
                        className="rounded-lg bg-accent px-3 py-1.5 text-xs font-semibold text-accent-foreground disabled:opacity-50"
                    >
                        Create
                    </button>
                </div>
            </div>
        </div>
    )
}

export default function InstallsRoute() {
    const { status } = useAuth()
    const { sync } = useLibrary()

    const [installs, setInstalls] = useState<InstallT[]>([])
    const [dirs, setDirs] = useState<Record<number, string | null>>({})
    const [loading, setLoading] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const [preview, setPreview] = useState<{
        install: InstallT
        plan: LaunchPreviewT | null
        error: string | null
    } | null>(null)
    const [creating, setCreating] = useState(false)

    const load = useCallback(async () => {
        setLoading(true)

        try {
            /*
             * From the LOCAL mirror, not the network. The list has to render
             * offline — an install is how a game is launched, and "no internet"
             * is not a reason to be unable to play. `librarySync` is what
             * refreshes the mirror.
             */
            const raw = await ipc.libraryInstalls()

            const parsed = raw
                .map((row) => InstallSchema.safeParse(row))
                .filter((r) => r.success)
                .map((r) => r.data)

            setInstalls(parsed)

            const pairs = await ipc.libraryInstallDirs()
            const found: Record<number, string | null> = {}

            for (const [id, dir] of pairs) found[id] = dir

            setDirs(found)
            setError(null)
        } catch (err) {
            setError(messageOf(err))
        } finally {
            setLoading(false)
        }
    }, [])

    useEffect(() => {
        void load()
    }, [load])

    const refresh = useCallback(async () => {
        await sync(true)
        await load()
    }, [sync, load])

    const onLaunch = useCallback(async (install: InstallT) => {
        setPreview({ install, plan: null, error: null })

        try {
            const plan = await ipc.launchPreview(install.id)

            setPreview({ install, plan, error: null })
        } catch (err) {
            setPreview({ install, plan: null, error: messageOf(err) })
        }
    }, [])

    const confirmLaunch = useCallback(async () => {
        if (!preview) return

        try {
            await ipc.launchInstall(preview.install.id)
            setPreview(null)
        } catch (err) {
            setPreview({ ...preview, error: messageOf(err) })
        }
    }, [preview])

    const byApp = useMemo(() => {
        const map = new Map<string, InstallT[]>()

        for (const install of installs) {
            const key = appLabel(install.app)

            map.set(key, [...(map.get(key) ?? []), install])
        }

        return [...map.entries()].sort(([a], [b]) => a.localeCompare(b))
    }, [installs])

    if (status !== 'signedIn') {
        return (
            <div className="p-6 text-center">
                <p className="text-sm text-muted">
                    Sign in to manage your installs.
                </p>
            </div>
        )
    }

    return (
        <div className="flex flex-col gap-3 p-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
                <div>
                    <h1 className="text-base font-semibold">Installs</h1>
                    <p className="text-xs text-muted">
                        Sandboxes for a game — each with its own mods and launch
                        settings.
                    </p>
                </div>

                <div className="flex items-center gap-1">
                    <button
                        type="button"
                        disabled={loading}
                        onClick={() => void refresh()}
                        className="flex items-center gap-2 rounded-lg border border-border px-3 py-1.5 text-xs disabled:opacity-50"
                    >
                        <FiRefreshCw
                            className={
                                loading ? 'h-3.5 w-3.5 animate-spin' : 'h-3.5 w-3.5'
                            }
                        />
                        Refresh
                    </button>

                    <button
                        type="button"
                        disabled={loading}
                        onClick={() => setCreating(true)}
                        className="flex items-center gap-2 rounded-lg border border-border px-3 py-1.5 text-xs disabled:opacity-50"
                    >
                        <FiPlus className="h-3.5 w-3.5" />
                        New install
                    </button>
                </div>
            </div>

            {error ? (
                <p className="flex items-start gap-2 rounded-lg border border-danger/40 bg-danger/10 p-2 text-xs text-danger">
                    <FiAlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                    {error}
                </p>
            ) : null}

            {installs.length === 0 && !loading ? (
                <div className="rounded-xl border border-border p-8 text-center">
                    <p className="text-sm">No installs yet.</p>
                    <p className="mx-auto mt-1 max-w-sm text-xs text-muted">
                        An install is a named set of mods for one game, with its own
                        launch settings. Create one on the website and it will
                        appear here on every device you are signed in on.
                    </p>
                </div>
            ) : null}

            {byApp.map(([app, list]) => (
                <section key={app} className="flex flex-col gap-2">
                    <h2 className="text-xs font-semibold uppercase tracking-wide text-muted">
                        {app}
                    </h2>

                    {list.map((install) => (
                        <InstallCard
                            key={install.id}
                            install={install}
                            localDir={dirs[install.id] ?? null}
                            onChanged={() => void load()}
                            onLaunch={(i) => void onLaunch(i)}
                        />
                    ))}
                </section>
            ))}

            {creating ? (
                <NewInstallDialog
                    onClose={() => setCreating(false)}
                    onCreated={() => {
                        setCreating(false)
                        void load()
                    }}
                />
            ) : null}

            {preview ? (
                <LaunchDialog
                    title={`Launch “${preview.install.name}”?`}
                    preview={preview.plan}
                    error={preview.error}
                    onCancel={() => setPreview(null)}
                    onConfirm={() => void confirmLaunch()}
                />
            ) : null}
        </div>
    )
}
