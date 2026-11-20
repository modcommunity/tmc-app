/**
 * Workshop flows on e2e/review/fixtures.ts: browse and search the catalog,
 * subscribe, pause and resume, scope to picked games and back, owner edit
 * and delete. Screenshots go to docs/review/flow-workshop-*.png.
 * Prints PASS/FAIL per step; exits 1 on any FAIL or page error.
 */
import { createRequire } from 'node:module'

const { chromium } = createRequire(process.env.PLAYWRIGHT_FROM ?? '/home/core/projects/website-gs/package.json')('playwright')
const base = `http://localhost:${process.env.TMC_REVIEW_PORT ?? 1431}/#`
const out = new URL('../../docs/review/', import.meta.url).pathname
const browser = await chromium.launch({ args: ['--use-gl=swiftshader', '--enable-unsafe-swiftshader'] })
const page = await browser.newPage({ viewport: { width: 1280, height: 900 } })
const errors = []
page.on('pageerror', (e) => errors.push(e.message))
page.on('console', (m) => {
    if (m.type() === 'error' && !/Failed to load resource/.test(m.text())) errors.push(m.text().slice(0, 200))
})
await page.addInitScript(() => (globalThis.__reviewFixtures = true))
let failed = 0
const check = (name, ok) => {
    console.log(`${ok ? 'PASS' : 'FAIL'} ${name}`)
    if (!ok) failed++
}
const sends = (path) => page.evaluate((p) => globalThis.__reviewLog.filter((x) => x.cmd === 'api_send' && x.args.path === p), path)
const items = page.locator('[data-testid=workshop-item]')
const subs = page.locator('[data-testid=workshop-sub]')
const sub = (name) => subs.filter({ hasText: name })

await page.goto(base + '/workshop')
await items.first().waitFor({ timeout: 30000 })
await page.waitForTimeout(800)
check('catalog lists three packs', (await items.count()) === 3)
await page.screenshot({ path: `${out}flow-workshop-browse.png`, fullPage: true })

await page.getByLabel('Search the Workshop').fill('neon')
await page.waitForTimeout(1200)
check('search narrows the catalog', (await items.count()) === 1)
await page.getByLabel('Search the Workshop').fill('')
await page.waitForTimeout(1200)

// Subscribe to Neon Rifles; it shows up under subscribed packs.
await items.filter({ hasText: 'Neon Rifles' }).getByRole('button', { name: 'Subscribe' }).click()
await items.filter({ hasText: 'Neon Rifles' }).getByRole('button', { name: 'Unsubscribe' }).waitFor({ timeout: 10000 })
await page.getByRole('tab', { name: 'Subscribed' }).click()
await sub('Neon Rifles').waitFor({ timeout: 10000 })
check('subscribe posts and lists the pack', (await sends('/subscriptions')).some((c) => c.args.body.itemId === 501 && c.args.body.subscribed === true))

// Pause, then resume.
await sub('Neon Rifles').getByRole('button', { name: 'Pause' }).click()
await sub('Neon Rifles').getByRole('button', { name: 'Resume' }).waitFor({ timeout: 10000 })
check('pause shows Resume', true)
await sub('Neon Rifles').getByRole('button', { name: 'Resume' }).click()
await sub('Neon Rifles').getByRole('button', { name: 'Pause' }).waitFor({ timeout: 10000 })
check('resume sends two prefs writes', (await sends('/subscriptions/prefs')).filter((c) => c.args.body.itemId === 501).length === 2)

// Scope: pick games, tick Airdrift, then back to every game.
await sub('Neon Rifles').getByRole('button', { name: 'Pick games' }).click()
const box = sub('Neon Rifles').getByLabel('Airdrift')
await box.waitFor({ timeout: 10000 })
await box.click()
await page.waitForFunction(() => [...document.querySelectorAll('[data-testid=workshop-sub] label')].some((l) => /Airdrift/.test(l.textContent) && l.querySelector('input').checked), null, { timeout: 10000 }).catch(() => {})
check('scope to a picked game', await box.isChecked())
await page.screenshot({ path: `${out}flow-workshop-scope.png`, fullPage: true })
await sub('Neon Rifles').getByRole('button', { name: 'Use in all games' }).click()
await sub('Neon Rifles').getByRole('button', { name: 'Pick games' }).waitFor({ timeout: 10000 })
check('scope writes: listed, game, all', (await sends('/workshop/scope')).length === 3)

// Unsubscribe from Desert Maps.
await sub('Desert Maps').getByRole('button', { name: 'Unsubscribe' }).click()
await page.waitForTimeout(1200)
check('unsubscribe removes the row', (await sub('Desert Maps').count()) === 0)

// Owner edit and delete.
await page.getByRole('tab', { name: 'My packs' }).click()
const own = page.locator('[data-testid=workshop-owned]').first()
await own.getByLabel('Pack name').fill('Neon Rifles Deluxe')
await own.getByRole('button', { name: 'Save' }).click()
await page.waitForTimeout(1200)
check('owner edit sends the new name', (await sends('/workshop/item')).some((c) => c.args.method === 'PATCH' && c.args.body.name === 'Neon Rifles Deluxe'))
await page.screenshot({ path: `${out}flow-workshop-owned.png`, fullPage: true })
await own.getByRole('button', { name: 'Delete' }).click()
await own.getByRole('button', { name: 'Delete for good' }).click()
await page.waitForTimeout(1200)
check('owner delete removes the pack', (await page.locator('[data-testid=workshop-owned]').count()) === 0)

check('no page errors', errors.length === 0)
if (errors.length) console.log('errors:', [...new Set(errors)].slice(0, 6))
await browser.close()
console.log(failed ? `FAILED ${failed}` : 'ALL PASS')
process.exit(failed ? 1 : 0)
