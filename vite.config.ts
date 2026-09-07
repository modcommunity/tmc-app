import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import process from 'node:process'

/** Set by `tauri android dev` / `tauri ios dev` so a device can reach the HMR server. */
const host = process.env.TAURI_DEV_HOST

export default defineConfig(() => ({
    plugins: [react(), tailwindcss()],

    /*
     * Tests run in the NODE environment, not jsdom.
     *
     * What is tested here is the pure logic the app's screens rest on — the URL
     * encoding every browse filter round-trips through, the latency ladder
     * three components read, the offline cache's allow-list. None of it needs a
     * DOM, and the two things that touch `window` stub the handful of methods
     * they use in four lines each. A DOM implementation would be a dependency
     * carried for tests that do not ask for one.
     *
     * There are deliberately no COMPONENT tests. The Rust side holds what would
     * most repay them and already has 460; a React render assertion mostly
     * pins the markup in place, and this app's markup is still moving.
     */
    test: {
        include: ['src/**/*.test.ts'],
        environment: 'node',
    },

    resolve: {
        alias: {
            '~': new URL('./src', import.meta.url).pathname,
        },
    },

    // Don't clear the screen — it hides the Rust compiler's output.
    clearScreen: false,

    server: {
        port: 1420,
        strictPort: true,
        host: host || false,
        hmr: host ? { protocol: 'ws', host, port: 1421 } : undefined,
        watch: { ignored: ['**/src-tauri/**'] },
    },

    /*
     * Safari 13 / Chrome 105 are the floors Tauri's webviews hit on the oldest
     * supported macOS and Android. Targeting them here rather than relying on
     * the default esbuild target is what stops a modern syntax feature from
     * shipping fine on this machine and white-screening on a real phone.
     */
    build: {
        target:
            process.env.TAURI_ENV_PLATFORM === 'windows'
                ? 'chrome105'
                : 'safari13',
        // Vite 8 minifies with oxc; naming `esbuild` here would require it as a
        // separate install for no gain.
        minify: !process.env.TAURI_ENV_DEBUG,
        sourcemap: !!process.env.TAURI_ENV_DEBUG,
    },
}))
