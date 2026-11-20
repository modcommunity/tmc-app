/**
 * Review harness: the real web UI with a mocked Tauri bridge (mock.ts), every
 * API read proxied to a dev backend (TMC_REVIEW_API, default tmcdev.net) with a
 * dev token from TMC_REVIEW_TOKEN. The token is added here, server-side, so it
 * never reaches the page. Never point this at prod with a prod token.
 */
import { mergeConfig, type Plugin, type UserConfig } from 'vite'

import base from '../../vite.config'

const API = process.env.TMC_REVIEW_API ?? 'https://tmcdev.net'
const TOKEN = process.env.TMC_REVIEW_TOKEN ?? ''

const auth = TOKEN ? { Authorization: `Bearer ${TOKEN}` } : undefined

/** Lets the mock see the schema a `call` expects, to fake unknown commands. */
const reviewHooks: Plugin = {
    name: 'review-hooks',
    enforce: 'pre',
    transform(code, id) {
        // Imported first so the bridge exists before any module calls invoke.
        if (id.endsWith('/src/main.tsx')) return `import '/e2e/review/mock.ts'\n${code}`
        if (!id.endsWith('/src/lib/ipc/index.ts')) return null

        return code.replace(
            'raw = await invoke(command, args)',
            'raw = await ((globalThis as { __reviewSchema?: unknown }).__reviewSchema = schema, invoke(command, args))'
        )
    },
}

export default mergeConfig((base as (env: unknown) => UserConfig)({ command: 'serve', mode: 'development' }), {
    plugins: [reviewHooks],
    root: new URL('../..', import.meta.url).pathname,
    server: {
        port: Number(process.env.TMC_REVIEW_PORT ?? 1431),
        strictPort: true,
        proxy: {
            '/__api': {
                target: API,
                changeOrigin: true,
                headers: auth,
                rewrite: (p: string) => p.replace(/^\/__api/, '/api/app/v1'),
            },
            '/__avatar': {
                target: API,
                changeOrigin: true,
                headers: auth,
                rewrite: (p: string) => p.replace(/^\/__avatar/, '/api/avatar/v1'),
            },
            '/__asset': {
                target: API,
                changeOrigin: true,
                rewrite: (p: string) => p.replace(/^\/__asset/, ''),
            },
        },
    },
})
