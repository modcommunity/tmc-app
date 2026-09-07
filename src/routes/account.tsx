import { Button } from '@modcommunity/shared'
import { FiCheck, FiCopy, FiExternalLink } from 'react-icons/fi'
import { openUrl } from '@tauri-apps/plugin-opener'
import { useState } from 'react'
import { Navigate } from 'react-router-dom'

import { useApiEnv } from '~/lib/api/env'
import { useAuth } from '~/lib/auth/provider'

/**
 * Sign-in, and what the user sees while the browser half is happening.
 *
 * The code is shown even though `verificationUriComplete` pre-fills it: the
 * browser may open on a different profile, a different device, or not at all,
 * and a user staring at a spinner with no code has no way to finish. It is also
 * how the flow degrades gracefully on a locked-down machine where the app
 * cannot launch a browser.
 */
export default function AccountRoute() {
    const { status, pending, error, signIn, cancel } = useAuth()
    const [copied, setCopied] = useState(false)
    const env = useApiEnv()

    const site = (env?.base ?? 'https://moddingcommunity.com').replace(
        /^https?:\/\//,
        ''
    )

    if (status === 'signedIn') return <Navigate to="/settings/account" replace />

    if (status === 'loading') return <Centered>Checking your session…</Centered>

    if (status === 'awaitingApproval' && pending)
        return (
            <Centered>
                <div className="flex w-full max-w-sm flex-col items-center gap-4">
                    <h1 className="text-lg font-bold">Approve this device</h1>

                    <p className="text-sm text-muted">
                        Your browser should have opened. Confirm the code below to
                        finish signing in.
                    </p>

                    <button
                        type="button"
                        onClick={() => {
                            void navigator.clipboard
                                .writeText(pending.userCode)
                                .then(() => {
                                    setCopied(true)
                                    window.setTimeout(() => setCopied(false), 2000)
                                })
                        }}
                        className="selectable flex items-center gap-3 rounded-xl border border-border bg-surface px-5 py-3 font-mono text-2xl tracking-[0.2em]"
                    >
                        {pending.userCode}
                        {copied ? (
                            <FiCheck className="size-4 text-success" />
                        ) : (
                            <FiCopy className="size-4 text-muted" />
                        )}
                    </button>

                    <button
                        type="button"
                        onClick={() =>
                            void openUrl(pending.verificationUriComplete)
                        }
                        className="flex items-center gap-1.5 text-xs text-accent underline"
                    >
                        <FiExternalLink className="size-3" />
                        Open the page again
                    </button>

                    {/*
                     * The address in full, because "open the page" is not
                     * always something the app can do: a locked-down machine
                     * may refuse to launch a browser, and the hand-off is
                     * scoped to https — so a dev build pointed at a plain-http
                     * local site cannot open it either. A code with no address
                     * to type it into is a dead end.
                     */}
                    <p className="selectable break-all text-center font-mono text-[10px] text-muted">
                        {pending.verificationUri}
                    </p>

                    <p className="text-center text-xs text-muted">
                        Waiting for approval… This code expires in about{' '}
                        {Math.round(pending.expiresIn / 60)} minutes.
                    </p>

                    <Button btnType="secondary" onClick={() => void cancel()}>
                        Cancel
                    </Button>
                </div>
            </Centered>
        )

    return (
        <Centered>
            <div className="flex w-full max-w-sm flex-col items-center gap-4 text-center">
                <h1 className="text-lg font-bold">Sign in to TMC</h1>

                {/*
                 * The host is named from the base this build actually talks to
                 * rather than written into the sentence. Telling someone they
                 * are about to sign in on moddingcommunity.com while the button
                 * opens a dev instance teaches them not to read the line —
                 * which is the one habit this whole flow depends on.
                 */}
                <p className="text-sm text-muted">
                    You will finish signing in on {site} in your own browser. The
                    app never sees your password.
                </p>

                {error && <p className="text-sm text-danger">{error}</p>}

                <Button btnType="primary" onClick={() => void signIn()}>
                    Continue in browser
                </Button>

                <p className="text-xs text-muted">
                    You can browse mods, assets and servers without signing in.
                </p>
            </div>
        </Centered>
    )
}

function Centered({ children }: { children: React.ReactNode }) {
    return (
        <div className="flex h-full items-center justify-center p-6">
            {children}
        </div>
    )
}
