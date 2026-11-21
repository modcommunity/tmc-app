import { Link } from 'react-router-dom'
import { FiAlertTriangle, FiRefreshCw } from 'react-icons/fi'

/**
 * A failed load, said the same way on every screen: what failed, the reason
 * the backend or bridge gave, and one button that retries. Replaces the bare
 * red `HTTP 502` lines some routes printed with no way out.
 */
export function LoadError({
    title,
    detail,
    onRetry,
    retrying = false,
}: {
    title: string
    detail?: string
    onRetry?: () => void
    retrying?: boolean
}) {
    return (
        <div
            role="alert"
            className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-danger/60 bg-danger/5 p-4"
        >
            <div className="flex min-w-0 items-start gap-2.5">
                <FiAlertTriangle className="mt-0.5 size-4 shrink-0 text-danger" aria-hidden />
                <div className="min-w-0">
                    <p className="text-sm text-danger">{title}</p>
                    {detail && <p className="mt-0.5 break-words text-xs text-muted">{detail}</p>}
                </div>
            </div>
            {onRetry && (
                <button
                    type="button"
                    onClick={onRetry}
                    disabled={retrying}
                    className="flex items-center gap-1.5 rounded-md border border-border px-3 py-1 text-xs text-foreground transition-colors hover:bg-surface-hover disabled:opacity-60"
                >
                    <FiRefreshCw className={`size-3 ${retrying ? 'animate-spin' : ''}`} aria-hidden />
                    Try again
                </button>
            )}
        </div>
    )
}

/** The signed-out state of a screen that needs an account: a sentence and the way in. */
export function SignInPrompt({ children }: { children: React.ReactNode }) {
    return (
        <div className="mx-auto max-w-md rounded-xl border border-border p-6 text-center">
            <p className="text-sm text-muted">{children}</p>
            <Link
                to="/account"
                className="mt-3 inline-block rounded-lg bg-accent px-4 py-2 text-sm text-accent-foreground"
            >
                Sign in
            </Link>
        </div>
    )
}
