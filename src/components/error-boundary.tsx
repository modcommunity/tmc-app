import { Component, useState, type ReactNode } from 'react'
import { FiAlertTriangle, FiCopy, FiRefreshCw } from 'react-icons/fi'

/**
 * The last line of defence against a white screen.
 *
 * A crashed React tree in a browser is a broken tab the user can reload. In an
 * app it is a window with nothing in it and no address bar, so there has to be
 * something on screen that offers a way out, and a way to hand the details to
 * whoever is helping.
 */
function Fallback({ error }: { error: Error }) {
    const [copied, setCopied] = useState(false)

    const details = [
        `GameSpice App crash report`,
        `When: ${new Date().toISOString()}`,
        `Where: ${window.location.hash || '/'}`,
        `Agent: ${navigator.userAgent}`,
        ``,
        `${error.name}: ${error.message}`,
        error.stack ?? '',
    ].join('\n')

    const copy = () => {
        navigator.clipboard
            .writeText(details)
            .then(() => {
                setCopied(true)
                window.setTimeout(() => setCopied(false), 2000)
            })
            .catch(() => undefined)
    }

    return (
        <div className="flex h-screen flex-col items-center justify-center gap-4 p-6 text-center text-foreground">
            <span className="flex size-14 items-center justify-center rounded-full bg-accent/15 text-accent">
                <FiAlertTriangle className="size-7" aria-hidden />
            </span>
            <h1 className="text-xl font-bold">Something went wrong</h1>
            <p className="max-w-md text-sm text-muted">
                The app hit an unexpected error. Reloading usually fixes it; if it
                keeps happening, copy the details and send them to us.
            </p>
            <pre className="selectable max-h-40 w-full max-w-lg overflow-auto rounded-lg border border-border bg-surface p-3 text-left text-xs text-muted">
                {error.message || String(error)}
            </pre>
            <div className="flex gap-2">
                <button
                    type="button"
                    onClick={() => window.location.reload()}
                    className="inline-flex items-center gap-2 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-accent-foreground transition-opacity hover:opacity-90 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
                >
                    <FiRefreshCw className="size-4" aria-hidden />
                    Reload
                </button>
                <button
                    type="button"
                    onClick={copy}
                    className="inline-flex items-center gap-2 rounded-lg border border-border bg-surface px-4 py-2 text-sm transition-colors hover:bg-surface-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
                >
                    <FiCopy className="size-4" aria-hidden />
                    {copied ? 'Copied' : 'Copy details'}
                </button>
            </div>
        </div>
    )
}

export default class ErrorBoundary extends Component<
    { children: ReactNode },
    { error: Error | null }
> {
    state = { error: null as Error | null }

    static getDerivedStateFromError(error: Error) {
        return { error }
    }

    componentDidCatch(error: Error) {
        console.error('[app] render crashed', error)
    }

    render() {
        if (!this.state.error) return this.props.children

        return <Fallback error={this.state.error} />
    }
}
