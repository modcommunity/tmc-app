import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import process from 'node:process'

/** Set by `tauri android dev` / `tauri ios dev` so a device can reach the HMR server. */
const host = process.env.TAURI_DEV_HOST

export default defineConfig(() => ({
    plugins: [react(), tailwindcss()],

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
