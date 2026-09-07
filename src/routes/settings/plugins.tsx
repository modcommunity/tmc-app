import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { FiAlertTriangle, FiShield } from 'react-icons/fi'
import { Button } from '@modcommunity/shared'

import { ipc } from '~/lib/ipc/commands'
import { isIpcError, messageOf } from '~/lib/ipc'
import type {
    PluginPreviewT,
    PluginRecordT,
    SignatureStateT,
} from '~/lib/ipc/schemas'
import { Toggle } from '~/components/form'
import { useFolderPicker } from '~/components/folder-picker'

/**
 * The plugin manager, and the consent screen in front of it.
 *
 * Approval is the whole security model's hinge, so the dialog shows the plugin's
 * FULL permission list — every entry, no summarising, no "and 2 more". The
 * fingerprint the preview returns is passed back to `pluginApprove`, which is
 * what stops a bundle being swapped between the moment it is read and the
 * moment it is trusted.
 *
 * A plugin whose manifest later changes is disabled by Rust on the next launch
 * and marked `needsReapproval`; the only way back is through this dialog, with
 * the new permissions on screen.
 */
/**
 * Who signed a plugin, in one line.
 *
 * Three states because there are three facts, and the middle one is the whole
 * reason this exists: a bundle carrying a signature nobody can check is either
 * from a publisher whose key has not been added yet or has been tampered with,
 * and folding that into "unsigned" is how the interesting case disappears.
 */
function SignatureBadge({ signature }: { signature: SignatureStateT }) {
    if (signature.state === 'trusted') {
        return (
            <span className="flex items-center gap-1.5 text-xs text-success">
                <FiShield className="size-3.5 shrink-0" />
                Signed by {signature.keyId}
            </span>
        )
    }

    if (signature.state === 'untrusted') {
        return (
            <span className="flex items-start gap-1.5 text-xs text-warning">
                <FiAlertTriangle className="mt-0.5 size-3.5 shrink-0" />
                <span>
                    Signed by a key you have not trusted. Add the publisher’s key
                    below if you recognise it — otherwise treat this as unverified.
                </span>
            </span>
        )
    }

    return (
        <span className="flex items-center gap-1.5 text-xs text-muted">
            <FiShield className="size-3.5 shrink-0" />
            Not signed
        </span>
    )
}

/**
 * The keys a plugin may be signed by.
 *
 * The store is the USER's, and nothing is compiled into the app — TMC's own
 * publishing key will be a row here like anybody else's. That is the honest
 * shape while there is no registry to distribute plugins through, and it stays
 * correct once there is.
 */
function TrustedKeys() {
    const keys = useQuery({
        queryKey: ['plugin-keys'],
        queryFn: () => ipc.pluginTrustedKeys(),
    })

    const queryClient = useQueryClient()

    const [adding, setAdding] = useState(false)
    const [id, setId] = useState('')
    const [label, setLabel] = useState('')
    const [publicKey, setPublicKey] = useState('')
    const [error, setError] = useState<string | null>(null)

    const refresh = async () => {
        await queryClient.invalidateQueries({ queryKey: ['plugin-keys'] })
        await queryClient.invalidateQueries({ queryKey: ['plugins'] })
    }

    const add = async () => {
        setError(null)

        try {
            await ipc.pluginTrustKey(id.trim(), label.trim(), publicKey.trim())

            setAdding(false)
            setId('')
            setLabel('')
            setPublicKey('')

            await refresh()
        } catch (err) {
            setError(messageOf(err))
        }
    }

    return (
        <section className="flex flex-col gap-2 rounded-xl border border-border p-3">
            <div className="flex items-center justify-between gap-2">
                <div>
                    <h2 className="text-sm font-semibold">Trusted publishers</h2>
                    <p className="text-xs text-muted">
                        A plugin signed by one of these keys is verified. A
                        signature says who wrote a plugin — it does not make the
                        plugin safe, and a signed plugin is confined by exactly the
                        same rules as any other.
                    </p>
                </div>

                <Button btnType="secondary" onClick={() => setAdding(!adding)}>
                    {adding ? 'Cancel' : 'Add a key'}
                </Button>
            </div>

            {adding && (
                <div className="flex flex-col gap-2 rounded-lg border border-border p-2">
                    <label className="text-xs">
                        Name
                        <input
                            value={id}
                            onChange={(e) => setId(e.target.value)}
                            placeholder="tmc"
                            className="mt-1 w-full rounded-lg border border-border bg-surface-2 px-2 py-1.5 text-xs"
                        />
                    </label>

                    <label className="text-xs">
                        Who it belongs to
                        <input
                            value={label}
                            onChange={(e) => setLabel(e.target.value)}
                            placeholder="The Modding Community"
                            className="mt-1 w-full rounded-lg border border-border bg-surface-2 px-2 py-1.5 text-xs"
                        />
                    </label>

                    <label className="text-xs">
                        Public key
                        <input
                            value={publicKey}
                            onChange={(e) => setPublicKey(e.target.value)}
                            placeholder="64 hexadecimal characters"
                            spellCheck={false}
                            className="selectable mt-1 w-full rounded-lg border border-border bg-surface-2 px-2 py-1.5 font-mono text-[11px]"
                        />
                    </label>

                    {error && <p className="text-xs text-danger">{error}</p>}

                    <Button btnType="primary" onClick={() => void add()}>
                        Trust this key
                    </Button>
                </div>
            )}

            {keys.data?.length === 0 && !adding && (
                <p className="text-xs text-muted">
                    No keys yet, so no plugin can be verified.
                </p>
            )}

            {keys.data?.map((key) => (
                <div
                    key={key.id}
                    className="flex items-center justify-between gap-2 rounded-lg border border-border px-2 py-1.5"
                >
                    <div className="min-w-0">
                        <p className="truncate text-xs font-medium">
                            {key.label || key.id}
                            <span className="ml-1.5 font-normal text-muted">
                                {key.id}
                            </span>
                        </p>
                        <p className="selectable truncate font-mono text-[10px] text-muted">
                            {key.publicKey}
                        </p>
                    </div>

                    <button
                        type="button"
                        aria-label={`Stop trusting ${key.label || key.id}`}
                        onClick={() => {
                            void ipc.pluginUntrustKey(key.id).then(refresh)
                        }}
                        className="shrink-0 rounded-lg border border-border px-2 py-1 text-[11px] text-muted hover:border-danger hover:text-danger"
                    >
                        Remove
                    </button>
                </div>
            ))}
        </section>
    )
}

export default function PluginsRoute() {
    const queryClient = useQueryClient()

    const [preview, setPreview] = useState<PluginPreviewT | null>(null)
    const [error, setError] = useState<string | null>(null)

    const { pick, element: picker } = useFolderPicker()

    const plugins = useQuery({
        queryKey: ['plugins'],
        queryFn: () => ipc.pluginList(),
    })

    const refresh = () =>
        void queryClient.invalidateQueries({ queryKey: ['plugins'] })

    const choose = async () => {
        setError(null)

        const dir = await pick({
            title: 'Choose a plugin folder',
            hint: 'The folder containing the plugin’s manifest.',
        })

        if (dir === null) return

        try {
            setPreview(await ipc.pluginInspect(dir))
        } catch (err) {
            setError(isIpcError(err) ? err.message : 'That folder is not a plugin.')
        }
    }

    const approve = async () => {
        if (!preview) return

        try {
            await ipc.pluginApprove(preview.dir, preview.fingerprint)
            setPreview(null)
            refresh()
        } catch (err) {
            setError(isIpcError(err) ? err.message : 'Could not install it.')
        }
    }

    return (
        <div className="flex flex-col gap-4">
            <div className="flex items-start justify-between gap-3">
                <p className="text-xs text-muted">
                    Plugins add game support, server queries and themes. They are
                    declarative — a plugin describes steps this app carries out, it
                    never runs code of its own.
                </p>
                <Button btnType="primary" onClick={() => void choose()}>
                    Add plugin
                </Button>
            </div>

            {error && (
                <p className="rounded-lg border border-danger p-3 text-xs text-danger">
                    {error}
                </p>
            )}

            {preview && (
                <div className="flex flex-col gap-3 rounded-xl border border-accent bg-surface p-4">
                    <div className="flex items-center gap-2">
                        <FiShield className="size-4 text-accent" />
                        <h2 className="text-sm font-semibold">
                            Approve “{preview.name}” v{preview.version}?
                        </h2>
                    </div>

                    <p className="text-xs text-muted">
                        by {preview.author} · {preview.kinds.join(', ')}
                    </p>

                    {preview.description && (
                        <p className="text-xs">{preview.description}</p>
                    )}

                    <div>
                        <p className="mb-1 text-xs font-medium">
                            This plugin will be allowed to:
                        </p>
                        <ul className="flex list-disc flex-col gap-0.5 pl-5 text-xs text-muted">
                            {preview.permissions.length === 0 ? (
                                <li>Nothing outside the app itself.</li>
                            ) : (
                                preview.permissions.map((permission) => (
                                    <li key={permission}>{permission}</li>
                                ))
                            )}
                        </ul>
                    </div>

                    {/*
                     * Beside the permissions, not after the fact. Who vouched
                     * for a plugin is part of what is being decided here.
                     */}
                    <SignatureBadge signature={preview.signature} />

                    <p className="selectable break-all text-[0.65rem] text-muted">
                        Fingerprint {preview.fingerprint.slice(0, 32)}…
                    </p>

                    <div className="flex gap-2">
                        <Button btnType="primary" onClick={() => void approve()}>
                            Approve and install
                        </Button>
                        <Button
                            btnType="secondary"
                            onClick={() => setPreview(null)}
                        >
                            Cancel
                        </Button>
                    </div>
                </div>
            )}

            {plugins.isPending ? (
                <p className="text-sm text-muted">Loading…</p>
            ) : (plugins.data?.length ?? 0) === 0 ? (
                <p className="py-8 text-center text-sm text-muted">
                    No plugins installed.
                </p>
            ) : (
                <ul className="flex flex-col gap-2">
                    {plugins.data?.map((plugin) => (
                        <PluginRow
                            key={plugin.id}
                            plugin={plugin}
                            onChanged={refresh}
                        />
                    ))}
                </ul>
            )}

            {/*
             * Last, under the plugins themselves: it is the thing a user comes
             * looking for AFTER seeing "signed by a key you have not trusted"
             * on one of the rows above.
             */}
            <TrustedKeys />

            {picker}
        </div>
    )
}

function PluginRow({
    plugin,
    onChanged,
}: {
    plugin: PluginRecordT
    onChanged: () => void
}) {
    const [confirmRemove, setConfirmRemove] = useState(false)

    return (
        <li className="flex flex-col gap-2 rounded-xl border border-border bg-surface p-3">
            <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                    <p className="text-sm font-medium">
                        {plugin.name}{' '}
                        <span className="text-xs font-normal text-muted">
                            v{plugin.version}
                        </span>
                    </p>
                    <p className="text-xs text-muted">
                        by {plugin.author} · {plugin.kinds.join(', ')}
                    </p>
                    <div className="mt-0.5">
                        <SignatureBadge signature={plugin.signature} />
                    </div>
                </div>

                <Toggle
                    label={`Enable ${plugin.name}`}
                    checked={plugin.enabled}
                    disabled={plugin.needsReapproval}
                    onChange={(enabled) => {
                        void ipc
                            .pluginSetEnabled(plugin.id, enabled)
                            .then(onChanged)
                    }}
                />
            </div>

            {plugin.needsReapproval && (
                <p className="flex items-start gap-1.5 rounded-lg border border-warning p-2 text-xs text-warning">
                    <FiAlertTriangle className="mt-0.5 size-3.5 shrink-0" />
                    This plugin changed since you approved it and has been disabled.
                    Add it again to review the new permissions.
                </p>
            )}

            <details className="text-xs">
                <summary className="cursor-pointer text-muted">
                    Permissions ({plugin.permissions.length})
                </summary>
                <ul className="mt-1 flex list-disc flex-col gap-0.5 pl-5 text-muted">
                    {plugin.permissions.map((permission) => (
                        <li key={permission}>{permission}</li>
                    ))}
                </ul>
            </details>

            {confirmRemove ? (
                <div className="flex items-center gap-2">
                    <Button
                        btnType="danger"
                        onClick={() => {
                            void ipc.pluginRemove(plugin.id).then(() => {
                                setConfirmRemove(false)
                                onChanged()
                            })
                        }}
                    >
                        Remove
                    </Button>
                    <Button
                        btnType="secondary"
                        onClick={() => setConfirmRemove(false)}
                    >
                        Cancel
                    </Button>
                </div>
            ) : (
                <button
                    type="button"
                    onClick={() => setConfirmRemove(true)}
                    className="self-start text-xs text-danger underline"
                >
                    Remove
                </button>
            )}
        </li>
    )
}
