import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { open } from '@tauri-apps/plugin-dialog'
import { FiAlertTriangle, FiShield } from 'react-icons/fi'
import { Button } from '@modcommunity/shared'

import { ipc } from '~/lib/ipc/commands'
import { isIpcError } from '~/lib/ipc'
import type { PluginPreviewT, PluginRecordT } from '~/lib/ipc/schemas'
import { Toggle } from '~/components/form'

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
export default function PluginsRoute() {
    const queryClient = useQueryClient()

    const [preview, setPreview] = useState<PluginPreviewT | null>(null)
    const [error, setError] = useState<string | null>(null)

    const plugins = useQuery({ queryKey: ['plugins'], queryFn: () => ipc.pluginList() })

    const refresh = () =>
        void queryClient.invalidateQueries({ queryKey: ['plugins'] })

    const pick = async () => {
        setError(null)

        const dir = await open({
            directory: true,
            multiple: false,
            title: 'Choose a plugin folder',
        })

        if (typeof dir !== 'string') return

        try {
            setPreview(await ipc.pluginInspect(dir))
        } catch (err) {
            setError(
                isIpcError(err) ? err.message : 'That folder is not a plugin.'
            )
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
                    declarative — a plugin describes steps this app carries out,
                    it never runs code of its own.
                </p>
                <Button btnType="primary" onClick={() => void pick()}>
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

                    <p className="selectable break-all text-[0.65rem] text-muted">
                        Fingerprint {preview.fingerprint.slice(0, 32)}…
                    </p>

                    <div className="flex gap-2">
                        <Button btnType="primary" onClick={() => void approve()}>
                            Approve and install
                        </Button>
                        <Button btnType="secondary" onClick={() => setPreview(null)}>
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
                    This plugin changed since you approved it and has been
                    disabled. Add it again to review the new permissions.
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
