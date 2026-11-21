import type { ContentKindT } from '~/lib/api/contract'

type LinkInput = {
    kind: ContentKindT
    id: string
    url?: string | null
    webUrl: string
    app?: { id: number; url?: string | null } | null
}

/**
 * Normalise a summary's website permalink.
 *
 * Older website deployments sent permalinks for routes that do not exist
 * (`/blog/view/<id>`, `/user/<name>`, `/m/<id>`, `/s/<id>`, `/s/m/<id>`), which
 * land on 'Page Not Found'. The real routes are `/blog/<url|id>`, `/u/<name>`,
 * and `/<app>/m|s/<..>`, `/<app>/servers/maps/m/<name>`. Rewrites only those
 * legacy shapes, so a correct server-supplied URL passes through untouched.
 */
export function normalizeWebUrl(item: LinkInput): string {
    let u: URL

    try {
        u = new URL(item.webUrl)
    } catch {
        return item.webUrl
    }

    const path = u.pathname
    const seg = encodeURIComponent(item.url ?? item.id)
    const app = item.app ? (item.app.url ?? String(item.app.id)) : null
    let next: string | null = null

    switch (item.kind) {
        case 'article':
            if (path.startsWith('/blog/view/')) next = `/blog/${seg}`
            break
        case 'user':
            if (path.startsWith('/user/')) next = `/u/${seg}`
            break
        case 'mod':
            if (app && /^\/m\/[^/]+$/.test(path))
                next = `/${encodeURIComponent(app)}/m/${encodeURIComponent(item.id)}`
            break
        case 'server':
            if (app && /^\/s\/[^/]+$/.test(path))
                next = `/${encodeURIComponent(app)}/s/${seg}`
            break
        case 'serverMap':
            if (app && /^\/s\/m\/[^/]+$/.test(path))
                next = `/${encodeURIComponent(app)}/servers/maps/m/${seg}`
            break
        default:
            break
    }

    if (next === null) return item.webUrl

    u.pathname = next

    return u.toString()
}
