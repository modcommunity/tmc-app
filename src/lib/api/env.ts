import { useEffect, useState } from 'react'

import { ipc } from '~/lib/ipc/commands'
import type { ApiEnvT } from '~/lib/ipc/schemas'

/**
 * Which site this build is talking to.
 *
 * The value is decided in Rust — compiled in, or taken from `TMC_API_BASE` in
 * the environment of a DEBUG build — and is never settable from here. What this
 * module adds is the display side of that: a build pointed at a dev instance
 * has to say so, or the first screenshot of missing data becomes a bug report
 * against production.
 *
 * Fetched once per process and shared, because it cannot change while the app
 * runs: `api_base()` resolves on first use in Rust and is a `OnceLock`
 * thereafter. Every extra caller would be an IPC round trip for a constant.
 */
let cached: Promise<ApiEnvT | null> | null = null

export function apiEnv(): Promise<ApiEnvT | null> {
    cached ??= ipc.apiEnv().catch(() => {
        // A browser-only `npm run dev` has no Tauri bridge. Null means "no
        // opinion", and every caller treats that as "say nothing" rather than
        // as "production" — claiming prod without having asked is the one
        // wrong answer here.
        return null
    })

    return cached
}

export function useApiEnv(): ApiEnvT | null {
    const [env, setEnv] = useState<ApiEnvT | null>(null)

    useEffect(() => {
        let live = true

        void apiEnv().then((value) => {
            if (live) setEnv(value)
        })

        return () => {
            live = false
        }
    }, [])

    return env
}

/**
 * The site's own URL, for the few links that leave the app to a page the API
 * does not hand us a permalink for.
 *
 * Anything reachable through the API carries its own `webUrl` and should use
 * that. This is for the fixed pages — account security, for one — which would
 * otherwise be hardcoded to production and open the wrong site's settings while
 * the app is signed in to a dev one.
 */
export function webUrl(env: ApiEnvT | null, path: string): string {
    return `${env?.base ?? 'https://moddingcommunity.com'}${path}`
}
