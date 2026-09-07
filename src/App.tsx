import { Component, type ReactNode } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'

import { attachOfflineCache } from '~/lib/api/offline-cache'
import { createHashRouter, Navigate, RouterProvider } from 'react-router-dom'

import { AuthProvider } from '~/lib/auth/provider'
import { SettingsProvider } from '~/lib/settings/provider'
import { LiveQueryProvider } from '~/lib/hooks/use-live-query'
import { LibraryProvider } from '~/lib/library/provider'
import { DownloadsProvider } from '~/lib/downloads/provider'
import Shell from '~/components/shell'
import DropImport from '~/components/drop-import'
import AppsRoute from '~/routes/apps'
import BrowseRoute from '~/routes/browse'
import LibraryRoute from '~/routes/library'
import InstallsRoute from '~/routes/installs'
import SandboxesRoute from '~/routes/sandboxes'
import DownloadsRoute from '~/routes/downloads'
import RconRoute from '~/routes/rcon'
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
            { index: true, element: <Navigate to="/apps" replace /> },
            { path: 'apps', element: <AppsRoute /> },
            { path: 'browse/:kind', element: <BrowseRoute /> },
            { path: 'view/:kind/:id', element: <ViewRoute /> },
            { path: 'library', element: <LibraryRoute /> },
            { path: 'installs', element: <InstallsRoute /> },
            { path: 'sandboxes', element: <SandboxesRoute /> },
            { path: 'sandboxes/:id', element: <SandboxesRoute /> },
            { path: 'downloads', element: <DownloadsRoute /> },
            { path: 'rcon', element: <RconRoute /> },
            { path: 'rcon/:id', element: <RconRoute /> },
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
            { path: '*', element: <Navigate to="/apps" replace /> },
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

/*
 * At module scope, and deliberately not in an effect: restoring is synchronous
 * and has to have happened before the first component mounts, or the first
 * render still sees an empty cache and every screen flashes its spinner before
 * the data it already had appears.
 *
 * Never unsubscribed. The client is a singleton for the life of the process, so
 * there is no teardown to run — the returned function exists for tests.
 */
attachOfflineCache(queryClient)

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
                                {/* Above the router as well: downloads keep
                                    running while somebody browses, and the
                                    queue badge has to be right on every
                                    screen rather than only on its own. */}
                                <DownloadsProvider>
                                    <RouterProvider router={router} />

                                    {/* Above the router as well, and it has to
                                        be: a file dropped during a route change
                                        must still be caught, and the overlay
                                        that says a drop will be accepted has to
                                        exist on every screen rather than on the
                                        one that remembered to draw it. */}
                                    <DropImport />
                                </DownloadsProvider>
                            </LibraryProvider>
                        </LiveQueryProvider>
                    </SettingsProvider>
                </AuthProvider>
            </QueryClientProvider>
        </ErrorBoundary>
    )
}
