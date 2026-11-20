/**
 * Drive the review harness: visit each main screen, screenshot it into
 * docs/review/, and print every console error and failed IPC parse per route.
 * Usage: node e2e/review/shoot.mjs [route ...]   (harness on TMC_REVIEW_PORT, 1431)
 * THEME=light|dark (default dark) and WIDTH=<px> (default 1280) pick the
 * colour scheme and window width; anything but the defaults suffixes the file.
 * FULL=1 also scrolls the main pane to its end and shoots that (`-end`).
 */
import { createRequire } from 'node:module'
import { mkdirSync } from 'node:fs'

const req = createRequire(process.env.PLAYWRIGHT_FROM ?? '/home/core/projects/website-gs/package.json')
const { chromium } = req('playwright')

const base = `http://localhost:${process.env.TMC_REVIEW_PORT ?? 1431}/#`
const out = new URL('../../docs/review/', import.meta.url).pathname
mkdirSync(out, { recursive: true })

const routes = process.argv.slice(2).length
    ? process.argv.slice(2)
    : ['/play', '/games', '/games/fatal-contact', '/games/fatal-contact/avatar', '/avatar', '/workshop', '/me', '/discover', '/apps', '/friends', '/browse/server', '/library', '/installs', '/sandboxes', '/downloads', '/stores', '/account', '/settings', '/settings/games', '/settings/plugins', '/settings/logging']

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? undefined, args: ['--use-gl=swiftshader', '--enable-unsafe-swiftshader'] })
const theme = process.env.THEME ?? 'dark'
const width = Number(process.env.WIDTH ?? 1280)
const suffix = (theme === 'dark' ? '' : `-${theme}`) + (width === 1280 ? '' : `-${width}`)
const page = await browser.newPage({ viewport: { width, height: 800 }, colorScheme: theme })
let errors = []
page.on('console', (m) => { if (m.type() === 'error') errors.push(m.text().slice(0, 300)) })
page.on('pageerror', (e) => errors.push(`pageerror: ${e.message.slice(0, 300)}`))

for (const r of routes) {
    errors = []
    await page.goto(base + r)
    await page.waitForLoadState('networkidle', { timeout: 30000 }).catch(() => {})
    await page.waitForTimeout(Number(process.env.SETTLE ?? 2500))
    const name = r.replace(/^\//, '').replace(/[/:]/g, '_') || 'root'
    await page.screenshot({ path: `${out}${name}${suffix}.png` })
    if (process.env.FULL) {
        await page.evaluate(() => document.querySelectorAll('main, main *').forEach((el) => { if (el.scrollHeight > el.clientHeight + 4) el.scrollTop = el.scrollHeight }))
        await page.waitForTimeout(300)
        await page.screenshot({ path: `${out}${name}${suffix}-end.png` })
    }
    const text = (await page.locator('main').first().innerText().catch(() => '')).replace(/\s+/g, ' ').slice(0, 220)
    console.log(`== ${r}: ${text}`)
    for (const e of [...new Set(errors)]) console.log(`   ERR ${e}`)
}
await browser.close()
