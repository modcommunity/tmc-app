/**
 * Store and Stats flows on the review harness. Signed out, the Fatal Contact
 * store loads from the real dev backend (a live loader call). Signed in, the
 * flows run on e2e/review/fixtures.ts: browse, filter, buy with points,
 * equip, a credits item opening the website, inventory, stats overview and
 * drill-down, and the /me summary. Screenshots go to docs/review/.
 * Prints PASS/FAIL per step; exits 1 on any FAIL or page error.
 */
import { createRequire } from 'node:module'

const { chromium } = createRequire(process.env.PLAYWRIGHT_FROM ?? '/home/core/projects/website-gs/package.json')('playwright')
const base = `http://localhost:${process.env.TMC_REVIEW_PORT ?? 1431}/#`
const out = new URL('../../docs/review/', import.meta.url).pathname
const browser = await chromium.launch({ args: ['--use-gl=swiftshader', '--enable-unsafe-swiftshader'] })
let failed = 0
const check = (name, ok) => {
    console.log(`${ok ? 'PASS' : 'FAIL'} ${name}`)
    if (!ok) failed++
}

async function open(fixtures, width = 1280) {
    const page = await browser.newPage({ viewport: { width, height: 800 } })
    const errors = []
    page.on('pageerror', (e) => errors.push(e.message))
    page.on('console', (m) => {
        if (m.type() === 'error' && !/Failed to load resource/.test(m.text())) errors.push(m.text().slice(0, 200))
    })
    if (fixtures) await page.addInitScript(() => (globalThis.__reviewFixtures = true))
    return { page, errors }
}
const calls = (page, cmd) => page.evaluate((c) => globalThis.__reviewLog.filter((x) => x.cmd === c), cmd)
const settle = (page) => page.waitForTimeout(Number(process.env.SETTLE ?? 1200))
const shot = (page, name) => page.screenshot({ path: `${out}${name}.png` })

// Signed out, real dev backend: Fatal Contact's store is open with items.
{
    const { page, errors } = await open(false)
    await page.goto(base + '/store/101')
    await page.locator('[data-testid=store-item]').first().waitFor({ timeout: 30000 })
    check('dev store/front loads and parses (signed out)', (await page.locator('[data-testid=store-item]').count()) > 0)
    await page.locator('[data-testid=store-item]').first().click()
    await page.getByRole('dialog').getByRole('button', { name: 'Sign in to buy' }).waitFor({ timeout: 10000 })
    check('signed out item offers sign in', true)
    check('no page errors signed out', errors.length === 0)
    if (errors.length) console.log('errors:', [...new Set(errors)].slice(0, 6))
    await page.close()
}

for (const width of [1280, 760]) {
    const tag = width === 1280 ? '' : '-narrow'
    const { page, errors } = await open(true, width)

    await page.goto(base + '/store')
    await page.locator('[data-testid=store-card]').first().waitFor({ timeout: 30000 })
    await settle(page)
    check(`directory lists stores${tag}`, (await page.locator('[data-testid=store-card]').count()) === 4)
    await shot(page, `store${tag}`)

    await page.locator('[data-testid=store-card]').first().click()
    await page.locator('[data-testid=store-item]').first().waitFor({ timeout: 30000 })
    await settle(page)
    await shot(page, `store_101${tag}`)
    check(`store shows 8 items${tag}`, (await page.locator('[data-testid=store-item]').count()) === 8)

    if (width === 1280) {
        await page.getByPlaceholder('Search items').fill('hat')
        await settle(page)
        check('search narrows to one item', (await page.locator('[data-testid=store-item]').count()) === 1)
        await page.getByPlaceholder('Search items').fill('')

        // Buy the Victory Dance with points.
        await page.locator('[data-testid=store-item]', { hasText: 'Victory Dance' }).click()
        await page.getByRole('dialog').getByRole('button', { name: /^Buy for 30/ }).click()
        await page.locator('[data-testid=store-confirm]').waitFor()
        await shot(page, 'flow-store-confirm')
        await page.getByRole('dialog').getByRole('button', { name: 'Confirm' }).click()
        await page.getByText('Bought. It is in your inventory.').waitFor({ timeout: 10000 })
        const buys = (await calls(page, 'api_send')).filter((c) => c.args.path === '/store/purchase')
        check('points buy sends one purchase with an idempotency key', buys.length === 1 && /^[A-Za-z0-9_-]{8,64}$/.test(buys[0].args.body.idemKey))
        await settle(page)
        check('wallet shows the new balance', /310/.test(await page.locator('[data-testid=store-wallet]').innerText()))
        await page.keyboard.press('Escape')

        // Equip the spray slot item, then the loadout panel lists it.
        await page.locator('[data-testid=store-item]', { hasText: 'Victory Dance' }).click()
        const equip = page.getByRole('dialog').getByRole('button', { name: /^Equip in / })
        await equip.click()
        await page.getByRole('dialog').getByRole('button', { name: /^Unequip from / }).waitFor({ timeout: 10000 })
        check('equip saves the loadout', (await calls(page, 'api_send')).some((c) => c.args.path === '/store/loadout' && c.args.body.key === 'fx-dance'))
        await shot(page, 'flow-store-equipped')
        await page.keyboard.press('Escape')
        await settle(page)
        check('loadout panel lists the item', /Victory Dance|fx-dance/.test(await page.locator('[data-testid=store-loadout]').innerText()))

        // A credits item opens the website in the external browser.
        await page.locator('[data-testid=store-item]', { hasText: 'VIP monthly' }).click()
        await page.getByRole('dialog').getByRole('button', { name: 'Buy on the website' }).click()
        await settle(page)
        const opened = (await page.evaluate(() => globalThis.__reviewLog)).filter((c) => /opener/.test(c.cmd))
        check('credits checkout opens the website externally', opened.length === 1 && JSON.stringify(opened[0].args).includes('tmcdev.net/dashboard/store/app/101'))
        check('credits checkout never posts a purchase', (await calls(page, 'api_send')).filter((c) => c.args.path === '/store/purchase').length === 1)
        await page.keyboard.press('Escape')

        await page.goto(base + '/store?tab=inventory')
        await page.getByText('Victory Dance').first().waitFor({ timeout: 10000 })
        await settle(page)
        check('inventory lists the new purchase', true)
        await shot(page, 'store_inventory')
    }

    await page.goto(base + '/stats')
    await page.locator('[data-testid=stats-totals]').waitFor({ timeout: 30000 })
    await settle(page)
    check(`stats overview shows three games${tag}`, (await page.locator('[data-testid=stats-game]').count()) === 3)
    await shot(page, `stats${tag}`)

    await page.locator('[data-testid=stats-game]').first().click()
    await page.locator('[data-testid=stats-mine]').waitFor({ timeout: 30000 })
    await settle(page)
    await shot(page, `stats_101${tag}`)
    check(`game stats lists values${tag}`, /1,284/.test(await page.locator('[data-testid=stats-mine]').innerText()))

    if (width === 1280) {
        await page.getByRole('tab', { name: /Achievements/ }).click()
        await page.locator('[data-testid=stats-achievements]').waitFor({ timeout: 10000 })
        await settle(page)
        check('achievements show unlocked and locked', /Ace/.test(await page.locator('[data-testid=stats-achievements]').innerText()))
        await shot(page, 'stats_101_achievements')
        await page.getByRole('tab', { name: /Leaderboards/ }).click()
        await settle(page)
        await shot(page, 'stats_101_leaderboards')
        check('leaderboard shows rows', (await page.getByText('Vexa').count()) > 0)

        await page.goto(base + '/me')
        await page.locator('[data-testid=me-stats]').waitFor({ timeout: 30000 })
        await settle(page)
        await shot(page, 'me')
        check('/me shows the stats summary', true)
    }
    check(`no page errors${tag}`, errors.length === 0)
    if (errors.length) console.log('errors:', [...new Set(errors)].slice(0, 6))
    await page.close()
}

await browser.close()
console.log(failed ? `FAILED ${failed}` : 'ALL PASS')
process.exit(failed ? 1 : 0)
