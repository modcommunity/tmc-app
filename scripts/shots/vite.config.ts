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

export default mergeConfig(review, {
    plugins: [seed],
    cacheDir: new URL('./.vite', import.meta.url).pathname,
    server: { port: Number(process.env.TMC_REVIEW_PORT ?? 1440), hmr: false },
})
