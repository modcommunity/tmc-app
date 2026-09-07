import { openUrl } from '@tauri-apps/plugin-opener'

import type { ContentKindT } from '~/lib/api/contract'

/**
 * Content kinds the app hands to the system browser instead of rendering.
 *
 * WHY ARTICLES ARE NOT RENDERED IN-APP
 * ------------------------------------
 * Every other kind's body is a mod description: prose, a list, a code block, a
 * table of config keys. `components/markdown.tsx` renders that faithfully
 * because that is the whole vocabulary in play.
 *
 * An article is written in the website's editor, and its body is laid out for a
 * 1200px content column with embedded media, callouts, footnotes and the site's
 * own typography. Passing it through the app's markdown subset strips exactly
 * the parts that carry the layout and leaves an unstyled wall of text — which
 * is the "looks weird" this list exists to fix. The honest answer is that the
 * article already has a renderer, on the web, and the app should send the
 * reader there rather than ship a worse second one.
 *
 * The app still owns the article's CARD — title, banner, author, stats — so
 * browsing and searching stay in the app. Only the body leaves.
 */
const EXTERNAL_KINDS = new Set<ContentKindT>(['article'])

export function opensExternally(kind: ContentKindT): boolean {
    return EXTERNAL_KINDS.has(kind)
}

/**
 * Open a permalink in the user's real browser.
 *
 * `openUrl` goes through `tauri-plugin-opener`, whose capability is scoped to
 * `https://*` — so this cannot be talked into launching a `file://` path or a
 * custom scheme registered by something else on the machine, however the URL
 * got into the payload.
 */
export async function openExternally(item: { webUrl: string }): Promise<void> {
    await openUrl(item.webUrl)
}
