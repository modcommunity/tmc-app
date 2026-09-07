import { useCallback, useEffect, useMemo, useState } from 'react'
import { Link, useNavigate, useParams } from 'react-router-dom'
import {
    FiAlertTriangle,
    FiArrowDown,
    FiLink2,
    FiArrowUp,
    FiCheck,
    FiCloud,
    FiCloudOff,
    FiDownload,
    FiFolder,
    FiPlay,
    FiPlus,
    FiRefreshCw,
    FiTrash2,
} from 'react-icons/fi'

import { api } from '~/lib/api/client'
import { ipc } from '~/lib/ipc/commands'
import { messageOf } from '~/lib/ipc'
import { appLabel } from '~/lib/api/labels'
import { GameIcon } from '~/components/game-icon'
import { ItemThumb } from '~/components/item-thumb'
import { useAppIcons } from '~/lib/hooks/use-app-icons'
import { useAuth } from '~/lib/auth/provider'
import { useLibrary } from '~/lib/library/provider'
import type {
    DependencyReportT,
    LaunchPreviewT,
    OutdatedT,
    DeployReportT,
    OptionSpecT,
    SandboxRowT,
    SandboxSpecT,
    StrategyReportT,
} from '~/lib/ipc/schemas'
import { LaunchDialog } from '~/components/launch-dialog'
import Select from '~/components/select'
import FolderPicker from '~/components/folder-picker'
import { Toggle } from '~/components/form'
import ConfigEditor from '~/components/config-editor'
import LocalMods from '~/components/local-mods'
import { useDropTarget } from '~/components/drop-import'

/**
 * **Sandboxes** — a named set of mods, pointed at one game.
 *
 * The screen the whole mod manager exists for. It is deliberately built around
 * three questions, in this order, because they are the order somebody actually
 * hits them in:
 *
 *   1. **Where does this go?** A sandbox with no game folder can do nothing,
 *      so that is the first thing shown and the only thing shown until it is
 *      answered.
 *   2. **What is in it?** The mod list, in load order, with which of them are
 *      downloaded.
 *   3. **Is it applied?** Staging and deploying are separate steps and the
 *      screen says which one is outstanding, because "I installed it and it is
 *      not in my game" is the failure this design is trying to make impossible
 *      to reach silently.
 */

const ENVIRONMENTS = [
    { value: 'client', label: 'Client', hint: 'A game you play' },
    { value: 'server', label: 'Server', hint: 'A server you host' },
    { value: 'shared', label: 'Shared', hint: 'Mods for both sides' },
] as const

const STRATEGY_LABELS: Record<string, string> = {
    direct: 'Copy into the game folder',
    hardlink: 'Hard links',
    symlink: 'Symbolic links',
    usvfs: 'Virtual filesystem',
}

const STRATEGY_HINTS: Record<string, string> = {
    direct: 'Modifies the game. Displaced files are backed up and restored.',
    hardlink: 'Free, invisible to the game. Same drive only.',
    symlink: 'Free, crosses drives. Needs Developer Mode on Windows.',
    usvfs: 'The game folder is never touched. Windows only.',
}

export default function SandboxesRoute() {
    const { id } = useParams<{ id?: string }>()
    const navigate = useNavigate()
    const { status } = useAuth()
    const iconFor = useAppIcons()

    const [sandboxes, setSandboxes] = useState<SandboxRowT[]>([])
    const [error, setError] = useState<string | null>(null)
    const [loading, setLoading] = useState(true)
    const [creating, setCreating] = useState(false)

    const selectedId = id ? Number(id) : null

    const load = useCallback(async () => {
        try {
            setSandboxes(await ipc.sandboxList())
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

    const selected = sandboxes.find((s) => s.id === selectedId) ?? null

    if (status !== 'signedIn')
        return (
            <Empty
                title="Sign in to use sandboxes"
                body="A sandbox holds mods from your subscriptions, so the app needs to know whose subscriptions they are."
            />
        )

    if (loading) return <Empty title="Loading…" body="" />

    return (
        <div className="flex flex-col gap-4 p-4">
            <header className="flex flex-wrap items-center justify-between gap-3">
                <div>
                    <h1 className="text-lg font-bold">Sandboxes</h1>
                    <p className="text-xs text-muted">
                        Each one is its own set of mods and its own launch settings.
                        Switching between them never re-downloads anything.
                    </p>
                </div>

                <button
                    type="button"
                    onClick={() => setCreating(true)}
                    className="flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-sm text-accent-foreground"
                >
                    <FiPlus className="size-3.5" />
                    New sandbox
                </button>
            </header>

            {error && <Banner tone="danger">{error}</Banner>}

            {creating && (
                <CreateSandbox
                    onCancel={() => setCreating(false)}
                    onCreated={async (created) => {
                        setCreating(false)
                        await load()
                        void navigate(`/sandboxes/${created.id}`)
                    }}
                />
            )}

            {sandboxes.length === 0 && !creating && (
                <Empty
                    title="No sandboxes yet"
                    body="Create one to keep a set of mods together — a vanilla-ish profile, the one your friends use, a server. The game folder stays untouched until you deploy."
                />
            )}

            <div className="grid gap-3 lg:grid-cols-[minmax(0,20rem)_minmax(0,1fr)]">
                {sandboxes.length > 0 && (
                    <nav className="flex flex-col gap-1.5">
                        {sandboxes.map((sandbox) => (
                            <button
                                key={sandbox.id}
                                type="button"
                                onClick={() => navigate(`/sandboxes/${sandbox.id}`)}
                                className={`rounded-xl border p-3 text-left transition-colors ${
                                    sandbox.id === selectedId
                                        ? 'border-accent bg-surface'
                                        : 'border-border bg-surface hover:border-accent'
                                }`}
                            >
                                <div className="flex items-center gap-2">
                                    <GameIcon
                                        app={{
                                            id: sandbox.appId,
                                            name: sandbox.appName ?? '',
                                            icon: iconFor(sandbox.appId),
                                        }}
                                        size="sm"
                                    />
                                    <span className="truncate text-sm font-medium">
                                        {sandbox.name}
                                    </span>
                                    {sandbox.isDefault && (
                                        <span className="rounded bg-surface-tertiary px-1.5 py-0.5 text-[0.6rem] uppercase">
                                            Default
                                        </span>
                                    )}
                                    {sandbox.cloudSync ? (
                                        <FiCloud
                                            aria-label="Synced to your account"
                                            className="ml-auto size-3 shrink-0 text-muted"
                                        />
                                    ) : (
                                        <FiCloudOff
                                            aria-label="Kept on this device only"
                                            className="ml-auto size-3 shrink-0 text-muted"
                                        />
                                    )}
                                </div>

                                <p className="mt-0.5 truncate text-[0.7rem] text-muted">
                                    {sandbox.appName ?? 'Unknown game'}
                                    {sandbox.loader ? ` · ${sandbox.loader}` : ''}
                                    {sandbox.gameVersion
                                        ? ` ${sandbox.gameVersion}`
                                        : ''}
                                </p>

                                <p className="mt-1 text-[0.7rem] text-muted">
                                    {sandbox.mods.length} item
                                    {sandbox.mods.length === 1 ? '' : 's'}
                                    {sandbox.needsDeploy && (
                                        <span className="ml-1 text-warning">
                                            · needs deploying
                                        </span>
                                    )}
                                </p>
                            </button>
                        ))}
                    </nav>
                )}

                {selected ? (
                    <SandboxDetail
                        key={selected.id}
                        sandbox={selected}
                        onChanged={load}
                        onDeleted={async () => {
                            await load()
                            void navigate('/sandboxes')
                        }}
                    />
                ) : (
                    sandboxes.length > 0 && (
                        <div className="rounded-xl border border-dashed border-border p-8 text-center text-sm text-muted">
                            Choose a sandbox to see what is in it.
                        </div>
                    )
                )}
            </div>
        </div>
    )
}

// ------------------------------------------------------------------- Detail

function SandboxDetail({
    sandbox,
    onChanged,
    onDeleted,
}: {
    sandbox: SandboxRowT
    onChanged: () => Promise<void>
    onDeleted: () => Promise<void>
}) {
    const library = useLibrary()

    /*
     * A file dropped anywhere in the window while this sandbox is on screen
     * means "put it in THIS sandbox". The overlay lives above the router, so it
     * cannot know which screen is up; this is how the screen tells it.
     */
    useDropTarget(sandbox)

    const [spec, setSpec] = useState<SandboxSpecT | null>(null)
    const [strategies, setStrategies] = useState<StrategyReportT[]>([])
    const [deps, setDeps] = useState<DependencyReportT | null>(null)
    const [busy, setBusy] = useState<string | null>(null)
    const [error, setError] = useState<string | null>(null)
    const [report, setReport] = useState<DeployReportT | null>(null)
    const [picking, setPicking] = useState(false)

    /*
     * `null` means the dialog is closed. It opens BEFORE the plan resolves, so
     * a slow rule still gives immediate feedback that the click landed — and so
     * a rule that cannot resolve reports why in the dialog rather than in the
     * page's error banner, where it would read as a deployment failure.
     */
    const [launch, setLaunch] = useState<{
        preview: LaunchPreviewT | null
        error: string | null
        starting: boolean
    } | null>(null)

    useEffect(() => {
        let live = true

        void (async () => {
            if (sandbox.appSlug) {
                const found = await ipc
                    .sandboxSpec(sandbox.appSlug)
                    .catch(() => null)

                if (live) setSpec(found)
            }

            // Needs a game folder to probe against, so it is asked for only
            // once there is one.
            if (sandbox.targetDir) {
                const listed = await ipc
                    .sandboxStrategies(sandbox.id)
                    .catch(() => [])

                if (live) setStrategies(listed)
            }

            // Local, so it costs nothing to ask on every change to the list.
            const checked = await ipc.sandboxCheck(sandbox.id).catch(() => null)

            if (live) setDeps(checked)
        })()

        return () => {
            live = false
        }
    }, [sandbox.id, sandbox.appSlug, sandbox.targetDir, sandbox.mods])

    const startLaunch = async () => {
        setLaunch({ preview: null, error: null, starting: false })

        try {
            const preview = await ipc.sandboxLaunchPreview(sandbox.id)

            setLaunch({ preview, error: null, starting: false })
        } catch (err) {
            setLaunch({ preview: null, error: messageOf(err), starting: false })
        }
    }

    const confirmLaunch = async () => {
        setLaunch((current) => (current ? { ...current, starting: true } : current))

        try {
            await ipc.sandboxLaunch(sandbox.id)
            setLaunch(null)
        } catch (err) {
            setLaunch((current) =>
                current
                    ? { ...current, error: messageOf(err), starting: false }
                    : current
            )
        }
    }

    const run = async (label: string, fn: () => Promise<unknown>) => {
        setBusy(label)
        setError(null)

        try {
            await fn()
            await onChanged()
        } catch (err) {
            setError(messageOf(err))
        } finally {
            setBusy(null)
        }
    }

    /** Items in this sandbox's game that the user is subscribed to. */
    const candidates = useMemo(
        () =>
            library.rows.filter(
                (row) =>
                    row.appId === sandbox.appId &&
                    !row.isContainer &&
                    !sandbox.mods.some(
                        (m) => m.kind === row.kind && m.itemId === row.itemId
                    )
            ),
        [library.rows, sandbox.appId, sandbox.mods]
    )

    const staged = sandbox.mods.filter((m) => m.stagedAt).length
    const unstaged = sandbox.mods.filter((m) => m.enabled && !m.stagedAt).length

    return (
        <div className="flex min-w-0 flex-col gap-4">
            {/* ------------------------------------------------ Where it goes */}
            {!sandbox.targetDir ? (
                <section className="rounded-xl border border-warning/50 bg-surface p-4">
                    <h2 className="flex items-center gap-2 text-sm font-semibold">
                        <FiAlertTriangle className="size-4 text-warning" />
                        This sandbox has nowhere to deploy
                    </h2>
                    <p className="mt-1 text-xs text-muted">
                        Choose the game&rsquo;s folder, or set one for{' '}
                        {sandbox.appName ?? 'this game'} under Settings → Games.
                        Nothing is written until you deploy.
                    </p>

                    <button
                        type="button"
                        onClick={() => setPicking(true)}
                        className="mt-3 flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-sm text-accent-foreground"
                    >
                        <FiFolder className="size-3.5" />
                        Choose a folder
                    </button>
                </section>
            ) : (
                <section className="rounded-xl border border-border bg-surface p-3">
                    <div className="flex items-start justify-between gap-3">
                        <div className="min-w-0">
                            <p className="text-xs text-muted">Deploys into</p>
                            <p
                                className="selectable truncate text-sm"
                                title={sandbox.targetDir}
                            >
                                {sandbox.targetDir}
                            </p>
                        </div>

                        <button
                            type="button"
                            onClick={() => setPicking(true)}
                            className="shrink-0 rounded-lg border border-border px-2 py-1 text-xs hover:border-accent"
                        >
                            Change
                        </button>
                    </div>
                </section>
            )}

            {launch && (
                <LaunchDialog
                    title={`Launch “${sandbox.name}”?`}
                    preview={launch.preview}
                    error={launch.error}
                    busy={launch.starting}
                    onCancel={() => setLaunch(null)}
                    onConfirm={() => void confirmLaunch()}
                />
            )}

            {picking && (
                <FolderPicker
                    title={`Where is ${sandbox.appName ?? 'the game'} installed?`}
                    onCancel={() => setPicking(false)}
                    onPick={async (path) => {
                        setPicking(false)
                        await run('folder', () =>
                            ipc.sandboxPatch(sandbox.id, { gameDir: path })
                        )
                    }}
                />
            )}

            {error && <Banner tone="danger">{error}</Banner>}

            {/* ------------------------------------------------------ Actions */}
            <section className="flex flex-wrap items-center gap-2">
                <button
                    type="button"
                    disabled={busy !== null || unstaged === 0}
                    onClick={() =>
                        void run('stage', () => ipc.sandboxStage(sandbox.id))
                    }
                    className="flex items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-sm hover:border-accent disabled:opacity-50"
                >
                    <FiDownload className="size-3.5" />
                    {busy === 'stage'
                        ? 'Downloading…'
                        : unstaged > 0
                          ? `Download ${unstaged} item${unstaged === 1 ? '' : 's'}`
                          : 'Everything is downloaded'}
                </button>

                <button
                    type="button"
                    disabled={busy !== null || !sandbox.targetDir}
                    onClick={() =>
                        void run('deploy', async () => {
                            setReport(await ipc.sandboxDeploy(sandbox.id))
                        })
                    }
                    className="flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-sm text-accent-foreground disabled:opacity-50"
                >
                    <FiCheck className="size-3.5" />
                    {busy === 'deploy' ? 'Deploying…' : 'Deploy'}
                </button>

                {/*
                 * Offered whether or not the game has a launch rule: a game
                 * with none is common and the honest place to say so is the
                 * dialog, which explains that the sandbox is deployed either
                 * way and the game can be started normally. Hiding the button
                 * instead leaves somebody looking for a Play control that is
                 * not there and no way to find out why.
                 */}
                <button
                    type="button"
                    disabled={busy !== null || !sandbox.targetDir}
                    onClick={() => void startLaunch()}
                    className="flex items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-sm hover:border-accent disabled:opacity-50"
                >
                    <FiPlay className="size-3.5" />
                    Play
                </button>

                {sandbox.deployedFiles > 0 && (
                    <button
                        type="button"
                        disabled={busy !== null}
                        onClick={() =>
                            void run('purge', () => ipc.sandboxPurge(sandbox.id))
                        }
                        className="rounded-lg border border-border px-3 py-1.5 text-sm hover:border-accent disabled:opacity-50"
                    >
                        {busy === 'purge' ? 'Removing…' : 'Undeploy'}
                    </button>
                )}

                <button
                    type="button"
                    disabled={busy !== null}
                    onClick={() =>
                        void run('verify', async () => {
                            const checked = await ipc.sandboxVerify(sandbox.id)

                            setError(
                                checked.missing.length === 0 &&
                                    checked.changed.length === 0
                                    ? null
                                    : `${checked.missing.length} file(s) missing, ${checked.changed.length} changed since deploying.`
                            )
                        })
                    }
                    className="flex items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-sm hover:border-accent disabled:opacity-50"
                >
                    <FiRefreshCw className="size-3.5" />
                    Check files
                </button>

                <span className="ml-auto text-xs text-muted">
                    {sandbox.deployedFiles > 0
                        ? `${sandbox.deployedFiles} file${sandbox.deployedFiles === 1 ? '' : 's'} in the game folder`
                        : 'Not deployed'}
                </span>
            </section>

            {report && <DeploySummary report={report} />}

            <UpdatesBanner sandbox={sandbox} onChanged={onChanged} />

            {deps && (
                <DependencyPanel
                    sandboxId={sandbox.id}
                    report={deps}
                    onChanged={async () => {
                        await onChanged()
                        setDeps(
                            await ipc.sandboxCheck(sandbox.id).catch(() => null)
                        )
                    }}
                />
            )}

            <ConfigSection sandbox={sandbox} />

            {/*
             * Imported mods, below the dependency panel and above the load
             * order. That position is the argument: the three ways something
             * gets into a sandbox WITHOUT an account behind it belong beside
             * the list it ends up in, not on a separate screen somebody has to
             * know exists.
             */}
            <LocalMods
                sandbox={sandbox}
                onChanged={() => {
                    void onChanged()
                }}
            />

            {/* -------------------------------------------------------- Mods */}
            <section className="rounded-xl border border-border bg-surface">
                <header className="flex items-center justify-between border-b border-border px-3 py-2">
                    <h2 className="text-sm font-semibold">
                        Load order
                        <span className="ml-2 text-xs font-normal text-muted">
                            later wins a contested file
                        </span>
                    </h2>
                    <span className="text-xs text-muted">
                        {staged}/{sandbox.mods.length} downloaded
                    </span>
                </header>

                {sandbox.mods.length === 0 ? (
                    <p className="p-4 text-center text-xs text-muted">
                        Nothing in this sandbox yet. Add something you are
                        subscribed to below.
                    </p>
                ) : (
                    <ul className="divide-y divide-border">
                        {sandbox.mods.map((mod, index) => (
                            <li
                                key={mod.modKey}
                                className="flex items-center gap-2 px-3 py-2"
                            >
                                <Toggle
                                    label={`Enable ${mod.name}`}
                                    checked={mod.enabled}
                                    onChange={(next) =>
                                        void run('toggle', () =>
                                            ipc.sandboxSetModEnabled(
                                                sandbox.id,
                                                mod.modKey,
                                                next
                                            )
                                        )
                                    }
                                />

                                {/*
                                 * The item's own cover, from the LOCAL library
                                 * row with the same key — a sandbox mod and a
                                 * subscription are both `kind:itemId`, so this
                                 * is a map hit rather than a request per row.
                                 */}
                                <ItemThumb
                                    image={
                                        library.rows.find(
                                            (r) => r.id === mod.modKey
                                        )?.image
                                    }
                                    kind={mod.kind}
                                    size="sm"
                                />

                                <div className="min-w-0 flex-1">
                                    <p className="truncate text-sm">{mod.name}</p>
                                    <p className="text-[0.7rem] text-muted">
                                        {/*
                                         * An import was never downloaded, so
                                         * it must not say it was. The row is
                                         * the same shape either way — one load
                                         * order, one deploy — and the sentence
                                         * is the only place the difference
                                         * shows.
                                         */}
                                        {mod.stagedAt
                                            ? `${mod.kind === 'local' ? 'Imported' : 'Downloaded'}${mod.version ? ` · ${mod.version}` : ''}`
                                            : mod.kind === 'local'
                                              ? 'Its files are missing'
                                              : 'Not downloaded yet'}
                                        {mod.lastError && (
                                            <span className="text-danger">
                                                {' '}
                                                · {mod.lastError}
                                            </span>
                                        )}
                                    </p>
                                </div>

                                <div className="flex shrink-0 items-center gap-1">
                                    <MoveButton
                                        label="Move up"
                                        disabled={index === 0}
                                        onClick={() =>
                                            void run('order', () =>
                                                ipc.sandboxReorder(
                                                    sandbox.id,
                                                    reorder(
                                                        sandbox.mods.map(
                                                            (m) => m.modKey
                                                        ),
                                                        index,
                                                        index - 1
                                                    )
                                                )
                                            )
                                        }
                                    >
                                        <FiArrowUp className="size-3" />
                                    </MoveButton>

                                    <MoveButton
                                        label="Move down"
                                        disabled={index === sandbox.mods.length - 1}
                                        onClick={() =>
                                            void run('order', () =>
                                                ipc.sandboxReorder(
                                                    sandbox.id,
                                                    reorder(
                                                        sandbox.mods.map(
                                                            (m) => m.modKey
                                                        ),
                                                        index,
                                                        index + 1
                                                    )
                                                )
                                            )
                                        }
                                    >
                                        <FiArrowDown className="size-3" />
                                    </MoveButton>

                                    <MoveButton
                                        label={`Remove ${mod.name}`}
                                        onClick={() =>
                                            void run('remove', () =>
                                                ipc.sandboxRemoveMod(
                                                    sandbox.id,
                                                    mod.modKey
                                                )
                                            )
                                        }
                                    >
                                        <FiTrash2 className="size-3" />
                                    </MoveButton>
                                </div>
                            </li>
                        ))}
                    </ul>
                )}

                {candidates.length > 0 && (
                    <div className="border-t border-border p-3">
                        <Select
                            label="Add an item to this sandbox"
                            fullWidth
                            placeholder="Add something from your library…"
                            value=""
                            onChange={(next) => {
                                const [kind, itemId] = next.split(':')

                                if (!kind || !itemId) return

                                void run('add', () =>
                                    ipc.sandboxAddMod(
                                        sandbox.id,
                                        kind,
                                        Number(itemId)
                                    )
                                )
                            }}
                            options={candidates.map((row) => ({
                                value: `${row.kind}:${row.itemId}`,
                                label: row.name,
                                hint: row.latestVersion ?? undefined,
                            }))}
                        />
                    </div>
                )}
            </section>

            {/* ---------------------------------------------------- Settings */}
            <SandboxSettings
                sandbox={sandbox}
                spec={spec}
                strategies={strategies}
                onChanged={onChanged}
                onDeleted={onDeleted}
            />
        </div>
    )
}

/**
 * "N items have a newer release."
 *
 * Shown whether or not this sandbox updates automatically — a pinned sandbox is
 * one somebody chose to pin, and being told what they are pinned away from is
 * the point of pinning rather than unsubscribing.
 */
function UpdatesBanner({
    sandbox,
    onChanged,
}: {
    sandbox: SandboxRowT
    onChanged: () => Promise<void>
}) {
    const [outdated, setOutdated] = useState<OutdatedT[]>([])
    const [busy, setBusy] = useState(false)

    useEffect(() => {
        let live = true

        void ipc
            .sandboxUpdates()
            .then((all) => {
                if (live)
                    setOutdated(all.filter((row) => row.sandboxId === sandbox.id))
            })
            .catch(() => setOutdated([]))

        return () => {
            live = false
        }
    }, [sandbox.id, sandbox.mods])

    if (outdated.length === 0) return null

    return (
        <section className="flex flex-wrap items-center gap-3 rounded-xl border border-accent/50 bg-surface p-3">
            <div className="min-w-0 flex-1">
                <p className="text-sm">
                    {outdated.length} item
                    {outdated.length === 1 ? ' has' : 's have'} a newer release
                </p>
                <p className="truncate text-xs text-muted">
                    {outdated
                        .map(
                            (row) =>
                                `${row.name}${
                                    row.toVersion ? ` → ${row.toVersion}` : ''
                                }`
                        )
                        .join(', ')}
                </p>
            </div>

            <button
                type="button"
                disabled={busy}
                onClick={() => {
                    setBusy(true)

                    void ipc
                        .sandboxStage(sandbox.id, true)
                        .then(onChanged)
                        .finally(() => setBusy(false))
                }}
                className="shrink-0 rounded-lg bg-accent px-3 py-1.5 text-xs text-accent-foreground disabled:opacity-50"
            >
                {busy ? 'Downloading…' : 'Update now'}
            </button>
        </section>
    )
}

/**
 * What this sandbox's items say about each other.
 *
 * **Warnings, never a block.** The metadata is author-written and frequently
 * wrong — a required edge left in place after a mod absorbed its own dependency
 * is the normal state of every mod site — and a refusal with no escape hatch is
 * one people learn to ignore, which costs the accurate warnings their
 * credibility too. So the deploy button stays live and says so.
 *
 * "Add the missing ones" only offers items the account is already subscribed
 * to. The installer refuses an item the account did not ask to keep, so adding
 * the rest would produce sandbox entries that can never download; those link
 * out to their pages instead.
 */
function DependencyPanel({
    sandboxId,
    report,
    onChanged,
}: {
    sandboxId: number
    report: DependencyReportT
    onChanged: () => Promise<void>
}) {
    const library = useLibrary()
    const [busy, setBusy] = useState(false)

    const nothing =
        report.missing.length === 0 &&
        report.conflicts.length === 0 &&
        report.suggested.length === 0 &&
        report.unchecked.length === 0

    if (nothing) return null

    const subscribed = (kind: string, id: number) =>
        library.rows.some((row) => row.kind === kind && row.itemId === id)

    const addable = report.missing.filter((edge) =>
        subscribed(edge.relKind, edge.relId)
    )

    return (
        <section className="flex flex-col gap-2 rounded-xl border border-border bg-surface p-3">
            <div className="flex items-center justify-between gap-2">
                <h2 className="flex items-center gap-1.5 text-sm font-semibold">
                    <FiLink2 className="size-3.5" />
                    Dependencies
                </h2>

                <button
                    type="button"
                    disabled={busy}
                    onClick={() => {
                        setBusy(true)

                        void ipc
                            .sandboxRefreshDependencies(sandboxId)
                            .then(onChanged)
                            .finally(() => setBusy(false))
                    }}
                    className="rounded-lg border border-border px-2 py-1 text-xs hover:border-accent disabled:opacity-50"
                >
                    {busy ? 'Checking…' : 'Check again'}
                </button>
            </div>

            {report.conflicts.map((clash) => (
                <p
                    key={`${clash.aKey}-${clash.bKey}`}
                    className="flex items-start gap-1.5 text-xs text-warning"
                >
                    <FiAlertTriangle className="mt-0.5 size-3 shrink-0" />
                    <span>
                        <strong>{clash.aName}</strong> and{' '}
                        <strong>{clash.bName}</strong> are marked as incompatible.
                        {clash.note && (
                            <span className="text-muted"> {clash.note}</span>
                        )}
                    </span>
                </p>
            ))}

            {report.missing.length > 0 && (
                <div className="flex flex-col gap-1.5">
                    <p className="text-xs">
                        Missing {report.missing.length} required item
                        {report.missing.length === 1 ? '' : 's'}:
                    </p>

                    <ul className="flex flex-wrap gap-1.5">
                        {report.missing.map((edge) => (
                            <li key={`${edge.relKind}:${edge.relId}`}>
                                <Link
                                    to={`/view/${edge.relKind}/${edge.relId}`}
                                    className="flex items-center gap-1.5 rounded-lg border border-border px-2 py-1 text-xs hover:border-accent"
                                >
                                    {edge.icon && (
                                        <img
                                            src={edge.icon}
                                            alt=""
                                            loading="lazy"
                                            className="size-3.5 rounded-sm object-cover"
                                        />
                                    )}
                                    {edge.name}
                                    {!subscribed(edge.relKind, edge.relId) && (
                                        <span className="text-muted">
                                            · subscribe first
                                        </span>
                                    )}
                                </Link>
                            </li>
                        ))}
                    </ul>

                    {addable.length > 0 && (
                        <button
                            type="button"
                            disabled={busy}
                            onClick={() => {
                                setBusy(true)

                                void ipc
                                    .sandboxAddMissing(sandboxId)
                                    .then(onChanged)
                                    .finally(() => setBusy(false))
                            }}
                            className="self-start rounded-lg bg-accent px-3 py-1.5 text-xs text-accent-foreground disabled:opacity-50"
                        >
                            Add {addable.length} subscribed item
                            {addable.length === 1 ? '' : 's'}
                        </button>
                    )}
                </div>
            )}

            {report.suggested.length > 0 && (
                <p className="text-xs text-muted">
                    Also recommended:{' '}
                    {report.suggested.map((edge, index) => (
                        <span key={`${edge.relKind}:${edge.relId}`}>
                            {index > 0 && ', '}
                            <Link
                                to={`/view/${edge.relKind}/${edge.relId}`}
                                className="hover:text-accent"
                            >
                                {edge.name}
                            </Link>
                        </span>
                    ))}
                </p>
            )}

            {report.unchecked.length > 0 && (
                <p className="text-[0.7rem] text-muted">
                    Not checked yet: {report.unchecked.join(', ')}. Press check
                    again once you are online.
                </p>
            )}
        </section>
    )
}

// ----------------------------------------------------------------- Settings

function SandboxSettings({
    sandbox,
    spec,
    strategies,
    onChanged,
    onDeleted,
}: {
    sandbox: SandboxRowT
    spec: SandboxSpecT | null
    strategies: StrategyReportT[]
    onChanged: () => Promise<void>
    onDeleted: () => Promise<void>
}) {
    const [error, setError] = useState<string | null>(null)
    const [confirming, setConfirming] = useState(false)

    const patch = async (next: Parameters<typeof ipc.sandboxPatch>[1]) => {
        try {
            await ipc.sandboxPatch(sandbox.id, next)
            await onChanged()
            setError(null)
        } catch (err) {
            setError(messageOf(err))
        }
    }

    /*
     * Options are filtered by ENVIRONMENT. A dedicated server has no resolution
     * and a client has no tick rate; showing both to both is how a settings
     * screen becomes noise nobody reads.
     */
    const options = (spec?.options ?? []).filter(
        (option) =>
            option.environments.length === 0 ||
            option.environments.includes(sandbox.environment)
    )

    const available = strategies.length > 0 ? strategies : null

    return (
        <section className="flex flex-col gap-3 rounded-xl border border-border bg-surface p-3">
            <h2 className="text-sm font-semibold">Settings</h2>

            {error && <Banner tone="danger">{error}</Banner>}

            <div className="grid gap-3 sm:grid-cols-2">
                <Labelled label="Environment">
                    <Select
                        label="Environment"
                        fullWidth
                        value={sandbox.environment}
                        onChange={(next) =>
                            void patch({
                                environment: next,
                            })
                        }
                        options={ENVIRONMENTS.map((e) => ({
                            value: e.value,
                            label: e.label,
                            hint: e.hint,
                        }))}
                    />
                </Labelled>

                <Labelled label="How files reach the game">
                    <Select
                        label="Deployment method"
                        fullWidth
                        value={sandbox.strategy}
                        onChange={(next) =>
                            void patch({
                                strategy: next as SandboxRowT['strategy'],
                            })
                        }
                        options={(available ?? []).map((s) => ({
                            value: s.strategy,
                            label: STRATEGY_LABELS[s.strategy] ?? s.strategy,
                            // The probe's own reason when there is one — "your
                            // staging folder is on another drive" is far more
                            // use than a greyed-out row with no explanation.
                            hint: s.reason ?? STRATEGY_HINTS[s.strategy],
                            disabled: !s.available,
                        }))}
                    />
                </Labelled>

                <Labelled label="Game version">
                    <TextField
                        value={sandbox.gameVersion ?? ''}
                        placeholder="1.21"
                        onCommit={(value) =>
                            void patch({ gameVersion: value || null })
                        }
                    />
                </Labelled>

                <Labelled label="Loader">
                    <TextField
                        value={sandbox.loader ?? ''}
                        placeholder="fabric"
                        onCommit={(value) => void patch({ loader: value || null })}
                    />
                </Labelled>
            </div>

            {options.length > 0 && (
                <div className="flex flex-col gap-2 border-t border-border pt-3">
                    <h3 className="text-xs font-semibold uppercase tracking-wide text-muted">
                        {sandbox.appName ?? 'Game'} options
                    </h3>

                    {options.map((option) => (
                        <GameOption
                            key={option.key}
                            option={option}
                            value={sandbox.options[option.key]}
                            onChange={(value) =>
                                void patch({
                                    options: {
                                        ...sandbox.options,
                                        [option.key]: value,
                                    },
                                })
                            }
                        />
                    ))}
                </div>
            )}

            <div className="flex items-center justify-between gap-3 border-t border-border pt-3">
                <div className="min-w-0">
                    <p className="text-sm">Keep in the cloud</p>
                    <p className="text-xs text-muted">
                        {sandbox.cloudSync
                            ? 'Its name, mods and settings are on your account, so signing in elsewhere reproduces it.'
                            : 'This sandbox stays on this device. Nothing about it is sent anywhere.'}
                    </p>
                </div>

                <Toggle
                    label="Keep this sandbox in the cloud"
                    checked={sandbox.cloudSync}
                    onChange={(next) => void patch({ cloudSync: next })}
                />
            </div>

            <div className="flex items-center justify-between gap-3 border-t border-border pt-3">
                <div className="min-w-0">
                    <p className="text-sm">Keep its mods up to date</p>
                    <p className="text-xs text-muted">
                        {sandbox.autoUpdate
                            ? 'New releases are downloaded automatically, and applied if this sandbox is already deployed.'
                            : 'Pinned. You will still be told when an update exists.'}
                    </p>
                </div>

                <Toggle
                    label="Keep this sandbox's mods up to date"
                    checked={sandbox.autoUpdate}
                    onChange={(next) => void patch({ autoUpdate: next })}
                />
            </div>

            {spec?.deploy.notes.map((note) => (
                <p key={note} className="text-[0.7rem] text-muted">
                    {note}
                </p>
            ))}

            <div className="flex items-center justify-between gap-3 border-t border-border pt-3">
                {!sandbox.isDefault && (
                    <button
                        type="button"
                        onClick={() =>
                            void ipc.sandboxSetDefault(sandbox.id).then(onChanged)
                        }
                        className="rounded-lg border border-border px-3 py-1.5 text-xs hover:border-accent"
                    >
                        Make this the default
                    </button>
                )}

                <button
                    type="button"
                    onClick={() => setConfirming(true)}
                    className="ml-auto rounded-lg border border-danger/50 px-3 py-1.5 text-xs text-danger hover:bg-danger/10"
                >
                    Delete sandbox
                </button>
            </div>

            {confirming && (
                <div className="rounded-lg border border-danger/50 p-3">
                    <p className="text-sm">Delete “{sandbox.name}”?</p>
                    <p className="mt-1 text-xs text-muted">
                        Its files come out of the game folder first, and anything it
                        displaced is put back. Downloaded mods stay on disk so
                        another sandbox can use them.
                    </p>

                    <div className="mt-3 flex gap-2">
                        <button
                            type="button"
                            onClick={() =>
                                void ipc
                                    .sandboxDelete(sandbox.id, true)
                                    .then(onDeleted)
                            }
                            className="rounded-lg bg-danger px-3 py-1.5 text-xs text-white"
                        >
                            Delete
                        </button>
                        <button
                            type="button"
                            onClick={() => setConfirming(false)}
                            className="rounded-lg border border-border px-3 py-1.5 text-xs"
                        >
                            Cancel
                        </button>
                    </div>
                </div>
            )}
        </section>
    )
}

function GameOption({
    option,
    value,
    onChange,
}: {
    option: OptionSpecT
    value: unknown
    onChange: (next: unknown) => void
}) {
    if (option.type === 'bool')
        return (
            <div className="flex items-center justify-between gap-3">
                <div className="min-w-0">
                    <p className="text-sm">{option.label}</p>
                    {option.description && (
                        <p className="text-xs text-muted">{option.description}</p>
                    )}
                </div>
                <Toggle
                    label={option.label}
                    checked={value === true}
                    onChange={onChange}
                />
            </div>
        )

    if (option.type === 'select')
        return (
            <Labelled label={option.label} hint={option.description ?? undefined}>
                <Select
                    label={option.label}
                    fullWidth
                    value={typeof value === 'string' ? value : ''}
                    onChange={onChange}
                    options={option.choices}
                />
            </Labelled>
        )

    return (
        <Labelled label={option.label} hint={option.description ?? undefined}>
            <div className="flex items-center gap-2">
                <TextField
                    value={renderOption(value)}
                    numeric={option.type === 'int'}
                    min={option.min ?? undefined}
                    max={option.max ?? undefined}
                    onCommit={(next) =>
                        onChange(
                            option.type === 'int'
                                ? next === ''
                                    ? null
                                    : Number(next)
                                : next
                        )
                    }
                />
                {option.unit && (
                    <span className="text-xs text-muted">{option.unit}</span>
                )}
            </div>
        </Labelled>
    )
}

// ------------------------------------------------------------------ Create

function CreateSandbox({
    onCancel,
    onCreated,
}: {
    onCancel: () => void
    onCreated: (created: SandboxRowT) => Promise<void>
}) {
    const [apps, setApps] = useState<
        { id: number; name: string; slug: string | null; icon: string | null }[]
    >([])
    const [appId, setAppId] = useState<number | null>(null)
    const [name, setName] = useState('')
    const [preset, setPreset] = useState('')
    const [spec, setSpec] = useState<SandboxSpecT | null>(null)
    const [cloudSync, setCloudSync] = useState(true)
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<string | null>(null)

    useEffect(() => {
        void (async () => {
            try {
                const [facets, supported] = await Promise.all([
                    api.facets('mod'),
                    ipc.librarySupportedApps(),
                ])

                const slugs = new Set(supported)

                setApps(
                    facets.apps
                        .filter((a) => a.url && slugs.has(a.url.toLowerCase()))
                        .map((a) => ({
                            id: a.id,
                            name: appLabel(a),
                            slug: a.url ? a.url.toLowerCase() : null,
                            icon: a.icon,
                        }))
                )
            } catch (err) {
                setError(messageOf(err))
            }
        })()
    }, [])

    const chosen = apps.find((a) => a.id === appId) ?? null

    useEffect(() => {
        setPreset('')

        if (!chosen?.slug) {
            setSpec(null)

            return
        }

        void ipc
            .sandboxSpec(chosen.slug)
            .then(setSpec)
            .catch(() => setSpec(null))
    }, [chosen?.slug])

    const create = async () => {
        if (!chosen || name.trim().length === 0) return

        setBusy(true)
        setError(null)

        try {
            const created = await ipc.sandboxCreate(
                {
                    appId: chosen.id,
                    appSlug: chosen.slug,
                    appName: chosen.name,
                    name: name.trim(),
                    cloudSync,
                },
                preset || undefined
            )

            await onCreated(created)
        } catch (err) {
            setError(messageOf(err))
        } finally {
            setBusy(false)
        }
    }

    return (
        <section className="flex flex-col gap-3 rounded-xl border border-border bg-surface p-4">
            <h2 className="text-sm font-semibold">New sandbox</h2>

            {error && <Banner tone="danger">{error}</Banner>}

            <Labelled label="Game">
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
                                loading="lazy"
                                className="size-4 rounded-sm object-cover"
                            />
                        ) : undefined,
                    }))}
                />
            </Labelled>

            {(spec?.presets.length ?? 0) > 0 && (
                <Labelled
                    label="Start from"
                    hint="A preset fills in the loader, the version and the game's own settings. You can change any of them afterwards."
                >
                    <Select
                        label="Preset"
                        fullWidth
                        placeholder="Empty sandbox"
                        value={preset}
                        onChange={setPreset}
                        options={[
                            { value: '', label: 'Empty sandbox' },
                            ...(spec?.presets ?? []).map((p) => ({
                                value: p.id,
                                label: p.label,
                                hint: p.description ?? undefined,
                            })),
                        ]}
                    />
                </Labelled>
            )}

            <Labelled label="Name">
                <input
                    value={name}
                    maxLength={64}
                    placeholder="Kitchen sink"
                    onChange={(e) => setName(e.target.value)}
                    className="w-full rounded-lg border border-border bg-background px-2 py-1.5 text-sm"
                />
            </Labelled>

            <div className="flex items-center justify-between gap-3">
                <div className="min-w-0">
                    <p className="text-sm">Keep in the cloud</p>
                    <p className="text-xs text-muted">
                        Reproduces this sandbox when you sign in elsewhere. Turn it
                        off to keep it on this device only.
                    </p>
                </div>
                <Toggle
                    label="Keep this sandbox in the cloud"
                    checked={cloudSync}
                    onChange={setCloudSync}
                />
            </div>

            <div className="flex gap-2">
                <button
                    type="button"
                    disabled={busy || !appId || name.trim().length === 0}
                    onClick={() => void create()}
                    className="rounded-lg bg-accent px-3 py-1.5 text-sm text-accent-foreground disabled:opacity-50"
                >
                    {busy ? 'Creating…' : 'Create'}
                </button>
                <button
                    type="button"
                    onClick={onCancel}
                    className="rounded-lg border border-border px-3 py-1.5 text-sm"
                >
                    Cancel
                </button>
            </div>
        </section>
    )
}

// ------------------------------------------------------------------- Bits

/**
 * The game-settings editor, when this game describes where its settings live.
 *
 * Absent rather than empty for a game that does not. `config_available` is a
 * separate command precisely so this can decide not to render — a game with no
 * `config.json` is the common case, and a section that opens onto "this game
 * does not describe its settings" is a section that trains people to skip it.
 *
 * Collapsed by default. Somebody on this screen is usually here to reorder mods
 * or deploy; the editor is what they come back for, and an open text pane above
 * the load order pushes the thing they wanted off the screen.
 */
function ConfigSection({ sandbox }: { sandbox: SandboxRowT }) {
    const [available, setAvailable] = useState<boolean | null>(null)
    const [open, setOpen] = useState(false)

    useEffect(() => {
        let live = true

        setAvailable(null)

        void ipc
            .configAvailable(sandbox.id)
            .then((ok) => {
                if (live) setAvailable(ok)
            })
            .catch(() => {
                if (live) setAvailable(false)
            })

        return () => {
            live = false
        }
    }, [sandbox.id])

    if (available !== true) return null

    return (
        <section className="rounded-xl border border-border bg-surface">
            <button
                type="button"
                onClick={() => setOpen((v) => !v)}
                aria-expanded={open}
                className="flex w-full items-center justify-between border-b border-border px-3 py-2 text-left"
            >
                <h2 className="text-sm font-semibold">
                    Game settings
                    <span className="ml-2 text-xs font-normal text-muted">
                        the files this game&rsquo;s mods read
                    </span>
                </h2>
                <span className="text-xs text-muted">{open ? 'Hide' : 'Edit'}</span>
            </button>

            {open && (
                <div className="flex p-3">
                    <ConfigEditor sandboxId={sandbox.id} />
                </div>
            )}
        </section>
    )
}

function DeploySummary({ report }: { report: DeployReportT }) {
    return (
        <section className="rounded-xl border border-border bg-surface p-3 text-xs">
            <p className="text-sm font-medium">
                Deployed with {STRATEGY_LABELS[report.used] ?? report.used}
            </p>

            {report.fellBack && (
                <p className="mt-1 text-warning">{report.fellBack}</p>
            )}

            <p className="mt-1 text-muted">
                {report.placed} placed · {report.reused} already correct ·{' '}
                {report.removed} removed
                {report.backedUp > 0 && ` · ${report.backedUp} backed up`}
            </p>

            {report.conflicts.length > 0 && (
                <details className="mt-2">
                    <summary className="cursor-pointer text-muted">
                        {report.conflicts.length} file
                        {report.conflicts.length === 1 ? '' : 's'} provided by more
                        than one mod
                    </summary>
                    <ul className="mt-1 flex flex-col gap-0.5">
                        {report.conflicts.slice(0, 40).map((conflict) => (
                            <li key={conflict.path} className="selectable">
                                <span className="text-muted">{conflict.path}</span>{' '}
                                → {conflict.winner}
                                <span className="text-muted">
                                    {' '}
                                    (over {conflict.losers.join(', ')})
                                </span>
                            </li>
                        ))}
                    </ul>
                </details>
            )}

            {report.empty.length > 0 && (
                <p className="mt-2 text-warning">
                    Contributed no files: {report.empty.join(', ')}
                </p>
            )}

            {report.warnings.map((warning) => (
                <p key={warning} className="mt-1 text-warning">
                    {warning}
                </p>
            ))}

            {report.errors.map((err) => (
                <p key={err} className="mt-1 text-danger">
                    {err}
                </p>
            ))}
        </section>
    )
}

function Labelled({
    label,
    hint,
    children,
}: {
    label: string
    hint?: string
    children: React.ReactNode
}) {
    return (
        <div className="min-w-0">
            <span className="mb-1 block text-xs text-muted">{label}</span>
            {children}
            {hint && <p className="mt-1 text-[0.7rem] text-muted">{hint}</p>}
        </div>
    )
}

/**
 * A text input that commits on blur and on Enter, not on every keystroke.
 *
 * Each commit is an IPC round trip and a database write; committing per
 * character would be one per letter typed into a version field.
 */
function TextField({
    value,
    onCommit,
    placeholder,
    numeric,
    min,
    max,
}: {
    value: string
    onCommit: (next: string) => void
    placeholder?: string
    numeric?: boolean
    min?: number
    max?: number
}) {
    const [draft, setDraft] = useState(value)

    useEffect(() => setDraft(value), [value])

    return (
        <input
            value={draft}
            type={numeric ? 'number' : 'text'}
            min={min}
            max={max}
            placeholder={placeholder}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={() => draft !== value && onCommit(draft)}
            onKeyDown={(e) => {
                if (e.key === 'Enter') e.currentTarget.blur()
                if (e.key === 'Escape') setDraft(value)
            }}
            className="w-full rounded-lg border border-border bg-background px-2 py-1.5 text-sm"
        />
    )
}

function MoveButton({
    label,
    disabled,
    onClick,
    children,
}: {
    label: string
    disabled?: boolean
    onClick: () => void
    children: React.ReactNode
}) {
    return (
        <button
            type="button"
            aria-label={label}
            title={label}
            disabled={disabled}
            onClick={onClick}
            className="rounded-lg border border-border p-1.5 text-muted hover:border-accent hover:text-foreground disabled:opacity-30"
        >
            {children}
        </button>
    )
}

function Banner({
    tone,
    children,
}: {
    tone: 'danger' | 'warning'
    children: React.ReactNode
}) {
    return (
        <p
            className={`selectable rounded-lg border px-3 py-2 text-xs ${
                tone === 'danger'
                    ? 'border-danger/50 text-danger'
                    : 'border-warning/50 text-warning'
            }`}
        >
            {children}
        </p>
    )
}

function Empty({ title, body }: { title: string; body: string }) {
    return (
        <div className="p-8 text-center">
            <p className="text-sm font-medium">{title}</p>
            {body && (
                <p className="mx-auto mt-1 max-w-md text-xs text-muted">{body}</p>
            )}
        </div>
    )
}

/**
 * One option value as an input's text.
 *
 * An option's value is `unknown` because a game declares its own schema, so
 * `String(value)` on an object would put `[object Object]` in the field. The
 * clamp in Rust drops anything that is not a scalar, so this only ever has to
 * cope with one arriving from an older row.
 */
function renderOption(value: unknown): string {
    if (value == null) return ''

    if (
        typeof value === 'string' ||
        typeof value === 'number' ||
        typeof value === 'boolean'
    )
        return String(value)

    return ''
}

/** Move `from` to `to`, returning the new order. */
function reorder(keys: string[], from: number, to: number): string[] {
    const next = [...keys]
    const [moved] = next.splice(from, 1)

    if (moved !== undefined) next.splice(to, 0, moved)

    return next
}
