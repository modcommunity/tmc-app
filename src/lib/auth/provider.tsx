import {
    createContext,
    useCallback,
    useContext,
    useEffect,
    useMemo,
    useRef,
    useState,
    type ReactNode,
} from 'react'
import { listen } from '@tauri-apps/api/event'
import { openUrl } from '@tauri-apps/plugin-opener'

import { ipc } from '~/lib/ipc/commands'
import { isIpcError } from '~/lib/ipc'
import type { PendingLoginT, SessionUserT } from '~/lib/ipc/schemas'

/**
 * The device-grant login, as the UI experiences it.
 *
 * The whole credential half lives in Rust; this provider only ever holds a
 * status and a user profile. What it owns is the *polling*, which is worth
 * being careful about:
 *
 *   * The interval is whatever the server asked for, not a number picked here.
 *   * Polling stops when the grant expires, when the user cancels, and when the
 *     window is hidden — a backgrounded app on a phone should not be waking a
 *     radio every five seconds.
 *   * The `tmc://` deep link fires an immediate poll, so approving in the
 *     browser feels instant instead of taking up to one interval.
 */

type Status = 'loading' | 'signedOut' | 'awaitingApproval' | 'signedIn'

type AuthContextT = {
    status: Status
    user: SessionUserT | null
    pending: PendingLoginT | null
    /** Set when the last attempt failed, for the login screen to show. */
    error: string | null
    signIn: () => Promise<void>
    cancel: () => Promise<void>
    signOut: () => Promise<void>
}

const AuthContext = createContext<AuthContextT | null>(null)

export function AuthProvider({ children }: { children: ReactNode }) {
    const [status, setStatus] = useState<Status>('loading')
    const [user, setUser] = useState<SessionUserT | null>(null)
    const [pending, setPending] = useState<PendingLoginT | null>(null)
    const [error, setError] = useState<string | null>(null)

    /** Held in a ref so the poll loop can read it without re-subscribing. */
    const pollTimer = useRef<number | null>(null)

    const stopPolling = useCallback(() => {
        if (pollTimer.current !== null) {
            window.clearInterval(pollTimer.current)
            pollTimer.current = null
        }
    }, [])

    const poll = useCallback(async () => {
        try {
            const result = await ipc.authPoll()

            if (result.status === 'signedIn') {
                stopPolling()
                setPending(null)
                setUser(result.user)
                setStatus('signedIn')
                return
            }

            if (result.status === 'denied' || result.status === 'expired') {
                stopPolling()
                setPending(null)
                setStatus('signedOut')
                setError(
                    result.status === 'denied'
                        ? 'That sign-in was rejected in the browser.'
                        : 'That sign-in expired. Try again.'
                )
            }
        } catch (err) {
            // A transient network failure mid-poll is not a reason to abandon
            // the login — the grant is still live on the server.
            console.warn('[auth] poll failed', err)
        }
    }, [stopPolling])

    const startPolling = useCallback(
        (intervalSec: number) => {
            stopPolling()

            pollTimer.current = window.setInterval(
                () => void poll(),
                Math.max(2, intervalSec) * 1000
            )
        },
        [poll, stopPolling]
    )

    // Initial state, once.
    useEffect(() => {
        let cancelled = false

        void (async () => {
            try {
                const snapshot = await ipc.session()

                if (cancelled) return

                if (!snapshot.signedIn) {
                    setStatus('signedOut')
                    return
                }

                setUser(snapshot.user)
                setStatus('signedIn')
            } catch {
                if (!cancelled) setStatus('signedOut')
            }
        })()

        return () => {
            cancelled = true
        }
    }, [])

    // The deep-link accelerator. The event carries nothing — it only means
    // "the browser just came back", which is the cue to poll now.
    useEffect(() => {
        const unlisten = listen('tmc://auth-return', () => void poll())

        return () => {
            void unlisten.then((fn) => fn())
        }
    }, [poll])

    // Do not poll a backgrounded app.
    useEffect(() => {
        const onVisibility = () => {
            if (document.hidden) {
                stopPolling()
                return
            }

            if (pending) {
                void poll()
                startPolling(pending.interval)
            }
        }

        document.addEventListener('visibilitychange', onVisibility)

        return () => document.removeEventListener('visibilitychange', onVisibility)
    }, [pending, poll, startPolling, stopPolling])

    useEffect(() => stopPolling, [stopPolling])

    const signIn = useCallback(async () => {
        setError(null)

        try {
            const login = await ipc.authBegin()

            setPending(login)
            setStatus('awaitingApproval')
            startPolling(login.interval)

            /*
             * The SYSTEM browser, never an embedded webview. A webview login
             * hides the address bar and the password manager, which is exactly
             * what makes it indistinguishable from a phishing page — and it is
             * why every major provider now refuses to authenticate inside one.
             */
            await openUrl(login.verificationUriComplete)
        } catch (err) {
            setStatus('signedOut')
            setPending(null)
            setError(
                isIpcError(err) ? err.message : 'Could not start the sign-in.'
            )
        }
    }, [startPolling])

    const cancel = useCallback(async () => {
        stopPolling()
        await ipc.authCancel()
        setPending(null)
        setStatus('signedOut')
    }, [stopPolling])

    const signOut = useCallback(async () => {
        stopPolling()
        await ipc.authSignOut()
        setUser(null)
        setPending(null)
        setStatus('signedOut')
    }, [stopPolling])

    const value = useMemo<AuthContextT>(
        () => ({ status, user, pending, error, signIn, cancel, signOut }),
        [status, user, pending, error, signIn, cancel, signOut]
    )

    return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>
}

export function useAuth(): AuthContextT {
    const ctx = useContext(AuthContext)

    if (!ctx) throw new Error('useAuth must be used inside <AuthProvider>')

    return ctx
}
