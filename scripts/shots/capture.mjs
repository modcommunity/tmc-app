/**
 * Shoot every enabled screen of screens.mjs in light and dark, 1600x1000, into
 * out/raw/<slot>-<theme>.png, from the shots harness (vite.config.ts here).
 *
 *   node capture.mjs [--only slot,slot] [--theme light|dark] [--port 1440]
 *
 * The browser is local Chromium (SwiftShader) unless GPU_CDP_URL is set, which
 * `gpu.sh cdp-run --serve <port> -- node capture.mjs` does: then a real GPU on
 * a staging box renders, which the 3D avatar needs. `--gpu-only` shoots just
 * the screens marked gpu, `--no-gpu` skips them.
 */
import { createRequire } from 'node:module'
import { mkdirSync, writeFileSync } from 'node:fs'

import { screens, themes, SIZE } from './screens.mjs'

const arg = (name, dflt) => {
    const i = process.argv.indexOf(`--${name}`)
    return i < 0 ? dflt : process.argv[i + 1]
}
const flag = (name) => process.argv.includes(`--${name}`)

const req = createRequire(new URL('./package.json', import.meta.url).pathname)
const { chromium } = req('playwright')

const port = Number(arg('port', process.env.TMC_REVIEW_PORT ?? 1440))
const base = `http://localhost:${port}/#`
const out = new URL('./out/raw/', import.meta.url).pathname
mkdirSync(out, { recursive: true })

const only = arg('only', '')?.split(',').filter(Boolean)
const wantThemes = arg('theme', '') ? [arg('theme')] : themes
const vars = { mod: arg('mod', process.env.SHOTS_MOD ?? '1') }

const list = screens.filter((s) => s.enabled !== false
    && (!only.length || only.includes(s.slot))
    && !(flag('gpu-only') && !s.gpu) && !(flag('no-gpu') && s.gpu))

const cdp = process.env.GPU_CDP_URL
const browser = cdp
    ? await chromium.connectOverCDP(cdp)
    : await chromium.launch({ args: ['--use-gl=swiftshader', '--enable-unsafe-swiftshader'] })
const context = await browser.newContext({ viewport: SIZE, deviceScaleFactor: 1, locale: 'en-US', timezoneId: 'UTC' })

const report = []
let failed = 0
for (const theme of wantThemes) {
    for (const s of list) {
        const page = await context.newPage()
        const errors = []
        page.on('pageerror', (e) => errors.push(e.message.slice(0, 300)))
        page.on('console', (m) => { if (m.type() === 'error') errors.push(m.text().slice(0, 300)) })
        await page.addInitScript((t) => {
            globalThis.__shotsTheme = t
            // Stable clock for relative times ("2 min ago") and charts.
            const fixed = Date.UTC(2026, 10, 20, 18, 0, 0)
            const RealDate = Date
            const offset = fixed - RealDate.now()
            globalThis.Date = class extends RealDate {
                constructor(...a) { super(...(a.length ? a : [RealDate.now() + offset])) }
                static now() { return RealDate.now() + offset }
            }
        }, theme)
        const route = s.route.replace(/\{(\w+)\}/g, (_, k) => vars[k])
        await page.goto(base + route)
        await page.waitForLoadState('networkidle', { timeout: 30000 }).catch(() => {})
        await page.locator(s.wait).first().waitFor({ state: 'visible', timeout: 30000 }).catch(() => errors.push(`wait ${s.wait} timed out`))
        if (s.act) await s.act(page)
        // No caret, no scrollbars, no running transitions in a still.
        await page.addStyleTag({ content: '*{caret-color:transparent!important}::-webkit-scrollbar{display:none}*{scrollbar-width:none!important}' })
        await page.waitForTimeout(Number(s.settle ?? process.env.SETTLE ?? 2500))
        const file = `${out}${s.slot}-${theme}.png`
        await page.screenshot({ path: file, animations: 'disabled' })
        const gl = s.gpu ? await page.evaluate(() => {
            const c = document.createElement('canvas').getContext('webgl2')
            const d = c?.getExtension('WEBGL_debug_renderer_info')
            return d ? c.getParameter(d.UNMASKED_RENDERER_WEBGL) : 'no webgl'
        }) : undefined
        report.push({ slot: s.slot, theme, route, gl, errors: [...new Set(errors)] })
        console.log(`${s.slot}-${theme}${gl ? ` [${gl}]` : ''}${errors.length ? `  ${errors.length} error(s): ${errors[0]}` : ''}`)
        if (errors.some((e) => e.startsWith('wait '))) failed++
        await page.close()
    }
}
writeFileSync(`${out}capture.json`, JSON.stringify(report, null, 2))
await (cdp ? context.close() : browser.close())
process.exit(failed ? 1 : 0)
