/**
 * Build the README images from the raw captures in out/raw (dark theme):
 *
 *   docs/banner.png            1280x640, the family banner size
 *   docs/screenshots/*.webp    the Previews section, 1200 wide
 *
 *   node make-banner.mjs [--raw <dir>] [--no-banner] [--no-shots]
 *
 * Capture first (see README.md here), then run this; no network needed. The
 * banner is HTML rendered by Chromium: a capture as the background, a dark
 * gradient over it, the mark and name top left, the feature pills along the
 * bottom. Edit PILLS below when the feature list changes.
 */
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { createRequire } from 'node:module'

const here = dirname(fileURLToPath(import.meta.url))
const repo = join(here, '..', '..')
const req = createRequire(join(here, 'package.json'))
const { chromium } = req('playwright')
const sharp = req('sharp')

const arg = (name, dflt) => {
    const i = process.argv.indexOf(`--${name}`)
    return i < 0 ? dflt : process.argv[i + 1]
}
const raw = arg('raw', join(here, 'out', 'raw'))
const docs = join(repo, 'docs')
mkdirSync(join(here, 'out'), { recursive: true })

const BACKGROUND = 'workshop'
const PILLS = [
    'Avatar editor', 'Workshop', 'Store', 'Stats', 'Server browser', 'RCON',
    'Mods and sandboxes', 'Auto-updates',
    'Windows', 'macOS', 'Linux', 'Android', 'iOS',
]

/** slot, file, alt text, caption: the Previews table reads from here in this order. */
export const SHOTS = [
    ['play', 'home', 'The Play screen with the game cards', 'Play: your games, quick play and servers'],
    ['avatar', 'avatar-editor', 'The avatar editor with a 3D preview', 'Avatar editor'],
    ['workshop', 'workshop', 'The Workshop pack browser', 'Workshop'],
    ['store', 'store', 'The Store', 'Store'],
    ['servers', 'server-browser', 'The server browser', 'Server browser'],
    ['monitor', 'server-monitor', 'The server monitor window for a Left 4 Dead 2 server', 'Server monitor: live latency, players and RCON'],
    ['sandboxes', 'sandboxes', 'The sandboxes screen with mod sets', 'Sandboxes: keep mod sets apart'],
    ['stats', 'stats', 'The Stats page', 'Stats'],
]

async function banner() {
    const fonts = join(repo, 'src', 'assets', 'fonts')
    const url = (p) => pathToFileURL(p).href
    const mark = `data:image/svg+xml;base64,${readFileSync(join(here, 'brand', 'gamespice-mark.svg')).toString('base64')}`
    const bg = url(join(raw, `${BACKGROUND}-dark.png`))
    const html = `<!doctype html><meta charset="utf-8"><style>
@font-face { font-family: 'Plus Jakarta Sans'; font-weight: 200 800; src: url('${url(join(fonts, 'plus-jakarta-sans-latin-wght-normal.woff2'))}') format('woff2'); }
@font-face { font-family: 'DM Sans'; font-weight: 100 1000; src: url('${url(join(fonts, 'dm-sans-latin-wght-normal.woff2'))}') format('woff2'); }
* { box-sizing: border-box; margin: 0 }
body { width: 1280px; height: 640px; position: relative; overflow: hidden; background: #120a08; font-family: 'DM Sans', sans-serif; color: #fff4ea }
.bg { position: absolute; inset: 0; background: url('${bg}') -250px -70px / 1500px auto no-repeat; filter: saturate(1.1) }
.shade { position: absolute; inset: 0; background:
  linear-gradient(90deg, rgba(14,8,6,.8) 0%, rgba(14,8,6,.3) 55%, rgba(14,8,6,.1) 100%),
  linear-gradient(180deg, rgba(14,8,6,.9) 0%, rgba(14,8,6,.35) 32%, rgba(14,8,6,.3) 55%, rgba(14,8,6,.95) 100%),
  radial-gradient(ellipse at 85% 45%, rgba(242,84,45,.16), transparent 60%) }
.head { position: absolute; left: 64px; top: 52px; display: flex; align-items: center; gap: 18px }
.head img { width: 84px; height: 84px; filter: drop-shadow(0 0 22px rgba(255,140,50,.45)) }
.head h1 { font: 800 56px/1 'Plus Jakarta Sans', sans-serif; letter-spacing: -.5px }
.head h1 span { color: #ff8a3d }
.sub { position: absolute; left: 66px; top: 160px; font-size: 24px; color: #e9cdb8 }
.pills { position: absolute; left: 64px; right: 64px; bottom: 56px; display: flex; flex-wrap: wrap; gap: 12px }
.pill { padding: 10px 20px; border-radius: 999px; font: 600 21px/1 'DM Sans', sans-serif; color: #fff4ea;
  background: rgba(60,28,18,.78); border: 1.5px solid rgba(255,138,61,.55); box-shadow: 0 2px 10px rgba(0,0,0,.4) }
.pill.os { background: rgba(242,84,45,.88); border-color: rgba(255,190,120,.7) }
.by { position: absolute; right: 64px; top: 70px; font: 700 15px 'DM Sans', sans-serif; letter-spacing: 2.5px; color: #cfb29c }
</style>
<div class="bg"></div><div class="shade"></div>
<div class="head"><img src="${mark}"><h1>GameSpice <span>App</span></h1></div>
<div class="sub">Play, mod and host your games from one app.</div>
<div class="by">POWERED BY TMC</div>
<div class="pills">${PILLS.map((p) => `<div class="pill${['Windows', 'macOS', 'Linux', 'Android', 'iOS'].includes(p) ? ' os' : ''}">${p}</div>`).join('')}</div>`
    const browser = await chromium.launch()
    const page = await browser.newPage({ viewport: { width: 1280, height: 640 }, deviceScaleFactor: 1 })
    const file = join(here, 'out', 'banner.html')
    writeFileSync(file, html)
    await page.goto(pathToFileURL(file).href, { waitUntil: 'load' })
    await page.evaluate(() => document.fonts.ready)
    const png = await page.screenshot({ type: 'png' })
    await browser.close()
    await sharp(png).png({ compressionLevel: 9, palette: false }).toFile(join(docs, 'banner.png'))
    console.log('docs/banner.png')
}

async function shots() {
    mkdirSync(join(docs, 'screenshots'), { recursive: true })
    for (const [slot, file] of SHOTS) {
        const out = join(docs, 'screenshots', `${file}.webp`)
        await sharp(join(raw, `${slot}-dark.png`)).resize({ width: 1200 }).webp({ quality: 80, effort: 6 }).toFile(out)
        console.log(`docs/screenshots/${file}.webp`)
    }
}

if (!process.argv.includes('--no-banner')) await banner()
if (!process.argv.includes('--no-shots')) await shots()
