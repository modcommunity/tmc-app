/**
 * The review harness (e2e/review) plus a seed layer for the screenshots:
 * commands that only exist on a real machine (server queries, downloads,
 * library, installed games, playtime) answer with clean demo data from
 * seed.ts instead of empty fakes. API reads still go to the dev backend.
 */
import { mergeConfig, type Plugin } from 'vite'

import review from '../../e2e/review/vite.config'

const seed: Plugin = {
    name: 'shots-seed',
    enforce: 'pre',
    transform(code, id) {
        if (!id.endsWith('/src/main.tsx')) return null
        // After the review mock, so the seed wraps its bridge.
        return code.replace("import '/e2e/review/mock.ts'\n", "import '/e2e/review/mock.ts'\nimport '/scripts/shots/seed.ts'\n")
    },
}

// The review harness sends every avatar asset to the API host, but the
// manifest's model lives on its own host (dev.dotgames.org on dev). This route
// keeps the host: /__shots-asset/<host>/<path>, dev hosts only.
const ASSET_HOSTS = new Set(['tmcdev.net', 'dev.dotgames.org'])
const assets: Plugin = {
    name: 'shots-asset',
    configureServer(server) {
        server.middlewares.use('/__shots-asset/', async (req, res) => {
            const [host, ...rest] = (req.url ?? '').replace(/^\//, '').split('/')
            if (!ASSET_HOSTS.has(host)) {
                res.statusCode = 403
                res.end()
                return
            }
            const r = await fetch(`https://${host}/${rest.join('/')}`).catch(() => null)
            res.statusCode = r?.status ?? 502
            res.setHeader('content-type', r?.headers.get('content-type') ?? 'application/octet-stream')
            res.end(r ? Buffer.from(await r.arrayBuffer()) : undefined)
        })
    },
}

export default mergeConfig(review, {
    plugins: [seed, assets],
    cacheDir: new URL('./.vite', import.meta.url).pathname,
    server: { host: '127.0.0.1', port: Number(process.env.TMC_REVIEW_PORT ?? 1440), hmr: false },
})
