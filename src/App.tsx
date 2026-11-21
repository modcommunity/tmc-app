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
import ErrorBoundary from '~/components/error-boundary'
import NotFound from '~/components/not-found'
import PlayRoute from '~/routes/play'
import DiscoverRoute from '~/routes/discover'
import StoresRoute from '~/routes/stores'
import ConsoleRoute from '~/routes/console'
import AppsRoute from '~/routes/apps'
import GameRoute from '~/routes/game'
import FriendsRoute from '~/routes/friends'
import BrowseRoute from '~/routes/browse'
import JoinRoute from '~/routes/join'
import LibraryRoute from '~/routes/library'
import InstallsRoute from '~/routes/installs'
import SandboxesRoute from '~/routes/sandboxes'
import DownloadsRoute from '~/routes/downloads'
import RconRoute from '~/routes/rcon'
import ViewRoute from '~/routes/view'
import GameHubRoute from '~/routes/game-hub'
import WorkshopRoute from '~/routes/workshop'
import GameSpiceAccountRoute from '~/routes/gamespice-account'
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
            { index: true, element: <Navigate to="/play" replace /> },
            { path: 'play', element: <PlayRoute /> },
            { path: 'games', element: <Navigate to="/play" replace /> },
            { path: 'games/:slug', element: <GameHubRoute /> },
            { path: 'games/:slug/avatar', lazy: () => import('~/routes/avatar-games').then((m) => ({ Component: m.default })) },
            { path: 'avatar', lazy: () => import('~/routes/avatar-member').then((m) => ({ Component: m.default })) },
            { path: 'workshop', element: <WorkshopRoute /> },
            { path: 'me', element: <GameSpiceAccountRoute /> },
            { path: 'store', lazy: () => import('~/routes/store').then((m) => ({ Component: m.StoreDirectoryRoute })) },
            { path: 'store/:appId', lazy: () => import('~/routes/store').then((m) => ({ Component: m.GameStoreRoute })) },
            { path: 'stats', lazy: () => import('~/routes/stats').then((m) => ({ Component: m.StatsRoute })) },
            { path: 'stats/:appId', lazy: () => import('~/routes/stats').then((m) => ({ Component: m.GameStatsRoute })) },
            { path: 'discover', element: <DiscoverRoute /> },
            { path: 'apps', element: <AppsRoute /> },
            { path: 'apps/:id', element: <GameRoute /> },
            { path: 'friends', element: <FriendsRoute /> },
            { path: 'browse/:kind', element: <BrowseRoute /> },
            { path: 'view/:kind/:id', element: <ViewRoute /> },
            { path: 'join', element: <JoinRoute /> },
            { path: 'library', element: <LibraryRoute /> },
            { path: 'installs', element: <InstallsRoute /> },
            { path: 'sandboxes', element: <SandboxesRoute /> },
            { path: 'sandboxes/:id', element: <SandboxesRoute /> },
            { path: 'downloads', element: <DownloadsRoute /> },
            { path: 'stores', element: <StoresRoute /> },
            { path: 'console', element: <ConsoleRoute /> },
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
            { path: '*', element: <NotFound /> },
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
 * A monitor window is a second document with its own JS context. It gets a
 * lean tree: no library sync, downloads or drop handling, which belong to the
 * main window and would otherwise run twice.
 */
const monitorRouter = createHashRouter([
    {
        path: '/monitor/server/:id',
        lazy: () => import('~/routes/monitor').then((m) => ({ Component: m.default })),
    },
    {
        path: '/monitor/addr',
        lazy: () => import('~/routes/monitor').then((m) => ({ Component: m.default })),
    },
    { path: '*', element: <Navigate to="/" replace /> },
])

const isMonitorWindow = window.location.hash.startsWith('#/monitor')

// The main window obeys a monitor window's "open the server page" request.
if (!isMonitorWindow && '__TAURI_INTERNALS__' in window) {
    void import('@tauri-apps/api/event').then(({ listen }) =>
        listen<{ path: string }>('monitor:navigate', (e) => {
            if (typeof e.payload?.path === 'string' && e.payload.path.startsWith('/'))
                void router.navigate(e.payload.path)
        })
    )
}

function MonitorApp() {
    return (
        <ErrorBoundary>
            <QueryClientProvider client={queryClient}>
                <AuthProvider>
                    <SettingsProvider>
                        <RouterProvider router={monitorRouter} />
                    </SettingsProvider>
                </AuthProvider>
            </QueryClientProvider>
        </ErrorBoundary>
    )
}

export default function App() {
    if (isMonitorWindow) return <MonitorApp />

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
