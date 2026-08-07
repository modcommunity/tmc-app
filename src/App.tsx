import { Component, type ReactNode } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { createHashRouter, Navigate, RouterProvider } from 'react-router-dom'

import { AuthProvider } from '~/lib/auth/provider'
import { SettingsProvider } from '~/lib/settings/provider'
import { LiveQueryProvider } from '~/lib/hooks/use-live-query'
import { LibraryProvider } from '~/lib/library/provider'
import Shell from '~/components/shell'
import BrowseRoute from '~/routes/browse'
import LibraryRoute from '~/routes/library'
import InstallsRoute from '~/routes/installs'
import ViewRoute from '~/routes/view'
import AccountRoute from '~/routes/account'
import SettingsLayout from '~/routes/settings'
import AppSettingsRoute from '~/routes/settings/app'
import AccountSettingsRoute from '~/routes/settings/account'
import GamesRoute from '~/routes/settings/games'
import PluginsRoute from '~/routes/settings/plugins'
import LoggingRoute from '~/routes/settings/logging'

/**
 * Hash routing, not browser routing.
 *
 * A Tauri build is a static bundle served from a custom protocol; a deep path
 * like `/view/mod/12` has no server to answer it, so a reload or a restored
 * window would 404. The hash keeps every route inside one document, which is
 * also what makes the same build work unchanged on all five targets.
 */
const router = createHashRouter([
    {
        path: '/',
        element: <Shell />,
        children: [
            { index: true, element: <Navigate to="/browse/mod" replace /> },
            { path: 'browse/:kind', element: <BrowseRoute /> },
            { path: 'view/:kind/:id', element: <ViewRoute /> },
            { path: 'library', element: <LibraryRoute /> },
            { path: 'installs', element: <InstallsRoute /> },
            { path: 'account', element: <AccountRoute /> },
            {
                path: 'settings',
                element: <SettingsLayout />,
                children: [
                    { index: true, element: <AppSettingsRoute /> },
                    { path: 'account', element: <AccountSettingsRoute /> },
                    { path: 'games', element: <GamesRoute /> },
                    { path: 'plugins', element: <PluginsRoute /> },
                    { path: 'logging', element: <LoggingRoute /> },
                ],
            },
            { path: '*', element: <Navigate to="/browse/mod" replace /> },
        ],
    },
])

const queryClient = new QueryClient({
    defaultOptions: {
        queries: {
            /*
             * A desktop app is left open for days, and a phone suspends and
             * resumes constantly. Refetching on every focus would make both
             * feel busy for no benefit — the data here changes on the scale of
             * minutes, and every screen has a pull-to-refresh path anyway.
             */
            refetchOnWindowFocus: false,
            staleTime: 60 * 1000,
            retry: 1,
        },
    },
})

/**
 * The last line of defence against a white screen.
 *
 * A crashed React tree in a browser is a broken tab the user can reload. In an
 * app it is a window with nothing in it and no address bar, so there has to be
 * something on screen that offers a way out.
 */
class ErrorBoundary extends Component<
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

        return (
            <div className="flex h-screen flex-col items-center justify-center gap-3 p-6 text-center">
                <h1 className="text-lg font-bold">Something broke</h1>
                <p className="selectable max-w-md text-sm text-muted">
                    {this.state.error.message}
                </p>
                <button
                    type="button"
                    onClick={() => window.location.reload()}
                    className="rounded-lg bg-accent px-4 py-2 text-sm text-accent-foreground"
                >
                    Reload the app
                </button>
            </div>
        )
    }
}

export default function App() {
    return (
        <ErrorBoundary>
            <QueryClientProvider client={queryClient}>
                <AuthProvider>
                    <SettingsProvider>
                        {/* Above the router: the live registry outlives a route
                            change, so scrolling back to the browser keeps the
                            latency history it already gathered. */}
                        <LiveQueryProvider>
                            {/* Above the router too: the sync loop has to run
                                whether or not a library screen is mounted —
                                that is the whole point of a subscription made
                                in a browser reaching this device. */}
                            <LibraryProvider>
                                <RouterProvider router={router} />
                            </LibraryProvider>
                        </LiveQueryProvider>
                    </SettingsProvider>
                </AuthProvider>
            </QueryClientProvider>
        </ErrorBoundary>
    )
}
