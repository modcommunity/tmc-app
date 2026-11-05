import { useEffect } from 'react'
import { useNavigate } from 'react-router-dom'
import { z } from 'zod'

import { call, subscribe } from '~/lib/ipc'

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
 *
 * THE JOIN FLOW
 * -------------
 * `tmc://play/<host>:<port>` is what the website's Join button produces. It is
 * the only link in this set whose payload is not an id of ours, and it is still
 * only a screen: `/join` resolves the address through `/servers/lookup`, shows
 * which game is running there with the latency measured from this device, and
 * offers the launch modes this machine actually has.
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
    z.object({
        type: z.literal('play'),
        host: z.string(),
        port: z.number().nullish(),
    }),
    z.object({
        type: z.literal('playApp'),
        app: z.string(),
        solo: z.string().nullish(),
        soloVersion: z.string().nullish(),
    }),
    z.object({ type: z.literal('auth') }),
])

export function useDeepLink() {
    const navigate = useNavigate()

    useEffect(() => {
        let stop: (() => void) | null = null
        let live = true

        const handle = (link: z.infer<typeof OpenSchema>) => {
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
                case 'play': {
                    /*
                     * A SCREEN, not a join. The link carries an address and the
                     * page looks it up, shows what is running there and puts a
                     * button under it — see `routes/join`. Any web page can
                     * navigate to a custom scheme without a click, so a version
                     * that connected on arrival would be a remote primitive for
                     * making this machine dial an address a stranger chose.
                     */
                    const query = new URLSearchParams({ host: link.host })

                    if (link.port) query.set('port', String(link.port))

                    void navigate(`/join?${query.toString()}`)

                    return
                }
                case 'playApp':
                    /*
                     * The serverless half. `?start` rings the Play control the
                     * same way `?install` rings the install button, rather than
                     * leaving somebody on a page wondering what the link did.
                     */
                    {
                        const query = new URLSearchParams({
                            app: link.app,
                            start: '1',
                        })

                        if (link.solo) query.set('solo', link.solo)

                        if (link.solo && link.soloVersion)
                            query.set('soloVersion', link.soloVersion)

                        void navigate(`/apps?${query.toString()}`)
                    }

                    return
                case 'auth':
                    // Handled by the auth provider's own listener; nothing to
                    // navigate to.
                    return
            }
        }

        // Subscribe first, then ask for the link the app was started with: a
        // cold start delivers it before this listener exists.
        void subscribe('tmc://open', OpenSchema, handle)
            .then(async (unlisten) => {
                if (live) stop = unlisten
                else unlisten()

                const first = await call(
                    'deep_link_take',
                    OpenSchema.nullable()
                )

                if (live && first) handle(first)
            })
            .catch((err) => console.error('[deeplink] could not subscribe', err))

        return () => {
            live = false
            stop?.()
        }
    }, [navigate])
}
