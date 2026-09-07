import { useEffect } from 'react'
import { useNavigate } from 'react-router-dom'
import { z } from 'zod'

import { subscribe } from '~/lib/ipc'

/**
 * What a `tmc://` link does once Rust has decided it is one.
 *
 * **It navigates. It never acts.** Rust has already refused anything that is
 * not a recognised, bounded shape (see `tmc_core::deeplink`); this turns what
 * survived into a route. The user then presses the button on the page that
 * opens, or does not — which is the whole design, because any program on the
 * machine can claim a custom scheme and any web page can navigate to one
 * without a click.
 *
 * THE GUEST INSTALL FLOW
 * ----------------------
 * Somebody not signed in presses "Install with the app" on the website. The
 * browser opens `tmc://install/mod/1234`, the app comes to the front on the
 * mod's page, and the install button is right there. A signed-in user gets a
 * subscription instead — it follows them to their other devices and keeps the
 * mod updated — but a guest has no account to hang one on.
 *
 * `?install` rides along so the page can scroll to and highlight the button
 * rather than leaving somebody on a page wondering what just happened.
 */

const OpenSchema = z.discriminatedUnion('type', [
    z.object({
        type: z.literal('install'),
        kind: z.string(),
        id: z.number(),
    }),
    z.object({
        type: z.literal('view'),
        kind: z.string(),
        id: z.number(),
    }),
    z.object({ type: z.literal('sandbox'), id: z.number() }),
    z.object({ type: z.literal('auth') }),
])

export function useDeepLink() {
    const navigate = useNavigate()

    useEffect(() => {
        let stop: (() => void) | null = null
        let live = true

        void subscribe('tmc://open', OpenSchema, (link) => {
            switch (link.type) {
                case 'install':
                    void navigate(`/view/${link.kind}/${link.id}?install=1`)

                    return
                case 'view':
                    void navigate(`/view/${link.kind}/${link.id}`)

                    return
                case 'sandbox':
                    void navigate(`/sandboxes/${link.id}`)

                    return
                case 'auth':
                    // Handled by the auth provider's own listener; nothing to
                    // navigate to.
                    return
            }
        })
            .then((unlisten) => {
                if (live) stop = unlisten
                else unlisten()
            })
            .catch((err) => console.error('[deeplink] could not subscribe', err))

        return () => {
            live = false
            stop?.()
        }
    }, [navigate])
}
