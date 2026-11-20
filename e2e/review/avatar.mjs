/**
 * Avatar save and load against the REAL dev site (no fixtures): the harness
 * must run with TMC_REVIEW_TOKEN set to a dev account that owns an avatar
 * listed in a game (app 101 lists the store avatar "[fx] Kenney Bot").
 *
 *   1. open the game's avatar page, pick the avatar: `avatar_me_put` goes out
 *      and the dev site answers with it chosen;
 *   2. open the page again in a fresh tab: `avatar_me` loads it back and the
 *      tile reads Chosen;
 *   3. put the account back the way it was.
 *
 * Env: TMC_REVIEW_PORT (default 1431), AVATAR_APP (default 101).
 * Prints PASS/FAIL per check; exits 1 on any FAIL or page error.
 */
import { createRequire } from 'node:module'

const { chromium } = createRequire(process.env.PLAYWRIGHT_FROM ?? '/home/core/projects/website-gs/package.json')('playwright')
const base = `http://localhost:${process.env.TMC_REVIEW_PORT ?? 1431}`
const APP = Number(process.env.AVATAR_APP ?? 101)
const out = new URL('../../docs/review/', import.meta.url).pathname
const browser = await chromium.launch({ args: ['--use-gl=swiftshader', '--enable-unsafe-swiftshader'] })
let failed = 0
const check = (name, ok, detail = '') => {
    console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${ok || !detail ? '' : ` (${detail})`}`)
    if (!ok) failed++
}

const me = async (page) => page.evaluate(async (app) => (await (await fetch(`/__avatar/me?app=${app}`)).json()), APP).then((j) => j?.data ?? j)
const put = async (page, body) =>
    page.evaluate(async (b) => (await fetch('/__avatar/me', { method: 'PUT', headers: { 'content-type': 'application/json' }, body: JSON.stringify(b) })).status, body)

async function open() {
    const page = await browser.newPage({ viewport: { width: 1280, height: 860 }, colorScheme: 'dark' })
    const errors = []
    page.on('pageerror', (e) => errors.push(e.message))
    return { page, errors }
}

/** The platform slug whose game is APP: open each avatar page until the library read names APP. */
async function slugOf(page) {
    const { PLATFORM_GAMES } = await import('../../src/lib/gamespice/games.ts').catch(() => ({ PLATFORM_GAMES: null }))
    const slugs = PLATFORM_GAMES?.map((g) => g[0]) ?? ['prop-hunt', 'playlot', 'fatal-contact', 'airdrift', 'hungario', 'the-room', 'look-at-me', 'hide-n-seek']
    for (const slug of slugs) {
        await page.goto(`${base}/#/games/${slug}/avatar`)
        const hit = await page
            .waitForFunction((app) => globalThis.__reviewLog?.some((x) => x.cmd === 'avatar_library' && Number(x.args.appId) === app), APP, { timeout: 6000 })
            .then(() => true)
            .catch(() => false)
        if (hit) return slug
    }
    return null
}

/** The avatar's tile; avatars sit under group tabs (Kenney Blocky, Other), so try each. */
async function findTile(p, id) {
    const tile = p.locator(`[data-avatar-tile="${id}"]`)
    await p.getByText('Built-in figure').first().waitFor({ timeout: 15000 })
    if (await tile.isVisible()) return tile
    const groups = p.locator('[role=tablist] [role=tab]').filter({ hasText: /\d+$/ })
    for (let i = 0; i < (await groups.count()); i++) {
        await groups.nth(i).click()
        if (await tile.waitFor({ timeout: 2000 }).then(() => true, () => false)) return tile
    }
    throw new Error(`no tile for avatar ${id} in any group`)
}

const { page, errors } = await open()
await page.goto(`${base}/#/play`)
const before = await me(page)
check('the harness reads the dev account', before && Number(before.appId) === APP, JSON.stringify(before)?.slice(0, 120))
const library = await page.evaluate(async (app) => (await (await fetch(`/__avatar/library?app=${app}`)).json()), APP).then((j) => j?.data ?? j)
const avatar = library?.avatars?.[0]
check('the game library lists an avatar', !!avatar, JSON.stringify(library)?.slice(0, 160))
const original = before?.choice?.avatarId ?? null

try {
    if (!avatar) throw new Error('nothing to pick')
    // Start from "no avatar picked" so the pick is a real change.
    if (original === avatar.id) await put(page, { app: APP, avatarId: null })

    const slug = await slugOf(page)
    check('a platform game opens the avatar studio for the app', !!slug)
    if (!slug) throw new Error('no slug')

    const tile = await findTile(page, avatar.id)
    await tile.locator('button').first().click()
    await page.waitForFunction(() => globalThis.__reviewLog.some((x) => x.cmd === 'avatar_me_put'), null, { timeout: 10000 })
    const sent = await page.evaluate(() => globalThis.__reviewLog.find((x) => x.cmd === 'avatar_me_put').args.body)
    check('picking sends avatar_me_put with the avatar', Number(sent.app) === APP && sent.avatarId === avatar.id, JSON.stringify(sent))

    // The dev site decides. A store avatar the account does not own is
    // refused: the studio must say why (the site's words, not "HTTP 403")
    // and nothing is stored. An owned one saves: the tile reads Chosen and a
    // fresh tab loads it back through avatar_me.
    const refused = page.getByText(/not available in this app|Buy .* first/)
    const chosen = tile.getByText('Chosen')
    const outcome = await Promise.race([
        refused.first().waitFor({ timeout: 10000 }).then(() => 'refused'),
        chosen.waitFor({ timeout: 10000 }).then(() => 'saved'),
    ])
    const saved = await me(page)
    await page.screenshot({ path: `${out}flow-avatar-${outcome}.png` })
    if (outcome === 'refused') {
        check('a refused pick shows the site\'s reason', true)
        check('a refused pick stores nothing', (saved?.choice?.avatarId ?? null) !== avatar.id, JSON.stringify(saved?.choice))
        console.log(`NOTE the account does not own avatar ${avatar.id}: the saved-and-reloaded path needs it (store item) to run`)
    } else {
        check('the dev site stored the pick', saved?.choice?.avatarId === avatar.id, JSON.stringify(saved?.choice))
        // Load: a fresh tab knows nothing of the save but what avatar_me returns.
        const fresh = await open()
        await fresh.page.goto(`${base}/#/games/${slug}/avatar`)
        const t2 = await findTile(fresh.page, avatar.id)
        await t2.getByText('Chosen').waitFor({ timeout: 15000 })
        check('a fresh tab loads the pick through avatar_me', await fresh.page.evaluate(() => globalThis.__reviewLog.some((x) => x.cmd === 'avatar_me')))
        check('fresh tab: no page errors', fresh.errors.length === 0, fresh.errors.join(' | '))
        await fresh.page.close()
    }
} catch (e) {
    await page.screenshot({ path: process.env.AVATAR_DEBUG ?? '/dev/null' }).catch(() => {}); console.log((await page.locator('main').innerText().catch(() => '')).slice(0, 600).replace(/\n+/g, ' | '))
    check('avatar flow', false, String(e.message ?? e).split('\n')[0])
} finally {
    const status = await put(page, { app: APP, avatarId: original })
    const after = await me(page)
    check('the account is put back', status === 200 && (after?.choice?.avatarId ?? null) === original, `${status} ${JSON.stringify(after?.choice)}`)
}
check('no page errors', errors.length === 0, errors.join(' | '))

await browser.close()
console.log(failed ? `${failed} FAILED` : 'ALL PASS')
process.exit(failed ? 1 : 0)
