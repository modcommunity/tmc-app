/**
 * Layout assertions on the review harness: the "Powered by TMC" footer stays
 * small, sits bottom-right, and never overlaps content, at the 760px desktop
 * edge and at a 480px phone width (compact layout with the bottom bar).
 * Fixture mode, so nothing here needs the network beyond the dev reads.
 * Prints PASS/FAIL per check; exits 1 on any FAIL or page error.
 */
import { createRequire } from 'node:module'

const { chromium } = createRequire(process.env.PLAYWRIGHT_FROM ?? '/home/core/projects/website-gs/package.json')('playwright')
const base = `http://localhost:${process.env.TMC_REVIEW_PORT ?? 1431}/#`
const out = new URL('../../docs/review/', import.meta.url).pathname
const browser = await chromium.launch({ args: ['--use-gl=swiftshader', '--enable-unsafe-swiftshader'] })
let failed = 0
const check = (name, ok, detail = '') => {
    console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${ok || !detail ? '' : ` (${detail})`}`)
    if (!ok) failed++
}

const ROUTES = ['/play', '/browse/mod', '/library', '/settings']

for (const width of [760, 480]) {
    const page = await browser.newPage({ viewport: { width, height: 800 }, colorScheme: 'dark' })
    const errors = []
    page.on('pageerror', (e) => errors.push(e.message))
    await page.addInitScript(() => (globalThis.__reviewFixtures = true))
    for (const route of ROUTES) {
        await page.goto(base + route)
        await page.locator('main').first().waitFor()
        await page.waitForTimeout(1200)
        // Scroll the content to the bottom too: the footer must not cover the last row.
        const m = await page.evaluate(() => {
            const btn = [...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Powered by TMC')
            if (!btn) return null
            const main = btn.closest('main')
            const scroller = main?.querySelector(':scope > .overflow-y-auto')
            scroller?.scrollTo(0, scroller.scrollHeight)
            const r = (el) => {
                const b = el.getBoundingClientRect()
                return { x: b.x, y: b.y, w: b.width, h: b.height, right: b.right, bottom: b.bottom }
            }
            const f = r(btn)
            const s = scroller ? r(scroller) : null
            const nav = main?.querySelector(':scope > nav')
            const hit = document.elementFromPoint(f.x + f.w / 2, f.y + f.h / 2)
            return {
                fontPx: parseFloat(getComputedStyle(btn).fontSize),
                f,
                s,
                nav: nav ? r(nav) : null,
                vw: window.innerWidth,
                vh: window.innerHeight,
                topmost: hit === btn || btn.contains(hit),
            }
        })
        const tag = `${width}px ${route}`
        if (!m) {
            check(`${tag}: footer present`, false)
            continue
        }
        check(`${tag}: footer is small`, m.fontPx <= 12 && m.f.h <= 18, `${m.fontPx}px, ${Math.round(m.f.h)}px tall`)
        check(`${tag}: footer is bottom-right`, m.vw - m.f.right <= 24 && m.f.y > m.vh * 0.75, JSON.stringify(m.f))
        check(`${tag}: footer below the content, not over it`, !!m.s && m.f.y >= m.s.bottom - 0.5, JSON.stringify({ f: m.f, s: m.s }))
        check(`${tag}: footer above the bottom bar`, !m.nav || m.f.bottom <= m.nav.y + 0.5, JSON.stringify({ f: m.f, nav: m.nav }))
        check(`${tag}: nothing drawn over the footer`, m.topmost)
        if (route === '/play') await page.screenshot({ path: `${out}footer-${width}.png` })
    }
    check(`${width}px: no page errors`, errors.length === 0, errors.join(' | '))
    await page.close()
}

await browser.close()
console.log(failed ? `${failed} FAILED` : 'ALL PASS')
process.exit(failed ? 1 : 0)
