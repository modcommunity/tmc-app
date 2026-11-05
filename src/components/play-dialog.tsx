import { useCallback, useEffect, useMemo, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import {
    FiAlertTriangle,
    FiCpu,
    FiDownload,
    FiExternalLink,
    FiGlobe,
    FiHardDrive,
    FiLayers,
    FiPlay,
    FiServer,
} from 'react-icons/fi'

import { api } from '~/lib/api/client'
import { useSettings } from '~/lib/settings/provider'
import { ipc } from '~/lib/ipc/commands'
import { messageOf } from '~/lib/ipc'
import { GameIcon } from '~/components/game-icon'
import Select from '~/components/select'
import { LaunchDialog } from '~/components/launch-dialog'
import { LiveLatency } from '~/components/server-live'
import { requestFor, useLiveServer } from '~/lib/hooks/use-live-query'
import type {
    AppSummaryT,
    ContentSummaryT,
    PlayOptionT,
    PlayOptionValuesT,
} from '~/lib/api/contract'
import type { InstalledGameT, LaunchPreviewT, SandboxRowT } from '~/lib/ipc/schemas'

/**
 * **The launcher**, and the reason the app's player beats a modal in a page.
 *
 * The website can offer one button, because a browser has exactly one way to
 * start a game. This device has four, and which of them is even possible is a
 * fact about THIS machine that no server can answer:
 *
 *   * **The TMC build** — a game this app downloaded and unpacked itself, run
 *     as a real process. It is offered first when it exists, because it is the
 *     only mode where the app knows the exact build on disk and can therefore
 *     say that it will start.
 *   * **In a window** — the app's own web loader, in a webview with no IPC.
 *   * **The installed copy** — through a sandbox, with its mods deployed, its
 *     load order applied and its launch options resolved. Nothing on the web
 *     can do this.
 *   * **Connect** — hand the server's own `connect` link to the game client
 *     that is already installed.
 *
 * So the dialog's job is to put those side by side with the facts that decide
 * between them, and the facts are the app's own: which sandboxes exist for this
 * game, whether the game folder is set, and — for a server — the latency
 * measured from this device rather than from a scanner in another hemisphere.
 *
 * WHAT IT DOES NOT DECIDE
 * ----------------------
 * Whether a launch is ALLOWED. `directPlay`, the play centre's own switch and
 * every other server-side rule are enforced in `/play/launch`, and this dialog
 * only decides what to draw. A button hidden here and a launch refused there
 * agree because the second is the authority, not because they were written to
 * match.
 */

/** The ways a game can be started, in the order they are offered. */
type Mode = 'native' | 'install' | 'web' | 'sandbox' | 'connect'

export type PlayTargetT = {
    appId: number
    /**
     * The full catalogue row, when the caller already has one.
     *
     * The Apps tab does; a server page does not — it has an `AppRef` with a
     * name and an icon and nothing about how the game can be launched. So the
     * dialog looks the rest up by id rather than making every caller carry it,
     * and `fallback` is what it draws while that is in flight or if it fails.
     */
    app?: AppSummaryT
    fallback?: { name: string; icon: string | null; slug: string | null }
    /** Set when launching into a specific server. */
    server?: ContentSummaryT | null
    /** A game pack named by the launch link; the site's own answer wins. */
    solo?: { contentId: string; version?: string }
}

/** Says which platforms a server's owner has closed, so a missing button is explained. */
function ClosedNote({ web, desktop }: { web: boolean; desktop: boolean }) {
    const what =
        web && desktop
            ? 'web and desktop players'
            : web
              ? 'players in a browser window'
              : 'desktop players'

    return (
        <p className="flex items-start gap-2 px-1 text-[11px] text-muted">
            <FiAlertTriangle className="mt-0.5 size-3 shrink-0 text-warning" />
            <span>This server&rsquo;s owner has closed it to {what}.</span>
        </p>
    )
}

function OptionField({
    option,
    value,
    onChange,
}: {
    option: PlayOptionT
    value: PlayOptionValuesT[string] | undefined
    onChange: (next: PlayOptionValuesT[string]) => void
}) {
    const label = (
        <span className="text-[11px] font-medium uppercase tracking-wide text-muted">
            {option.label}
        </span>
    )

    const help = option.help ? (
        <span className="text-[11px] text-muted">{option.help}</span>
    ) : null

    if (option.kind === 'bool')
        return (
            <label className="flex items-start gap-2 text-xs">
                <input
                    type="checkbox"
                    className="mt-0.5"
                    checked={value === true}
                    onChange={(e) => onChange(e.target.checked)}
                />
                <span className="flex flex-col gap-0.5">
                    <span>{option.label}</span>
                    {help}
                </span>
            </label>
        )

    if (option.kind === 'select')
        return (
            <label className="flex flex-col gap-1">
                {label}
                <Select
                    label={option.label}
                    value={String(value ?? option.def)}
                    options={option.choices.map((c) => ({
                        value: c.value,
                        label: c.label,
                    }))}
                    onChange={onChange}
                    fullWidth
                />
                {help}
            </label>
        )

    if (option.kind === 'int')
        return (
            <label className="flex flex-col gap-1">
                {label}
                <input
                    type="number"
                    min={option.min}
                    max={option.max}
                    value={Number(value ?? option.def)}
                    onChange={(e) => onChange(Number(e.target.value))}
                    className="rounded-lg border border-border bg-surface-2 px-2 py-1 text-xs"
                />
                {help}
            </label>
        )

    return (
        <label className="flex flex-col gap-1">
            {label}
            <input
                type="text"
                maxLength={option.maxLen ?? 200}
                value={String(value ?? option.def)}
                onChange={(e) => onChange(e.target.value)}
                className="rounded-lg border border-border bg-surface-2 px-2 py-1 text-xs"
            />
            {help}
        </label>
    )
}

/** Every option at its declared default — what the form opens on. */
function defaultsFor(options: PlayOptionT[]): PlayOptionValuesT {
    const out: PlayOptionValuesT = {}

    for (const option of options) {
        if (option.kind === 'select') {
            /*
             * A `def` naming a choice that no longer exists falls back to the
             * first, matching `PlayOptionDefaults` on the server. An
             * unselectable form is a worse answer than a different game mode,
             * and the server would coerce it to this anyway.
             */
            out[option.key] = option.choices.some((c) => c.value === option.def)
                ? option.def
                : (option.choices[0]?.value ?? '')

            continue
        }

        if (option.kind === 'int') {
            out[option.key] = Math.min(Math.max(option.def, option.min), option.max)

            continue
        }

        out[option.key] = option.def
    }

    return out
}

export default function PlayDialog({
    target,
    onClose,
}: {
    target: PlayTargetT
    onClose: () => void
}) {
    const { server } = target
    const { user } = useSettings()

    /*
     * Skipped entirely when the caller already handed over a row. `enabled`
     * rather than a conditional hook: the Apps tab passes one and a server page
     * does not, and a hook that sometimes runs is not a hook.
     */
    const looked = useQuery({
        queryKey: ['apps', { ids: [target.appId] }],
        queryFn: () => api.apps({ ids: [target.appId], limit: 1 }),
        enabled: !target.app,
        staleTime: 5 * 60 * 1000,
    })

    /*
     * The catalogue row if there is one, and a stub otherwise.
     *
     * The stub carries `play: null`, which is the honest value: without the
     * server's answer nothing is known about web launches, so none is offered.
     * The SANDBOX launch still works — it needs nothing from the catalogue —
     * which is what keeps this dialog useful offline.
     */
    const app: AppSummaryT = useMemo(
        () =>
            target.app ??
            looked.data?.apps[0] ?? {
                id: target.appId,
                name: target.fallback?.name ?? `Game ${target.appId}`,
                slug: target.fallback?.slug ?? null,
                description: null,
                type: 'GAME',
                images: {
                    card: null,
                    banner: null,
                    icon: target.fallback?.icon ?? null,
                },
                isOfficial: false,
                integrations: [],
                hasServers: true,
                engine: null,
                counts: { mods: 0, assets: 0, servers: 0, players: 0 },
                play: null,
                // Same honesty as `play`: without the catalogue row nothing is
                // known about installable builds, so none is offered.
                install: null,
                webUrl: '',
            },
        [target.app, target.appId, target.fallback, looked.data]
    )

    /*
     * The TMC build installed for this game, or null.
     *
     * `undefined` while it is being looked up, so the mode chooser below can
     * wait for the answer rather than defaulting to a worse mode and then
     * moving under somebody's cursor.
     */
    const [native, setNative] = useState<InstalledGameT | null | undefined>(
        undefined
    )
    /** A TMC pack game whose engine's build this machine could install. */
    const [engineInstallable, setEngineInstallable] = useState(false)
    const [engineId, setEngineId] = useState<number | null>(null)
    /** A newer build of the installed one; Play installs it before starting. */
    const [update, setUpdate] = useState<string | null>(null)
    /** Whether this OS can run an installed build at all (false on mobile). */
    const [canInstallHere, setCanInstallHere] = useState(false)
    const [installing, setInstalling] = useState(false)
    const [fullscreen, setFullscreen] = useState(false)
    /*
     * The ACCOUNT's language, for a build whose launch arguments name
     * `{locale}`. It is an account setting rather than a machine one, so Rust
     * has none and the caller supplies it — and a signed-out user gets the
     * default rather than a blank, which is what the site's own launch
     * resolution does with the same field.
     */
    const locale = user?.locale ?? 'en'

    const [sandboxes, setSandboxes] = useState<SandboxRowT[] | null>(null)
    const [sandboxId, setSandboxId] = useState<string>('')
    const [mode, setMode] = useState<Mode | null>(null)
    const [values, setValues] = useState<PlayOptionValuesT>({})
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<string | null>(null)

    // The sandbox launcher's own confirmation, which shows the resolved argv.
    const [preview, setPreview] = useState<LaunchPreviewT | null>(null)
    const [confirming, setConfirming] = useState(false)

    const { attach, ...live } = useLiveServer<HTMLDivElement>(
        server ? requestFor(server) : null
    )

    const options = useMemo(
        () =>
            (app.play?.options ?? []).filter(
                (o) => !o.modes?.length || o.modes.includes('web')
            ),
        [app.play]
    )

    // Back to the defaults whenever the option list changes. Done during render
    // (React's "adjust state when a prop changes") rather than in an effect, so
    // the first paint already has them.
    const [optionsSeen, setOptionsSeen] = useState<typeof options | null>(null)

    if (optionsSeen !== options) {
        setOptionsSeen(options)
        setValues(defaultsFor(options))
    }

    useEffect(() => {
        let live = true

        void ipc
            .gameNativeTarget(app.id, locale)
            .then((target) => {
                if (!live) return
                /*
                 * A game's own build, or its ENGINE's for a TMC pack game:
                 * TMC publishes the shell once, under the engine app, and the
                 * shell runs each game's pack (dot-server-deploy's
                 * `export-native`). Rust decides which, by the same gate
                 * `game_launch` uses, so a third-party game made with the
                 * same engine is never offered the TMC shell.
                 */
                setNative(target.installed)
                setEngineInstallable(target.engineInstallable)
                setEngineId(target.engineId)
                setUpdate(target.update ?? null)
            })
            .catch(() => {
                if (live) {
                    setNative(null)
                    setEngineInstallable(false)
                }
            })

        return () => {
            live = false
        }
    }, [app.id, locale])

    useEffect(() => {
        let live = true

        void ipc
            .gamesPlatform()
            .then((info) => {
                if (live) setCanInstallHere(info.installable)
            })
            .catch(() => {
                if (live) setCanInstallHere(false)
            })

        return () => {
            live = false
        }
    }, [])

    useEffect(() => {
        let live = true

        void ipc
            .sandboxList(app.id)
            .then((rows) => {
                if (!live) return

                setSandboxes(rows)
                setSandboxId(
                    String(rows.find((r) => r.isDefault)?.id ?? rows[0]?.id ?? '')
                )
            })
            .catch(() => {
                if (live) setSandboxes([])
            })

        return () => {
            live = false
        }
    }, [app.id])

    /*
     * Which modes exist at all.
     *
     * `web` is the server's answer and `directPlay` gates the SERVERLESS case
     * only — launching into a chosen server is allowed without it, which is
     * exactly what that flag means. `connect` needs an address the owner has
     * not hidden.
     */
    /*
     * Which clients this server's owner lets in (`sv_allow_web` /
     * `sv_allow_desktop`, read by the site from the server's query rules).
     * The server turns a closed platform away at the door regardless; this
     * only stops the dialog offering a button that would be refused. Every
     * way this app starts a game into a server other than the web player is
     * the desktop client. Absent (no server, an older site) means open.
     */
    const webClosed = server?.server?.platforms.web === false
    const desktopClosed = server?.server?.platforms.desktop === false

    const canWeb =
        (app.play?.modes.includes('web') ?? false) &&
        (app.play?.directPlay === true || !!server) &&
        !webClosed

    const canConnect = !!server?.server?.connectUrl && !desktopClosed

    const canSandbox = (sandboxes?.length ?? 0) > 0

    /*
     * An installed TMC build can always be started — with a server or without
     * one. `directPlay` is not consulted, and that is deliberate rather than an
     * oversight: the flag answers "does pressing Play with no server start
     * something worth starting", which is a question about a launch the SITE
     * performs. This launch is a binary on this disk, and whether it opens to a
     * menu is the game's own business.
     */
    const canNative = native != null && !desktopClosed

    /*
     * What Install would install: the game's own build, or its engine's shell
     * for a TMC pack game (the two cases `native_target` decides in Rust).
     * Null when TMC publishes nothing this machine can run.
     */
    const engineAppId = engineId ?? app.engine?.id ?? null
    const installTarget = useMemo<{
        appId: number
        name: string
        slug?: string
    } | null>(() => {
        if (native !== null || !canInstallHere || desktopClosed) return null

        if (engineInstallable && engineAppId !== null)
            return { appId: engineAppId, name: app.engine?.name ?? app.name }

        return app.install
            ? { appId: app.id, name: app.name, slug: app.slug ?? undefined }
            : null
    }, [native, canInstallHere, desktopClosed, engineInstallable, engineAppId, app])

    const canInstall = installTarget !== null

    /*
     * Chosen once the facts are in, not on every render.
     *
     * The default is the mode that produces the best result rather than the
     * first available one: an installed copy with its mods beats a browser
     * build of the same game, and both beat handing the address to a client
     * that may not be installed.
     *
     * Done during render: setting a component's own state from what it has
     * just computed is React's supported way to derive it, and an effect for it
     * costs a frame on the wrong mode.
     */
    if (
        mode === null &&
        sandboxes !== null &&
        native !== undefined &&
        (target.app || !looked.isPending)
    ) {
        const best = canNative
            ? 'native'
            : canInstall
              ? 'install'
              : canSandbox
                ? 'sandbox'
                : canWeb
                  ? 'web'
                  : canConnect
                    ? 'connect'
                    : null

        if (best !== null) setMode(best)
    }

    const launchWeb = useCallback(async () => {
        setBusy(true)
        setError(null)

        try {
            await ipc.playOpenWeb({
                appId: app.id,
                serverId: server ? Number(server.id) : undefined,
                options: values,
                title: app.name,
                appSlug: app.slug ?? undefined,
                fullscreen,
            })

            onClose()
        } catch (err) {
            setError(messageOf(err))
        } finally {
            setBusy(false)
        }
    }, [app, server, values, fullscreen, onClose])

    const launchNative = useCallback(async () => {
        setBusy(true)
        setError(null)

        try {
            // An old shell cannot read what the site and servers publish now.
            if (update && native) {
                setInstalling(true)
                await ipc.gameInstall({
                    appId: native.appId,
                    name: native.name,
                    slug: native.slug ?? undefined,
                })
                setUpdate(null)
            }

            /*
             * A server ID, never an address. The host and port are read from
             * the API in Rust, for the same reason `playConnect` reads a
             * connect link there — "point this process at that box" is not a
             * decision the webview makes.
             */
            await ipc.gameLaunch({
                appId: app.id,
                serverId: server ? Number(server.id) : undefined,
                options: values,
                locale,
                solo: server ? undefined : target.solo?.contentId,
                soloVersion: server ? undefined : target.solo?.version,
            })

            onClose()
        } catch (err) {
            setError(messageOf(err))
        } finally {
            setInstalling(false)
            setBusy(false)
        }
    }, [app.id, server, values, locale, target.solo, onClose, update, native])

    /*
     * Install, then start. The build goes through the ordinary download queue
     * (Downloads shows it); `gameInstall` resolves once it is unpacked and
     * verified, and only then is the launch made, with the same `gameLaunch`
     * the installed case uses.
     */
    const installAndPlay = useCallback(async () => {
        if (!installTarget) return

        setBusy(true)
        setInstalling(true)
        setError(null)

        try {
            await ipc.gameInstall(installTarget)

            const now = await ipc.gameNativeTarget(app.id, locale)

            setNative(now.installed)

            if (!now.installed) {
                setError('Installed, but this game cannot be started from it.')

                return
            }

            await ipc.gameLaunch({
                appId: app.id,
                serverId: server ? Number(server.id) : undefined,
                options: values,
                locale,
                solo: server ? undefined : target.solo?.contentId,
                soloVersion: server ? undefined : target.solo?.version,
            })

            onClose()
        } catch (err) {
            setError(messageOf(err))
        } finally {
            setInstalling(false)
            setBusy(false)
        }
    }, [installTarget, app.id, locale, server, values, target.solo, onClose])

    const launchSandbox = useCallback(async () => {
        const id = Number(sandboxId)

        if (!id) return

        setBusy(true)
        setError(null)

        try {
            /*
             * Resolved first and shown, never launched blind. That is the whole
             * reason a declarative launcher is acceptable — see `LaunchDialog`.
             */
            setPreview(await ipc.sandboxLaunchPreview(id))
            setConfirming(true)
        } catch (err) {
            setError(messageOf(err))
        } finally {
            setBusy(false)
        }
    }, [sandboxId])

    const confirmSandbox = useCallback(async () => {
        const id = Number(sandboxId)

        setBusy(true)

        try {
            await ipc.sandboxLaunch(id)

            onClose()
        } catch (err) {
            setError(messageOf(err))
            setConfirming(false)
        } finally {
            setBusy(false)
        }
    }, [sandboxId, onClose])

    const connect = useCallback(async () => {
        if (!server) return

        setBusy(true)
        setError(null)

        try {
            // The ID, not the URL. Rust reads the link and checks its scheme —
            // see `playConnect`.
            await ipc.playConnect(Number(server.id))

            onClose()
        } catch (err) {
            setError(messageOf(err))
        } finally {
            setBusy(false)
        }
    }, [server, onClose])

    if (confirming)
        return (
            <LaunchDialog
                title={`Start ${app.name}?`}
                preview={preview}
                error={error}
                busy={busy}
                onCancel={() => setConfirming(false)}
                onConfirm={() => void confirmSandbox()}
            />
        )

    const nothing =
        !canNative && !canInstall && !canWeb && !canConnect && !canSandbox

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
            <div className="flex max-h-[90vh] w-full max-w-lg flex-col overflow-hidden rounded-xl border border-border bg-surface">
                <header className="flex items-start gap-3 border-b border-border p-4">
                    <GameIcon
                        app={{ id: app.id, name: app.name, icon: app.images.icon }}
                        size="lg"
                    />

                    <div className="min-w-0 flex-1">
                        <h2 className="truncate text-sm font-semibold">
                            {app.name}
                        </h2>

                        {server ? (
                            <p
                                ref={attach}
                                className="mt-0.5 flex items-center gap-2 truncate text-xs text-muted"
                            >
                                <FiServer className="size-3 shrink-0" />
                                <span className="truncate">{server.name}</span>
                                {/* Measured from THIS device. The number a
                                    scanner in another hemisphere reports is not
                                    the number this player will experience, and
                                    it is the one fact the website cannot give
                                    them. */}
                                <LiveLatency
                                    result={live.result}
                                    series={live.series}
                                    error={live.error}
                                    settled={live.settled}
                                    probeable={!!server.server?.query}
                                />
                            </p>
                        ) : (
                            <p className="mt-0.5 text-xs text-muted">
                                {app.play?.help ?? 'Choose how to start.'}
                            </p>
                        )}
                    </div>
                </header>

                <div className="flex-1 overflow-y-auto p-4">
                    {nothing ? (
                        <div className="flex flex-col gap-2 rounded-lg border border-border bg-surface-2 p-3 text-xs text-muted">
                            <p className="flex items-start gap-2">
                                <FiAlertTriangle className="mt-0.5 size-3.5 shrink-0 text-warning" />
                                <span>
                                    There is no way to start {app.name} from this
                                    device yet.
                                </span>
                            </p>
                            {(webClosed || desktopClosed) && (
                                <ClosedNote
                                    web={webClosed}
                                    desktop={desktopClosed}
                                />
                            )}
                            <p>
                                Set up a sandbox for it in{' '}
                                <Link to="/sandboxes" className="underline">
                                    Sandboxes
                                </Link>
                                , or open a server&rsquo;s page to connect directly.
                            </p>
                        </div>
                    ) : (
                        <div className="flex flex-col gap-2">
                            {canNative && native && (
                                <ModeCard
                                    active={mode === 'native'}
                                    onSelect={() => setMode('native')}
                                    icon={FiCpu}
                                    title="Play natively"
                                    body={
                                        update
                                            ? `Updates version ${native.version} to ${update}, then starts it${server ? ' and joins the server' : ''}.`
                                            : server
                                              ? `Starts version ${native.version} on this machine and joins the server.`
                                              : `Starts version ${native.version} installed on this machine.`
                                    }
                                >
                                    {options.length > 0 && (
                                        <div className="flex flex-col gap-3">
                                            {options.map((option) => (
                                                <OptionField
                                                    key={option.key}
                                                    option={option}
                                                    value={values[option.key]}
                                                    onChange={(next) =>
                                                        setValues((prev) => ({
                                                            ...prev,
                                                            [option.key]: next,
                                                        }))
                                                    }
                                                />
                                            ))}
                                        </div>
                                    )}
                                </ModeCard>
                            )}

                            {canInstall && (
                                <ModeCard
                                    active={mode === 'install'}
                                    onSelect={() => setMode('install')}
                                    icon={FiDownload}
                                    title="Install and play locally (recommended)"
                                    body="Downloads the native game once, then starts it. Faster than the web build, with every setting and mod support."
                                >
                                    {installing && (
                                        <p className="mb-3 text-[11px] text-muted">
                                            Downloading and installing. The game
                                            starts when this finishes; progress is
                                            also on the Downloads screen.
                                        </p>
                                    )}

                                    {options.length > 0 && (
                                        <div className="flex flex-col gap-3">
                                            {options.map((option) => (
                                                <OptionField
                                                    key={option.key}
                                                    option={option}
                                                    value={values[option.key]}
                                                    onChange={(next) =>
                                                        setValues((prev) => ({
                                                            ...prev,
                                                            [option.key]: next,
                                                        }))
                                                    }
                                                />
                                            ))}
                                        </div>
                                    )}
                                </ModeCard>
                            )}

                            {canSandbox && (
                                <ModeCard
                                    active={mode === 'sandbox'}
                                    onSelect={() => setMode('sandbox')}
                                    icon={FiLayers}
                                    title="Play the installed copy"
                                    body="Starts the game on this machine with a sandbox's mods deployed and its load order applied."
                                >
                                    <Select
                                        label="Sandbox"
                                        value={sandboxId}
                                        options={(sandboxes ?? []).map((s) => ({
                                            value: String(s.id),
                                            label: s.name,
                                            hint: s.isDefault
                                                ? 'Default'
                                                : undefined,
                                        }))}
                                        onChange={setSandboxId}
                                        fullWidth
                                    />
                                </ModeCard>
                            )}

                            {canWeb && (
                                <ModeCard
                                    active={mode === 'web'}
                                    onSelect={() => setMode('web')}
                                    icon={FiGlobe}
                                    title="Play in a window (web build)"
                                    body="Runs the game's web build in a window of its own. Nothing is installed."
                                >
                                    <p className="mb-3 flex items-start gap-2 rounded-lg border border-warning/40 bg-warning/10 p-2 text-[11px] text-muted">
                                        <FiAlertTriangle className="mt-0.5 size-3 shrink-0 text-warning" />
                                        <span>
                                            The web build runs in a browser view. It
                                            is slower, may lack features (mods, some
                                            settings, threads), and is not the
                                            native game.
                                        </span>
                                    </p>

                                    <label className="mb-3 flex cursor-pointer items-start gap-2 text-xs">
                                        <input
                                            type="checkbox"
                                            className="mt-0.5"
                                            checked={fullscreen}
                                            onChange={(e) =>
                                                setFullscreen(e.target.checked)
                                            }
                                        />
                                        <span className="flex flex-col gap-0.5">
                                            <span>Start full screen</span>
                                            {/* The window is a REMOTE page with
                                                no IPC, so it cannot offer its
                                                own control for this — the
                                                choice has to be made here,
                                                before it opens. Escape leaves
                                                full screen. */}
                                            <span className="text-[11px] text-muted">
                                                Press Escape to leave full screen.
                                            </span>
                                        </span>
                                    </label>

                                    {options.length > 0 && (
                                        <div className="flex flex-col gap-3">
                                            {options.map((option) => (
                                                <OptionField
                                                    key={option.key}
                                                    option={option}
                                                    value={values[option.key]}
                                                    onChange={(next) =>
                                                        setValues((prev) => ({
                                                            ...prev,
                                                            [option.key]: next,
                                                        }))
                                                    }
                                                />
                                            ))}
                                        </div>
                                    )}
                                </ModeCard>
                            )}

                            {canConnect && (
                                <ModeCard
                                    active={mode === 'connect'}
                                    onSelect={() => setMode('connect')}
                                    icon={FiExternalLink}
                                    title="Connect with the game client"
                                    body="Hands the server's address to the copy already installed on this machine, through its own launcher."
                                />
                            )}

                            {(webClosed || desktopClosed) && (
                                <ClosedNote
                                    web={webClosed}
                                    desktop={desktopClosed}
                                />
                            )}

                            {!canSandbox && (
                                <p className="flex items-start gap-2 px-1 text-[11px] text-muted">
                                    <FiHardDrive className="mt-0.5 size-3 shrink-0" />
                                    <span>
                                        No sandbox exists for this game on this
                                        device.{' '}
                                        <Link to="/library" className="underline">
                                            Add one
                                        </Link>{' '}
                                        to play it with mods.
                                    </span>
                                </p>
                            )}
                        </div>
                    )}

                    {error && (
                        <p className="mt-3 flex items-start gap-2 rounded-lg border border-danger/40 bg-danger/10 p-2 text-xs text-danger">
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
                        Cancel
                    </button>

                    <button
                        type="button"
                        disabled={busy || mode === null}
                        onClick={() => {
                            if (mode === 'native') void launchNative()
                            else if (mode === 'install') void installAndPlay()
                            else if (mode === 'web') void launchWeb()
                            else if (mode === 'sandbox') void launchSandbox()
                            else if (mode === 'connect') void connect()
                        }}
                        className="flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-xs font-semibold text-accent-foreground disabled:opacity-50"
                    >
                        <FiPlay className="size-3.5" />
                        {installing
                            ? 'Installing…'
                            : busy
                              ? 'Starting…'
                              : mode === 'install'
                                ? 'Install and play'
                                : 'Play'}
                    </button>
                </footer>
            </div>
        </div>
    )
}

function ModeCard({
    active,
    onSelect,
    icon: Icon,
    title,
    body,
    children,
}: {
    active: boolean
    onSelect: () => void
    icon: typeof FiPlay
    title: string
    body: string
    children?: React.ReactNode
}) {
    return (
        <div
            className={`rounded-lg border p-3 transition ${
                active ? 'border-accent bg-accent/5' : 'border-border'
            }`}
        >
            <button
                type="button"
                onClick={onSelect}
                className="flex w-full items-start gap-2.5 text-left"
            >
                <Icon
                    className={`mt-0.5 size-4 shrink-0 ${active ? 'text-accent' : 'text-muted'}`}
                />
                <span className="flex flex-col gap-0.5">
                    <span className="text-xs font-semibold">{title}</span>
                    <span className="text-[11px] text-muted">{body}</span>
                </span>
            </button>

            {/* Only the selected mode's controls, so the dialog is a choice
                followed by its settings rather than three forms at once. */}
            {active && children ? (
                <div className="mt-3 border-t border-border pt-3">{children}</div>
            ) : null}
        </div>
    )
}
