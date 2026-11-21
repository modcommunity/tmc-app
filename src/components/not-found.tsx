import { Link, useLocation, useNavigate } from 'react-router-dom'
import { FiArrowLeft, FiCompass } from 'react-icons/fi'

/** Unknown route: say so, and offer the two ways out. */
export default function NotFound() {
    const { pathname } = useLocation()
    const navigate = useNavigate()

    return (
        <div className="flex min-h-full flex-col items-center justify-center gap-3 p-8 text-center">
            <span className="flex size-14 items-center justify-center rounded-full bg-accent/15 text-accent">
                <FiCompass className="size-7" aria-hidden />
            </span>
            <h1 className="text-xl font-bold">Page not found</h1>
            <p className="selectable max-w-sm text-sm text-muted">
                There is nothing at <code className="break-all">{pathname}</code>.
                It may have moved, or the link may be out of date.
            </p>
            <div className="flex gap-2">
                <button
                    type="button"
                    onClick={() => navigate(-1)}
                    className="inline-flex items-center gap-2 rounded-lg border border-border bg-surface px-4 py-2 text-sm transition-colors hover:bg-surface-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
                >
                    <FiArrowLeft className="size-4" aria-hidden />
                    Go back
                </button>
                <Link
                    to="/play"
                    replace
                    className="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-accent-foreground transition-opacity hover:opacity-90 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
                >
                    Take me to Play
                </Link>
            </div>
        </div>
    )
}
