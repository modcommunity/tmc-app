import {
    createContext,
    useCallback,
    useContext,
    useEffect,
    useMemo,
    useState,
    type ReactNode,
} from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import { api } from '~/lib/api/client'
import { ipc } from '~/lib/ipc/commands'
import type { AppSettingsT } from '~/lib/ipc/schemas'
import type { UserSettingsT } from '~/lib/api/contract'
import { useAuth } from '~/lib/auth/provider'

/**
 * The two halves of "settings", kept deliberately apart.
 *
 *   * **App settings** are local to this install and live in Rust
 *     (`settings.json`). Theme, scale, game directories, logging verbosity —
 *     things that describe THIS machine and would be wrong to apply elsewhere.
 *   * **User settings** are the account's, and live on the website. Notification
 *     preferences, locale, timezone — things that should follow the user to a
 *     new device.
 *
 * They are exposed through one hook because a settings screen needs both, but
 * they are never merged: each has its own writer, its own failure mode (a local
 * write cannot fail because the network is down; a synced write can), and its
 * own audience.
 */

type SettingsContextT = {
    app: AppSettingsT | null
    user: UserSettingsT | null
    /** True while the account's settings are being fetched or are unavailable. */
    userPending: boolean
    setApp: (patch: Partial<AppSettingsT>) => Promise<void>
    setUser: (patch: Partial<UserSettingsT>) => Promise<void>
    resetApp: () => Promise<void>
}

const SettingsContext = createContext<SettingsContextT | null>(null)

export function SettingsProvider({ children }: { children: ReactNode }) {
    const { status } = useAuth()
    const queryClient = useQueryClient()

    const [app, setAppState] = useState<AppSettingsT | null>(null)

    useEffect(() => {
        void ipc.settingsGet().then(setAppState).catch(() => setAppState(null))
    }, [])

    const me = useQuery({
        queryKey: ['me'],
        queryFn: () => api.me(),
        enabled: status === 'signedIn',
        staleTime: 5 * 60 * 1000,
    })

    const setApp = useCallback(async (patch: Partial<AppSettingsT>) => {
        setAppState(await ipc.settingsPatch(patch))
    }, [])

    const resetApp = useCallback(async () => {
        setAppState(await ipc.settingsReset())
    }, [])

    const setUser = useCallback(
        async (patch: Partial<UserSettingsT>) => {
            const next = await api.updateSettings(patch)

            // Write the server's answer back rather than the optimistic patch:
            // the server clamps and normalises, and showing the unclamped value
            // would leave the UI disagreeing with the account until a refetch.
            queryClient.setQueryData(['me'], (old: unknown) =>
                old && typeof old === 'object'
                    ? { ...(old), settings: next }
                    : old
            )
        },
        [queryClient]
    )

    /*
     * Theme and scale are applied to the document here rather than in a
     * component, so they survive every route change and are correct on first
     * paint after a reload.
     */
    useEffect(() => {
        if (!app) return

        const root = document.documentElement

        root.style.setProperty('--app-scale', String(app.uiScale))

        const applyBase = (base: 'light' | 'dark') => {
            root.classList.toggle('dark', base === 'dark')
            root.dataset.theme = base
        }

        /*
         * Clear any plugin tokens FIRST, on every path.
         *
         * Doing it per-branch missed the `system` one, so switching from a
         * plugin theme back to "match the system" left the plugin's colours
         * painted over the built-in palette — a theme the user had just turned
         * off, with no way to shift it but a restart.
         */
        clearPluginTokens(root)

        if (app.theme === 'system') {
            const media = window.matchMedia('(prefers-color-scheme: dark)')

            applyBase(media.matches ? 'dark' : 'light')

            const onChange = (e: MediaQueryListEvent) =>
                applyBase(e.matches ? 'dark' : 'light')

            media.addEventListener('change', onChange)

            return () => media.removeEventListener('change', onChange)
        }

        if (app.theme === 'light' || app.theme === 'dark') {
            applyBase(app.theme)
            return
        }

        // A plugin theme. Tokens are validated in Rust before they are ever
        // stored, so they can be written straight onto the root.
        const id = app.theme.slice('plugin:'.length)

        void ipc
            .pluginTheme(id)
            .then((theme) => {
                if (!theme) {
                    applyBase('dark')
                    return
                }

                applyBase(theme.base === 'light' ? 'light' : 'dark')

                for (const [token, value] of Object.entries(theme.tokens)) {
                    root.style.setProperty(token, value)
                    appliedTokens.add(token)
                }
            })
            .catch(() => applyBase('dark'))

        return
    }, [app])

    const value = useMemo<SettingsContextT>(
        () => ({
            app,
            user: me.data?.settings ?? null,
            userPending: status === 'signedIn' && !me.data,
            setApp,
            setUser,
            resetApp,
        }),
        [app, me.data, status, setApp, setUser, resetApp]
    )

    return (
        <SettingsContext.Provider value={value}>
            {children}
        </SettingsContext.Provider>
    )
}

/** Tokens a plugin theme set, so switching themes does not leave them behind. */
const appliedTokens = new Set<string>()

function clearPluginTokens(root: HTMLElement) {
    for (const token of appliedTokens) root.style.removeProperty(token)

    appliedTokens.clear()
}

export function useSettings(): SettingsContextT {
    const ctx = useContext(SettingsContext)

    if (!ctx) throw new Error('useSettings must be used inside <SettingsProvider>')

    return ctx
}
