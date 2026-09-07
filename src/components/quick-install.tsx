import { useMemo, useState } from 'react'
import { Link } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import {
    FiAlertTriangle,
    FiCheck,
    FiDownloadCloud,
    FiLayers,
    FiPlus,
} from 'react-icons/fi'

import { ipc } from '~/lib/ipc/commands'
import { messageOf } from '~/lib/ipc'
import { useAuth } from '~/lib/auth/provider'
import Select from '~/components/select'
import SandboxEditor from '~/components/sandbox-editor'
import type { ContentSummaryT } from '~/lib/api/contract'
import type { QuickInstallReportT } from '~/lib/ipc/schemas'

/**
 * **One-click install into a sandbox** — for a mod or an asset, identically.
 *
 * The app already had two buttons on an item's page and neither did this.
 * *Subscribe* keeps the item on every device and materialises it into the game's
 * MAIN folder; the plugin *Install* runs a registry installer against that same
 * folder. Both write to the copy of the game everything shares — which is the
 * one thing a mod manager exists to avoid.
 *
 * This one puts it in a profile. Pick a sandbox, press it once, and the item is
 * subscribed, staged and deployed into that sandbox's own view of the game.
 *
 * WHY THE WHOLE THING IS ONE CALL
 * ------------------------------
 * `sandboxInstallItem` is a single Rust command covering all five steps, and
 * the five are not exposed here in sequence on purpose — see its doc comment.
 * The frontend names a sandbox and an item; it does not own the plan.
 *
 * ASSETS ARE NOT A SPECIAL CASE
 * ----------------------------
 * A resource pack and a jar mod differ only in which app rule matches them
 * (`manage_asset` versus `manage_mod`), and the executor already picks. So this
 * component takes `summary.kind` and passes it through, and there is no branch
 * anywhere in it that reads the word "asset".
 */
export default function QuickInstall({ summary }: { summary: ContentSummaryT }) {
    const { status } = useAuth()

    const [sandboxId, setSandboxId] = useState('')
    const [busy, setBusy] = useState(false)
    const [report, setReport] = useState<QuickInstallReportT | null>(null)
    const [error, setError] = useState<string | null>(null)
    const [creating, setCreating] = useState(false)

    const appId = summary.app?.id ?? null

    const sandboxes = useQuery({
        queryKey: ['sandboxes', appId],
        queryFn: () => ipc.sandboxList(appId ?? undefined),
        enabled: appId !== null,
        staleTime: 10 * 1000,
    })

    const options = useMemo(
        () =>
            (sandboxes.data ?? []).map((s) => ({
                value: String(s.id),
                label: s.name,
                hint: s.isDefault ? 'Default' : undefined,
            })),
        [sandboxes.data]
    )

    // The default sandbox, once the list has arrived and while the user has not
    // chosen. A picker that opens empty makes the button refuse for a reason
    // nothing on screen explains.
    const chosen =
        sandboxId ||
        String(
            sandboxes.data?.find((s) => s.isDefault)?.id ??
                sandboxes.data?.[0]?.id ??
                ''
        )

    if (appId === null) return null

    const run = async () => {
        if (!chosen) return

        setBusy(true)
        setError(null)
        setReport(null)

        try {
            setReport(
                await ipc.sandboxInstallItem(
                    Number(chosen),
                    summary.kind,
                    Number(summary.id)
                )
            )

            void sandboxes.refetch()
        } catch (err) {
            setError(messageOf(err))
        } finally {
            setBusy(false)
        }
    }

    if (status !== 'signedIn')
        return (
            <p className="rounded-lg border border-border px-3 py-2 text-xs text-muted">
                <Link to="/account" className="underline">
                    Sign in
                </Link>{' '}
                to install this into a sandbox.
            </p>
        )

    if (report) {
        /*
         * A staged-but-not-deployed result is reported as a partial rather than
         * a success. The game folder is unchanged in that case, and calling it
         * "installed" is how somebody launches the game, finds nothing, and
         * concludes the manager is broken.
         */
        const ok = report.staged?.ok === true

        return (
            <div
                className={`flex flex-col gap-1 rounded-lg border px-3 py-2 text-xs ${
                    ok ? 'border-success text-success' : 'border-danger text-danger'
                }`}
            >
                <span className="flex items-center gap-2">
                    {ok ? (
                        <FiCheck className="size-3.5 shrink-0" />
                    ) : (
                        <FiAlertTriangle className="size-3.5 shrink-0" />
                    )}
                    {ok
                        ? report.deployed
                            ? `Installed — ${report.deployed.placed} file(s) placed.`
                            : 'Staged. Deploy the sandbox to put it in the game.'
                        : (report.staged?.error ?? 'The install did not finish.')}
                </span>

                {report.subscribed && (
                    <span className="text-muted">
                        Subscribed, so it stays updated on your other devices too.
                    </span>
                )}

                {report.warnings.map((warning) => (
                    <span key={warning} className="text-warning">
                        {warning}
                    </span>
                ))}

                <button
                    type="button"
                    onClick={() => setReport(null)}
                    className="self-start text-muted underline"
                >
                    Install into another sandbox
                </button>
            </div>
        )
    }

    if (sandboxes.isPending)
        return (
            <p className="rounded-lg border border-border px-3 py-2 text-xs text-muted">
                Checking your sandboxes…
            </p>
        )

    if (options.length < 1)
        return (
            <>
                <button
                    type="button"
                    onClick={() => setCreating(true)}
                    className="flex items-center gap-2 rounded-lg border border-border px-3 py-2 text-xs"
                >
                    <FiPlus className="size-3.5" />
                    Make a sandbox for {summary.app?.name ?? 'this game'} first
                </button>

                {creating && (
                    <SandboxEditor
                        appId={appId}
                        // An app ref's `url` IS the plugin folder's name, and a
                        // sandbox created without one has no game rules at all.
                        appSlug={summary.app?.url?.toLowerCase() ?? null}
                        appName={summary.app?.name ?? null}
                        existing={null}
                        onClose={() => setCreating(false)}
                        onSaved={() => void sandboxes.refetch()}
                    />
                )}
            </>
        )

    return (
        <div className="flex flex-col gap-2 rounded-lg border border-border p-3">
            <span className="flex items-center gap-1.5 text-[11px] font-medium uppercase tracking-wide text-muted">
                <FiLayers className="size-3" />
                Install into a sandbox
            </span>

            <div className="flex items-center gap-2">
                <Select
                    label="Sandbox"
                    value={chosen}
                    options={options}
                    onChange={setSandboxId}
                    className="flex-1"
                    fullWidth
                />

                <button
                    type="button"
                    disabled={busy || !chosen}
                    onClick={() => void run()}
                    className="flex shrink-0 items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-xs font-semibold text-accent-foreground disabled:opacity-50"
                >
                    <FiDownloadCloud className="size-3.5" />
                    {busy ? 'Installing…' : 'Install'}
                </button>
            </div>

            {error && (
                <p className="flex items-start gap-2 text-xs text-danger">
                    <FiAlertTriangle className="mt-0.5 size-3.5 shrink-0" />
                    {error}
                </p>
            )}
        </div>
    )
}
