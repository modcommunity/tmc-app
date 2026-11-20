/**
 * Install flows on the mocked bridge (fixture mode), the ones library.mjs does
 * not cover:
 *   - a TMC game: install, an update appearing and applied, uninstall;
 *   - a mod: one-click install into a sandbox, an update, undeploy (restores
 *     the game's own files);
 *   - a server: the live query's ping on the Play list, then Join through the
 *     launcher to `play_connect`.
 * Each step asserts on the IPC log (`__reviewLog`) and on what the screen says.
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

async function open() {
    const page = await browser.newPage({ viewport: { width: 1280, height: 860 }, colorScheme: 'dark' })
    const errors = []
    page.on('pageerror', (e) => errors.push(e.message))
    page.on('dialog', (d) => d.accept())
    await page.addInitScript(() => (globalThis.__reviewFixtures = true))
    return { page, errors }
}
const calls = (page, cmd) => page.evaluate((c) => globalThis.__reviewLog.filter((x) => x.cmd === c), cmd)
const shot = (page, name) => page.screenshot({ path: `${out}${name}.png` })
const button = (page, re) => page.locator('button:visible', { hasText: re }).first()
const step = async (name, fn) => {
    try {
        await fn()
    } catch (e) {
        check(name, false, String(e.message ?? e).split('\n')[0])
    }
}

// ------------------------------------------------------------ A TMC game
{
    const { page, errors } = await open()
    await step('game flow', async () => {
        await page.goto(base + '/library?view=tmc')
        await page.locator('h3', { hasText: /^Godot$/ }).waitFor({ timeout: 15000 })
        const row = page // the dev site publishes one installable game

        await button(row, /^Install$/).click()
        await button(row, /^Play$/).waitFor({ timeout: 5000 })
        const inst = await calls(page, 'game_install')
        check('game: Install calls game_install for the app', inst.length === 1 && Number(inst[0].args.request.appId) === 69, JSON.stringify(inst))
        await shot(page, 'flow-game-installed')

        // An older build on disk: the check must offer the published one.
        await page.evaluate(() => (globalThis.__reviewGames.get(69).version = '1.20260101.0000'))
        await button(page, /Check for updates/).click()
        await row.getByText(/Update to 1\.20261113\.1637/).waitFor({ timeout: 5000 })
        check('game: check for updates offers the published build', (await calls(page, 'games_check_updates')).length >= 1)
        await shot(page, 'flow-game-update')

        await button(row, /^Update$/).click()
        await row.getByText(/Update to/).waitFor({ state: 'detached', timeout: 5000 })
        const ver = await page.evaluate(() => globalThis.__reviewGames.get(69).version)
        check('game: Update installs the published build', (await calls(page, 'game_install')).length === 2 && ver === '1.20261113.1637', ver)

        await button(row, /^Remove$/).click()
        await button(row, /^Install$/).waitFor({ timeout: 5000 })
        const un = await calls(page, 'game_uninstall')
        check('game: Remove calls game_uninstall', un.length === 1 && Number(un[0].args.appId) === 69)
    })
    check('game: no page errors', errors.length === 0, errors.join(' | '))
    await page.close()
}

// ------------------------------------------------------------ A mod in a sandbox
{
    const { page, errors } = await open()
    await step('mod flow', async () => {
        await page.goto(base + '/sandboxes')
        await button(page, /New sandbox/).click()
        await page.getByRole('button', { name: 'Game' }).or(page.getByRole('combobox', { name: 'Game' })).first().click()
        await page.getByRole('option', { name: /Counter-Strike: Source/ }).first().click()
        await page.getByRole('textbox').last().fill('Review mods')
        await button(page, /^Create/).click()
        await page.waitForTimeout(600)

        await page.goto(base + '/view/mod/2')
        await page.getByText('Install into a sandbox').first().waitFor({ timeout: 15000 })
        await button(page, /^Install$/).click()
        await page.getByText(/Installed|Staged|Deployed/i).first().waitFor({ timeout: 5000 })
        const inst = await calls(page, 'sandbox_install_item')
        check('mod: Install calls sandbox_install_item for the item', inst.length === 1 && Number(inst[0].args.itemId) === 2 && inst[0].args.kind === 'mod', JSON.stringify(inst.map((c) => c.args)))
        await shot(page, 'flow-mod-installed')

        await page.goto(base + '/sandboxes/1')
        await page.getByText(/1 item has a newer release/).waitFor({ timeout: 8000 })
        check('mod: the sandbox shows the update', true)
        await button(page, /Update now/).click()
        await page.getByText(/newer release/).waitFor({ state: 'detached', timeout: 5000 })
        const st = await calls(page, 'sandbox_stage')
        check('mod: Update now re-stages with force', st.some((c) => c.args.force === true && Number(c.args.id) === 1))

        await button(page, /^Undeploy$/).click()
        await button(page, /^Undeploy$/).waitFor({ state: 'detached', timeout: 5000 })
        check('mod: Undeploy calls sandbox_purge (restores the game files)', (await calls(page, 'sandbox_purge')).length === 1)
        await shot(page, 'flow-mod-restored')
    })
    check('mod: no page errors', errors.length === 0, errors.join(' | '))
    await page.close()
}

// ------------------------------------------------------------ A server
{
    const { page, errors } = await open()
    await step('server flow', async () => {
        await page.addInitScript(() => (globalThis.__reviewConnectUrl = true))
        await page.goto(base + '/view/server/652114')
        await page.getByText(/^42ms$/).first().waitFor({ timeout: 15000 })
        check('server: the live query ping is shown', (await calls(page, 'query_server')).length >= 1)
        await shot(page, 'flow-server-ping')

        await page.getByRole('button', { name: 'Play', exact: true }).first().click()
        await page.getByText('Connect with the game client').click({ timeout: 10000 })
        await shot(page, 'flow-server-join')
        await page.locator('footer button:visible', { hasText: /^\s*Play$/ }).last().click()
        await page.waitForTimeout(800)
        const pc = await calls(page, 'play_connect')
        check('server: Join connects through play_connect', pc.length === 1 && Number(pc[0].args.serverId) > 0, JSON.stringify(pc))
    })
    check('server: no page errors', errors.length === 0, errors.join(' | '))
    await page.close()
}

await browser.close()
console.log(failed ? `${failed} FAILED` : 'ALL PASS')
process.exit(failed ? 1 : 0)
