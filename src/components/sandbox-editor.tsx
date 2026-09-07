import { useEffect, useMemo, useState } from 'react'
import { FiAlertTriangle, FiTrash2 } from 'react-icons/fi'

import { ipc } from '~/lib/ipc/commands'
import { messageOf } from '~/lib/ipc'
import Select from '~/components/select'
import { useFolderPicker } from '~/components/folder-picker'
import type { SandboxRowT, SandboxSpecT } from '~/lib/ipc/schemas'

/**
 * **Creating and editing a sandbox**, in one dialog for both.
 *
 * One component rather than two because the fields are the same fields and a
 * separate "new" form is how the two drift — a strategy offered on create and
 * missing on edit is a sandbox somebody cannot fix.
 *
 * WHAT THIS FORM IS ALLOWED TO OFFER
 * ---------------------------------
 * Only what the GAME says is possible. `sandbox.json` declares which deployment
 * strategies suit it and which presets it has, and `sandboxStrategies` answers
 * which of those actually work on THIS machine — staging on another drive rules
 * out hard links, a Windows box without Developer Mode rules out symlinks.
 * Offering a strategy that would fail and then failing is worse than not
 * offering it, so an unavailable one is shown with its reason rather than
 * silently dropped: "greyed out with no explanation" is the thing this avoids.
 *
 * The preset is applied in RUST. It carries a strategy and a set of option
 * values, and assembling those here would produce a sandbox whose settings
 * never went through the game's own schema.
 */

const ENVIRONMENTS = [
    {
        value: 'client' as const,
        label: 'Client',
        hint: 'A copy of the game you play.',
    },
    {
        value: 'server' as const,
        label: 'Server',
        hint: 'A dedicated server you host.',
    },
    {
        value: 'shared' as const,
        label: 'Both',
        hint: 'Accepts mods for either side.',
    },
]

const STRATEGY_LABELS: Record<string, string> = {
    direct: 'Copy into the game folder',
    hardlink: 'Hard links',
    symlink: 'Symbolic links',
    usvfs: 'Virtual filesystem',
}

export default function SandboxEditor({
    appId,
    appSlug,
    appName,
    existing,
    onClose,
    onSaved,
}: {
    appId: number
    appSlug: string | null
    appName: string | null
    /** Null creates a new sandbox; a row edits that one. */
    existing: SandboxRowT | null
    onClose: () => void
    onSaved: () => void
}) {
    const { pick, element: picker } = useFolderPicker()

    const [name, setName] = useState(existing?.name ?? '')
    const [description, setDescription] = useState(existing?.description ?? '')
    const [environment, setEnvironment] = useState<'client' | 'server' | 'shared'>(
        existing?.environment ?? 'client'
    )
    const [strategy, setStrategy] = useState<string>(existing?.strategy ?? '')
    const [preset, setPreset] = useState<string>(existing?.preset ?? '')
    const [gameDir, setGameDir] = useState<string | null>(existing?.gameDir ?? null)
    const [cloudSync, setCloudSync] = useState(existing?.cloudSync ?? true)
    const [autoUpdate, setAutoUpdate] = useState(existing?.autoUpdate ?? true)

    const [spec, setSpec] = useState<SandboxSpecT | null>(null)
    const [available, setAvailable] = useState<
        { strategy: string; available: boolean; reason: string | null }[]
    >([])
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const [confirmDelete, setConfirmDelete] = useState(false)

    useEffect(() => {
        if (!appSlug) return

        void ipc
            .sandboxSpec(appSlug)
            .then(setSpec)
            .catch(() => undefined)
    }, [appSlug])

    useEffect(() => {
        if (!existing) return

        void ipc
            .sandboxStrategies(existing.id)
            .then(setAvailable)
            .catch(() => undefined)
    }, [existing])

    // The game's own default, once its rules have loaded and only while the
    // user has not chosen. Seeding state from a prop that arrives later is what
    // makes this an effect rather than an initialiser.
    useEffect(() => {
        if (strategy || !spec) return

        /*
         * `defaultStrategy` is nullable in the spec — a game may declare which
         * strategies it supports and decline to prefer one. Falling back to the
         * first supported entry keeps the picker from opening on an empty
         * value, which would make Create refuse for a reason nothing on screen
         * explains.
         */
        setStrategy(
            spec.deploy.defaultStrategy ??
                spec.deploy.supportedStrategies[0] ??
                'direct'
        )
    }, [spec, strategy])

    const strategies = useMemo(() => {
        const supported = spec?.deploy.supportedStrategies ?? [
            'direct',
            'hardlink',
            'symlink',
        ]

        return supported.map((value) => {
            const probe = available.find((a) => a.strategy === value)

            return {
                value,
                label: STRATEGY_LABELS[value] ?? value,
                hint:
                    probe && !probe.available
                        ? (probe.reason ?? 'Not available here')
                        : undefined,
                disabled: probe ? !probe.available : false,
            }
        })
    }, [spec, available])

    const save = async () => {
        if (!name.trim()) {
            setError('Give the sandbox a name.')

            return
        }

        setBusy(true)
        setError(null)

        try {
            if (existing) {
                await ipc.sandboxPatch(existing.id, {
                    name: name.trim(),
                    description: description.trim() || null,
                    environment,
                    strategy: strategy as 'direct',
                    gameDir,
                    cloudSync,
                    autoUpdate,
                })
            } else {
                await ipc.sandboxCreate(
                    {
                        appId,
                        appSlug,
                        appName,
                        name: name.trim(),
                        description: description.trim() || null,
                        environment,
                        strategy: strategy ? (strategy as 'direct') : undefined,
                        gameDir,
                        cloudSync,
                        autoUpdate,
                    },
                    preset || undefined
                )
            }

            onSaved()
            onClose()
        } catch (err) {
            setError(messageOf(err))
        } finally {
            setBusy(false)
        }
    }

    const remove = async (keepFiles: boolean) => {
        if (!existing) return

        setBusy(true)
        setError(null)

        try {
            await ipc.sandboxDelete(existing.id, keepFiles)

            onSaved()
            onClose()
        } catch (err) {
            setError(messageOf(err))
            setBusy(false)
        }
    }

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
            <div className="flex max-h-[90vh] w-full max-w-md flex-col overflow-hidden rounded-xl border border-border bg-surface">
                <header className="border-b border-border p-4">
                    <h2 className="text-sm font-semibold">
                        {existing ? 'Edit sandbox' : 'New sandbox'}
                    </h2>
                    <p className="text-[11px] text-muted">
                        {appName ?? 'This game'} · a named set of mods with its own
                        load order and launch settings.
                    </p>
                </header>

                <div className="flex flex-1 flex-col gap-3 overflow-y-auto p-4">
                    <Field label="Name">
                        <input
                            type="text"
                            value={name}
                            maxLength={64}
                            onChange={(e) => setName(e.target.value)}
                            placeholder="Modded"
                            className="w-full rounded-lg border border-border bg-surface-2 px-2 py-1.5 text-xs"
                        />
                    </Field>

                    <Field label="Description" optional>
                        <input
                            type="text"
                            value={description}
                            maxLength={500}
                            onChange={(e) => setDescription(e.target.value)}
                            className="w-full rounded-lg border border-border bg-surface-2 px-2 py-1.5 text-xs"
                        />
                    </Field>

                    {!existing && (spec?.presets.length ?? 0) > 0 && (
                        <Field
                            label="Preset"
                            hint="Applied by the app, so its settings go through the game's own schema."
                            optional
                        >
                            <Select
                                label="Preset"
                                value={preset}
                                placeholder="None"
                                options={[
                                    { value: '', label: 'None' },
                                    ...(spec?.presets ?? []).map((p) => ({
                                        value: p.id,
                                        label: p.label,
                                    })),
                                ]}
                                onChange={setPreset}
                                fullWidth
                            />
                        </Field>
                    )}

                    <Field label="For">
                        <Select
                            label="Environment"
                            value={environment}
                            options={ENVIRONMENTS}
                            onChange={setEnvironment}
                            fullWidth
                        />
                    </Field>

                    <Field
                        label="How mods reach the game"
                        hint={
                            spec?.deploy.antiCheat === 'kernel'
                                ? 'This game uses kernel anti-cheat. Anything but copying can be seen as tampering — the cost of guessing wrong is the account, not a failed install.'
                                : undefined
                        }
                    >
                        <Select
                            label="Deployment"
                            value={strategy}
                            options={strategies}
                            onChange={setStrategy}
                            fullWidth
                        />
                    </Field>

                    <Field
                        label="Game folder"
                        hint="Leave unset to use the folder configured for this game."
                        optional
                    >
                        <div className="flex items-center gap-2">
                            <span className="selectable min-w-0 flex-1 truncate rounded-lg border border-border bg-surface-2 px-2 py-1.5 text-[11px] text-muted">
                                {gameDir ?? existing?.targetDir ?? 'Not set'}
                            </span>
                            <button
                                type="button"
                                onClick={() =>
                                    void pick({
                                        title: 'Choose the game folder',
                                        hint: "This becomes the sandbox's deployment target.",
                                        initial: gameDir,
                                    }).then((dir) => {
                                        if (dir !== null) setGameDir(dir)
                                    })
                                }
                                className="shrink-0 rounded-lg border border-border px-2.5 py-1.5 text-[11px]"
                            >
                                Choose
                            </button>
                        </div>
                    </Field>

                    <label className="flex items-start gap-2 text-xs">
                        <input
                            type="checkbox"
                            className="mt-0.5"
                            checked={cloudSync}
                            onChange={(e) => setCloudSync(e.target.checked)}
                        />
                        <span className="flex flex-col gap-0.5">
                            <span>Keep this on my account</span>
                            <span className="text-[11px] text-muted">
                                Its name, mods and load order follow you to another
                                machine. Off means it never leaves this device.
                            </span>
                        </span>
                    </label>

                    <label className="flex items-start gap-2 text-xs">
                        <input
                            type="checkbox"
                            className="mt-0.5"
                            checked={autoUpdate}
                            onChange={(e) => setAutoUpdate(e.target.checked)}
                        />
                        <span className="flex flex-col gap-0.5">
                            <span>Keep its mods up to date</span>
                            <span className="text-[11px] text-muted">
                                Stages the newest release each subscription offers.
                            </span>
                        </span>
                    </label>

                    {error && (
                        <p className="flex items-start gap-2 rounded-lg border border-danger/40 bg-danger/10 p-2 text-xs text-danger">
                            <FiAlertTriangle className="mt-0.5 size-3.5 shrink-0" />
                            {error}
                        </p>
                    )}

                    {confirmDelete && existing && (
                        <div className="flex flex-col gap-2 rounded-lg border border-danger/40 bg-danger/10 p-3 text-xs">
                            <p className="font-medium text-danger">
                                Delete “{existing.name}”?
                            </p>
                            {/*
                             * The ledger goes with the row, so the choice has
                             * to be made now: it is the only record of which
                             * files in the game folder are ours, and after the
                             * row is gone nothing can tell them from files the
                             * game shipped with.
                             */}
                            <p className="text-muted">
                                Its deployed files are removed from the game folder
                                first. Keeping them leaves files nothing can
                                undeploy later.
                            </p>
                            <div className="flex flex-wrap gap-2">
                                <button
                                    type="button"
                                    disabled={busy}
                                    onClick={() => void remove(false)}
                                    className="rounded-lg bg-danger px-2.5 py-1 text-[11px] font-semibold text-white disabled:opacity-50"
                                >
                                    Remove its files and delete
                                </button>
                                <button
                                    type="button"
                                    disabled={busy}
                                    onClick={() => void remove(true)}
                                    className="rounded-lg border border-danger/50 px-2.5 py-1 text-[11px] text-danger disabled:opacity-50"
                                >
                                    Leave the files, delete anyway
                                </button>
                                <button
                                    type="button"
                                    onClick={() => setConfirmDelete(false)}
                                    className="rounded-lg border border-border px-2.5 py-1 text-[11px]"
                                >
                                    Cancel
                                </button>
                            </div>
                        </div>
                    )}
                </div>

                <footer className="flex items-center gap-2 border-t border-border p-4">
                    {existing && !confirmDelete && (
                        <button
                            type="button"
                            onClick={() => setConfirmDelete(true)}
                            className="flex items-center gap-1.5 rounded-lg border border-border px-2.5 py-1.5 text-xs text-danger"
                        >
                            <FiTrash2 className="size-3.5" />
                            Delete
                        </button>
                    )}

                    <button
                        type="button"
                        onClick={onClose}
                        className="ml-auto rounded-lg border border-border px-3 py-1.5 text-xs"
                    >
                        Cancel
                    </button>

                    <button
                        type="button"
                        disabled={busy || !name.trim()}
                        onClick={() => void save()}
                        className="rounded-lg bg-accent px-3 py-1.5 text-xs font-semibold text-accent-foreground disabled:opacity-50"
                    >
                        {busy ? 'Saving…' : existing ? 'Save' : 'Create'}
                    </button>
                </footer>
            </div>

            {picker}
        </div>
    )
}

function Field({
    label,
    hint,
    optional,
    children,
}: {
    label: string
    hint?: string
    optional?: boolean
    children: React.ReactNode
}) {
    return (
        <label className="flex flex-col gap-1">
            <span className="text-[11px] font-medium uppercase tracking-wide text-muted">
                {label}
                {optional && <span className="ml-1 opacity-60">optional</span>}
            </span>
            {children}
            {hint && <span className="text-[11px] text-muted">{hint}</span>}
        </label>
    )
}
