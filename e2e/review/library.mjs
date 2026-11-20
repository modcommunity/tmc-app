/**
 * Library flows on the review harness (nightly gamespice-app-workshop-e2e-2).
 * Signed in on e2e/review/fixtures.ts: sandbox create, a second one, switch
 * the default, delete, against mock.ts's in-memory sandbox store. Signed out
 * on the dev backend: the server browser's game filter narrows the list and
 * lands in the URL. Screenshots go to docs/review/flow-*.png.
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

async function open(fixtures) {
    const page = await browser.newPage({ viewport: { width: 1280, height: 800 }, colorScheme: 'dark' })
    const errors = []
    page.on('pageerror', (e) => errors.push(e.message))
    page.on('console', (m) => {
        if (m.type() === 'error' && !/Failed to load resource/.test(m.text())) errors.push(m.text().slice(0, 200))
    })
    if (fixtures) await page.addInitScript(() => (globalThis.__reviewFixtures = true))
    return { page, errors }
}
const calls = (page, cmd) => page.evaluate((c) => globalThis.__reviewLog.filter((x) => x.cmd === c), cmd)
const shot = (page, name) => page.screenshot({ path: `${out}${name}.png` })

async function newSandbox(page, name) {
    await page.goto(base + '/sandboxes')
    await page.locator('button:visible', { hasText: /New sandbox/ }).first().click()
    await page.getByRole('button', { name: 'Game' }).or(page.getByRole('combobox', { name: 'Game' })).first().click()
    await page.getByRole('option', { name: /Counter-Strike: Source/ }).first().click()
    await page.getByRole('textbox').last().fill(name)
    await page.locator('button:visible', { hasText: /^Create/ }).first().click()
    await page.waitForTimeout(800)
}

// Sandboxes: create two for one game, switch the default, delete one.
{
    const { page, errors } = await open(true)
    try {
        await newSandbox(page, 'Review one')
        check('first sandbox created', (await calls(page, 'sandbox_create')).length === 1)
        await shot(page, 'flow-sandbox-created')

        await newSandbox(page, 'Review two')
        check('second sandbox created', (await calls(page, 'sandbox_create')).length === 2)

        await page.goto(base + '/sandboxes')
        await page.getByText('Review two').first().click()
        await page.locator('button:visible', { hasText: /Make this the default/ }).first().click()
        await page.waitForTimeout(500)
        const def = await calls(page, 'sandbox_set_default')
        check('default switched to the second', def.length === 1 && Number(def[0].args.id) === 2)

        await page.locator('button:visible', { hasText: /Delete sandbox/ }).first().click()
        await page.locator('button:visible', { hasText: /^Delete$/ }).first().click()
        await page.waitForTimeout(800)
        const del = await calls(page, 'sandbox_delete')
        check('second sandbox deleted', del.length === 1 && Number(del[0].args.id) === 2)
        await page.goto(base + '/sandboxes')
        await page.getByText('Review one').first().waitFor({ timeout: 10000 })
        check('list keeps only the first', (await page.getByText('Review two').count()) === 0)
        await shot(page, 'flow-sandbox-after-delete')
    } catch (e) {
        check(`sandbox flow: ${String(e.message ?? e).split('\n')[0]}`, false)
        await shot(page, 'flow-sandbox-failed')
    }
    check('no page errors in sandbox flow', errors.length === 0)
    if (errors.length) console.log('errors:', [...new Set(errors)].slice(0, 6))
}

// Servers: the game filter narrows the dev backend's list and is kept in the URL.
{
    const { page, errors } = await open(false)
    try {
        await page.goto(base + '/browse/server')
        const rows = page.locator('main tbody tr')
        await rows.first().waitFor({ timeout: 30000 })
        const before = await rows.count()
        await page.locator('main aside').getByText('Playlot', { exact: true }).first().click()
        await page.getByText('Clear 1 filter').first().waitFor({ timeout: 10000 })
        await page.waitForTimeout(1500)
        const after = await rows.count()
        check(`server game filter narrows ${before} -> ${after}`, after > 0 && after < before)
        const hash = await page.evaluate(() => location.hash)
        check(`filter is in the URL (${hash})`, hash.includes('?'))
        await shot(page, 'flow-servers-filtered')
    } catch (e) {
        check(`server flow: ${String(e.message ?? e).split('\n')[0]}`, false)
        await shot(page, 'flow-servers-failed')
    }
    check('no page errors in server flow', errors.length === 0)
    if (errors.length) console.log('errors:', [...new Set(errors)].slice(0, 6))
}

await browser.close()
process.exit(failed ? 1 : 0)
