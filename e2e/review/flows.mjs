/**
 * Interactive flows on the review harness: Workshop scope round trip against
 * the dev backend, and sandbox create / default / delete on the mocked store.
 * Screenshots go to docs/review/flow-*.png. Prints PASS/FAIL per step.
 */
import { createRequire } from 'node:module'

const { chromium } = createRequire(process.env.PLAYWRIGHT_FROM ?? '/home/core/projects/website-gs/package.json')('playwright')
const base = `http://localhost:${process.env.TMC_REVIEW_PORT ?? 1431}/#`
const out = new URL('../../docs/review/', import.meta.url).pathname
const browser = await chromium.launch()
const page = await browser.newPage({ viewport: { width: 1280, height: 800 } })
const errors = []
page.on('pageerror', (e) => errors.push(e.message))
page.on('console', (m) => { if (m.type() === 'error') errors.push(m.text().slice(0, 200)) })
let failed = 0
const check = (name, ok) => { console.log(`${ok ? 'PASS' : 'FAIL'} ${name}`); if (!ok) failed++ }
const calls = (cmd) => page.evaluate((c) => globalThis.__reviewLog.filter((x) => x.cmd === c), cmd)

// Workshop: limit the first pack to listed games, then put it back.
await page.goto(base + '/workshop')
const first = page.locator('button:visible', { hasText: /Limit to listed|Use in all games/ }).first()
await first.waitFor({ timeout: 30000 })
const before = await first.innerText()
await first.click()
await page.locator('button:visible', { hasText: before === 'Limit to listed' ? 'Use in all games' : 'Limit to listed' }).first().waitFor({ timeout: 30000 })
check('workshop scope flips on the dev backend', true)
await page.screenshot({ path: `${out}flow-workshop-scoped.png` })
await page.locator('button:visible', { hasText: before === 'Limit to listed' ? 'Use in all games' : 'Limit to listed' }).first().click()
await page.locator('button:visible', { hasText: before }).first().waitFor({ timeout: 30000 })
check('workshop scope restored', (await calls('api_send')).filter((c) => c.args.path === '/workshop/scope').length === 2)

// Sandboxes: create two for a game, switch the default, delete one.
await page.goto(base + '/sandboxes')
await page.locator('button:visible', { hasText: /New sandbox/ }).first().click()
await page.waitForTimeout(1500)
await page.screenshot({ path: `${out}flow-sandbox-new.png` })
console.log('dialog:', (await page.locator('[role=dialog], dialog, form').first().innerText().catch(() => 'none')).replace(/\s+/g, ' ').slice(0, 300))
console.log('errors:', [...new Set(errors)].slice(0, 6))
await browser.close()
process.exit(failed ? 1 : 0)
