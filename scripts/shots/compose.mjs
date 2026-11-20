/**
 * Turn the raw captures in out/raw into everything the website and the press
 * kit use. Deterministic: same raws in, same bytes out (fixed fonts, no clock).
 *
 *   node compose.mjs [--site <website-gs checkout>] [--slide-theme dark|light]
 *
 * out/site/<slot>-<theme>-<800|1600>.webp       raw shots for the /app page
 * out/frames/<slot>-<theme>@<1x|2x>.webp        framed marketing shots (chrome,
 *                                               ember backdrop, shadow, callouts)
 * out/slides/<nn>-<slot>.png|.webp              1920x1080 tour slides
 * out/slides/<slot>-1280.webp                   the same, for the site slideshow
 * out/presentation/gamespice-app-tour.pdf|.pptx the deck (pptx: art + native text)
 *
 * --site copies site shots to public/app/shots and slides, PDF and deck to
 * public/app/presentation, plus slides.json (order, titles, captions).
 */
import { mkdirSync, readFileSync, writeFileSync, existsSync, copyFileSync, rmSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { chromium } from 'playwright'
import sharp from 'sharp'
import PptxGenJS from 'pptxgenjs'
import JSZip from 'jszip'
import { screens, tour, themes, SIZE } from './screens.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const arg = (name, dflt) => {
    const i = process.argv.indexOf(`--${name}`)
    return i < 0 ? dflt : process.argv[i + 1]
}
const site = arg('site')
const slideTheme = arg('slide-theme', 'dark')
const out = join(here, 'out')
const raw = join(out, 'raw')
const report = existsSync(join(raw, 'capture.json')) ? JSON.parse(readFileSync(join(raw, 'capture.json'), 'utf8')) : []
const calloutsOf = (slot, theme) => report.find((r) => r.slot === slot && r.theme === theme)?.callouts ?? []
const rawPath = (slot, theme) => join(raw, `${slot}-${theme}.png`)
const have = (slot, theme) => existsSync(rawPath(slot, theme))
for (const d of ['site', 'frames', 'slides', 'presentation', 'html']) {
    rmSync(join(out, d), { recursive: true, force: true })
    mkdirSync(join(out, d), { recursive: true })
}

const DECK = {
    file: 'gamespice-app-tour',
    title: 'GameSpice App',
    tagline: 'Play, mod and host your games from one app.',
    download: 'gamespice.net/app',
    link: 'Open any gs:// link to jump straight to a game, mod or server.',
}
const fonts = join(here, '../../src/assets/fonts')
const url = (p) => pathToFileURL(p).href
const mark = url(join(here, 'brand/gamespice-mark.svg'))
const tmcMark = `<svg viewBox="0 0 24 24" width="40" height="40" aria-hidden="true"><rect width="24" height="24" rx="6" fill="#22c55e"/><path d="M6 8h12M12 8v10" stroke="#04130a" stroke-width="2.6" stroke-linecap="round"/></svg>`
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c])

const CSS = `
@font-face { font-family: 'Plus Jakarta Sans'; font-weight: 200 800; src: url('${url(join(fonts, 'plus-jakarta-sans-latin-wght-normal.woff2'))}') format('woff2'); }
@font-face { font-family: 'DM Sans'; font-weight: 100 1000; src: url('${url(join(fonts, 'dm-sans-latin-wght-normal.woff2'))}') format('woff2'); }
* { box-sizing: border-box; margin: 0; padding: 0; }
html, body { background: transparent; }
body { font-family: 'DM Sans', sans-serif; -webkit-font-smoothing: antialiased; }
h1, h2, .brand { font-family: 'Plus Jakarta Sans', sans-serif; }
.ember { background: linear-gradient(135deg, #ffb02e 0%, #f2542d 48%, #b8193c 100%); }
.night { background: radial-gradient(1200px 700px at 85% 110%, rgba(242,84,45,.38), transparent 70%),
                     radial-gradient(900px 600px at 0% -10%, rgba(255,176,46,.20), transparent 70%),
                     linear-gradient(160deg, #1c0e0b, #0d0807 70%); }
.win { position: absolute; border-radius: 14px; overflow: hidden;
       box-shadow: 0 30px 80px rgba(20,4,2,.55), 0 8px 24px rgba(20,4,2,.35), 0 0 0 1px rgba(255,255,255,.10); }
.win.light { box-shadow: 0 30px 80px rgba(60,10,4,.40), 0 8px 24px rgba(60,10,4,.25), 0 0 0 1px rgba(43,20,16,.18); }
.bar { height: var(--bar); display: flex; align-items: center; gap: 8px; padding: 0 14px;
       font: 600 calc(var(--bar) * .38)/1 'Plus Jakarta Sans', sans-serif; }
.win.dark .bar { background: #1a1210; color: #e9d9d2; border-bottom: 1px solid rgba(255,255,255,.07); }
.win.light .bar { background: #f4ece8; color: #3a2420; border-bottom: 1px solid rgba(43,20,16,.10); }
.bar img { height: calc(var(--bar) * .58); }
.bar .ctl { margin-left: auto; display: flex; gap: calc(var(--bar) * .45); opacity: .55; }
.bar .ctl i { width: calc(var(--bar) * .28); height: calc(var(--bar) * .28); border: 1.5px solid currentColor; border-radius: 2px; }
.shot { display: block; position: relative; }
.shot img { display: block; width: 100%; height: 100%; }
.co { position: absolute; border: 3px solid #ff7a2d; border-radius: 10px; box-shadow: 0 0 0 4px rgba(255,122,45,.25); }
.pill { position: absolute; white-space: nowrap; transform: translateX(-50%); padding: .45em .95em; border-radius: 999px;
        color: #fff; font: 700 var(--pill)/1.1 'Plus Jakarta Sans', sans-serif; box-shadow: 0 8px 22px rgba(80,10,4,.45); }
`

/** A window with the app shot inside, `w` wide, top-left at (x, y); callouts scaled from capture coords. */
function windowHtml({ slot, theme, x, y, w, bar, pill }) {
    const h = Math.round((w * SIZE.height) / SIZE.width)
    const k = w / SIZE.width
    const co = calloutsOf(slot, theme).map((c) => {
        const pad = 6
        const bx = c.x * k - pad, by = c.y * k - pad, bw = c.width * k + pad * 2, bh = c.height * k + pad * 2
        const below = by + bh + pill * 3 < h
        const py = below ? by + bh + pill * 1.2 : by - pill * 3.1
        const px = Math.min(Math.max(bx + bw / 2, pill * 9), w - pill * 9)
        return `<div class="co" style="left:${bx}px;top:${by}px;width:${bw}px;height:${bh}px"></div>` +
            `<div class="pill ember" style="left:${px}px;top:${py}px">${esc(c.label)}</div>`
    }).join('')
    return `<div class="win ${theme}" style="left:${x}px;top:${y}px;width:${w}px;--bar:${bar}px;--pill:${pill}px">
  <div class="shot" style="width:${w}px;height:${h}px"><img src="${url(rawPath(slot, theme))}" alt="">${co}</div>
</div>`
}

const page = (body, w, h) => `<!doctype html><html><head><meta charset="utf-8"><style>${CSS}
.page { position: relative; width: ${w}px; height: ${h}px; overflow: hidden; }</style></head><body>${body}</body></html>`

// ---- slides ---------------------------------------------------------------

const W = 1920, H = 1080
const SHOT = { x: 320, y: 236, w: 1280, bar: 0 } // 1280x800 shot: bottom at 1036
const TEXT = { x: 320, title: 78, caption: 166 }
const list = tour
    .map((slot) => screens.find((s) => s.slot === slot))
    .filter((s) => s && s.enabled !== false && have(s.slot, slideTheme))
const slides = [
    { slot: 'intro', title: DECK.title, caption: DECK.tagline },
    ...list.map((s) => ({ slot: s.slot, title: s.title, caption: s.caption, shot: true })),
    { slot: 'outro', title: 'Get the GameSpice App', caption: `Free for Windows and Linux at ${DECK.download}` },
]

function slideBody(s, i, { text = true } = {}) {
    const n = `${String(i).padStart(2, '0')} / ${String(slides.length - 2).padStart(2, '0')}`
    const corner = `<div class="brand" style="position:absolute;right:${TEXT.x}px;top:92px;display:flex;align-items:center;gap:12px;color:#f6e4dc;font-weight:700;font-size:24px">
        <img src="${mark}" alt="" style="height:44px">GameSpice App</div>`
    const rule = `<div class="ember" style="position:absolute;left:0;top:0;width:100%;height:8px"></div>`
    if (s.slot === 'intro') {
        return `<div class="page night">${rule}
  <img src="${mark}" alt="" style="position:absolute;left:${W / 2 - 170}px;top:200px;width:340px">
  ${text ? `<h1 style="position:absolute;left:0;right:0;top:590px;text-align:center;color:#fff;font-size:120px;font-weight:800;letter-spacing:-.02em">GameSpice <span style="background:linear-gradient(90deg,#ffb02e,#f2542d 55%,#ff5a6e);-webkit-background-clip:text;color:transparent">App</span></h1>
  <p style="position:absolute;left:0;right:0;top:760px;text-align:center;color:#f3d6cb;font-size:40px">${esc(s.caption)}</p>` : ''}
</div>`
    }
    if (s.slot === 'outro') {
        return `<div class="page night">${rule}
  <img src="${mark}" alt="" style="position:absolute;left:${W / 2 - 120}px;top:120px;width:240px">
  ${text ? `<h1 style="position:absolute;left:0;right:0;top:400px;text-align:center;color:#fff;font-size:96px;font-weight:800;letter-spacing:-.02em">${esc(s.title)}</h1>
  <p style="position:absolute;left:0;right:0;top:540px;text-align:center;color:#f3d6cb;font-size:40px">Free for Windows and Linux at <b style="color:#ffb02e">${esc(DECK.download)}</b></p>
  <p style="position:absolute;left:0;right:0;top:620px;text-align:center;color:#f3d6cb;font-size:34px">${esc(DECK.link).replace('gs://', '<b style="color:#ff7a2d">gs://</b>')}</p>
  <div class="brand" style="position:absolute;left:${W / 2 - 190}px;top:800px;width:380px;height:84px;border-radius:20px;border:2px solid rgba(34,197,94,.55);background:rgba(34,197,94,.08);display:flex;align-items:center;justify-content:center;gap:16px;color:#fff;font-size:34px;font-weight:700">${tmcMark}<span>Powered by <span style="color:#4ade80">TMC</span></span></div>` : ''}
</div>`
    }
    return `<div class="page night">${rule}${corner}
  ${text ? `<div class="brand" style="position:absolute;left:${TEXT.x}px;top:46px;color:#ff9a5a;font-size:22px;font-weight:700;letter-spacing:.14em">${n}</div>
  <h2 style="position:absolute;left:${TEXT.x}px;top:${TEXT.title}px;color:#fff;font-size:68px;font-weight:800;letter-spacing:-.015em;line-height:1.1">${esc(s.title)}</h2>
  <p style="position:absolute;left:${TEXT.x}px;top:${TEXT.caption}px;width:${W - 2 * TEXT.x}px;color:#f3d6cb;font-size:30px;line-height:1.3">${esc(s.caption)}</p>` : ''}
  ${windowHtml({ slot: s.slot, theme: slideTheme, x: SHOT.x, y: SHOT.y, w: SHOT.w, bar: SHOT.bar, pill: 24 })}
</div>`
}

// ---- render ---------------------------------------------------------------

const browser = await chromium.launch()
const tab = await browser.newPage({ viewport: { width: W, height: H }, deviceScaleFactor: 1 })
async function render(name, html, size, scale = 1) {
    const file = join(out, 'html', `${name}.html`)
    writeFileSync(file, html)
    await tab.setViewportSize(size)
    const p = scale === 1 ? tab : await browser.newPage({ viewport: size, deviceScaleFactor: scale })
    await p.goto(url(file))
    await p.evaluate(() => document.fonts.ready)
    await p.waitForFunction(() => [...document.images].every((i) => i.complete && i.naturalWidth > 0))
    const png = await p.screenshot({ type: 'png', omitBackground: true })
    if (p !== tab) await p.close()
    return png
}

// Site shots: straight resizes of the raws.
for (const s of screens) for (const theme of themes) {
    if (!have(s.slot, theme)) continue
    for (const w of [800, 1600]) {
        await sharp(rawPath(s.slot, theme)).resize(w).webp({ quality: 86, effort: 6 })
            .toFile(join(out, 'site', `${s.slot}-${theme}-${w}.webp`))
    }
}

// Marketing frames: 960x660 CSS, window 800 wide on the ember backdrop.
for (const s of screens) for (const theme of themes) {
    if (!have(s.slot, theme)) continue
    const body = `<div class="page ember">
  <div style="position:absolute;inset:0;background:radial-gradient(600px 400px at 20% 0%,rgba(255,255,255,.22),transparent 70%)"></div>
  ${windowHtml({ slot: s.slot, theme, x: 80, y: 80, w: 800, bar: 0, pill: 15 })}</div>`
    for (const scale of [1, 2]) {
        const png = await render(`frame-${s.slot}-${theme}`, page(body, 960, 660), { width: 960, height: 660 }, scale)
        await sharp(png).webp({ quality: 88, effort: 6 }).toFile(join(out, 'frames', `${s.slot}-${theme}@${scale}x.webp`))
    }
}

// Slides, their text-free art (for the pptx), and the PDF.
const art = []
for (const [i, s] of slides.entries()) {
    const stem = `${String(i).padStart(2, '0')}-${s.slot}`
    const png = await render(`slide-${stem}`, page(slideBody(s, i), W, H), { width: W, height: H })
    writeFileSync(join(out, 'slides', `${stem}.png`), await sharp(png).png({ compressionLevel: 9 }).toBuffer())
    await sharp(png).webp({ quality: 88, effort: 6 }).toFile(join(out, 'slides', `${stem}.webp`))
    await sharp(png).resize(1280).webp({ quality: 84, effort: 6 }).toFile(join(out, 'slides', `${s.slot}-1280.webp`))
    await sharp(png).webp({ quality: 88, effort: 6 }).toFile(join(out, 'slides', `${s.slot}-1920.webp`))
    const bare = await render(`art-${stem}`, page(slideBody(s, i, { text: false }), W, H), { width: W, height: H })
    art.push(await sharp(bare).jpeg({ quality: 90 }).toBuffer())
}
const deck = slides.map((s, i) => slideBody(s, i).replace('class="page', 'style="page-break-after:always" class="page')).join('\n')
writeFileSync(join(out, 'html', 'deck.html'), page(deck, W, H))
await tab.goto(url(join(out, 'html', 'deck.html')))
await tab.evaluate(() => document.fonts.ready)
await tab.waitForFunction(() => [...document.images].every((i) => i.complete && i.naturalWidth > 0))
let pdf = Buffer.from(await tab.pdf({ width: `${W}px`, height: `${H}px`, printBackground: true, pageRanges: `1-${slides.length}` }))
// Chromium stamps the clock into the PDF; pin it (same length, so offsets hold).
pdf = Buffer.from(pdf.toString('latin1').replace(/\(D:\d{14}/g, '(D:20261120180000'), 'latin1')
writeFileSync(join(out, 'presentation', `${DECK.file}.pdf`), pdf)
await browser.close()

// PowerPoint: each slide is its art as a background plus native, editable text.
const px = (v) => v / 144 // 1920 px = 13.333 in
const pptx = new PptxGenJS()
pptx.layout = 'LAYOUT_WIDE'
pptx.title = 'GameSpice App tour'
pptx.company = 'GameSpice'
const head = { fontFace: 'Plus Jakarta Sans', bold: true, color: 'FFFFFF', margin: 0 }
const body = { fontFace: 'DM Sans', color: 'F3D6CB', margin: 0 }
for (const [i, s] of slides.entries()) {
    const sl = pptx.addSlide()
    sl.background = { data: `image/jpeg;base64,${art[i].toString('base64')}` }
    const alt = s.shot ? `${s.title}: ${s.caption}` : s.title
    sl.addNotes(alt)
    if (s.slot === 'intro') {
        sl.addText('GameSpice App', { ...head, x: 0, y: px(590), w: px(W), h: px(150), fontSize: 90, align: 'center' })
        sl.addText(s.caption, { ...body, x: 0, y: px(760), w: px(W), h: px(60), fontSize: 30, align: 'center' })
    } else if (s.slot === 'outro') {
        sl.addText(s.title, { ...head, x: 0, y: px(400), w: px(W), h: px(120), fontSize: 72, align: 'center' })
        sl.addText([{ text: 'Free for Windows and Linux at ' }, { text: DECK.download, options: { bold: true, color: 'FFB02E', hyperlink: { url: `https://${DECK.download}` } } }],
            { ...body, x: 0, y: px(540), w: px(W), h: px(60), fontSize: 30, align: 'center' })
        sl.addText(DECK.link, { ...body, x: 0, y: px(620), w: px(W), h: px(54), fontSize: 26, align: 'center' })
        sl.addText('Powered by TMC', { ...head, x: px(W / 2 - 190), y: px(800), w: px(380), h: px(84), fontSize: 26, align: 'center', color: 'FFFFFF',
            line: { color: '22C55E', width: 1.5 }, rectRadius: 0.15 })
    } else {
        sl.addText(`${String(i).padStart(2, '0')} / ${String(slides.length - 2).padStart(2, '0')}`, { ...head, x: px(TEXT.x), y: px(46), w: px(300), h: px(30), fontSize: 16, color: 'FF9A5A', charSpacing: 3 })
        sl.addText(s.title, { ...head, x: px(TEXT.x), y: px(TEXT.title), w: px(1000), h: px(84), fontSize: 51 })
        sl.addText(s.caption, { ...body, x: px(TEXT.x), y: px(TEXT.caption), w: px(W - 2 * TEXT.x), h: px(46), fontSize: 22.5 })
    }
}
// Pin every timestamp in the zip and in docProps so the deck is byte-stable.
const zip = await JSZip.loadAsync(await pptx.write({ outputType: 'nodebuffer' }))
const fixed = new Date(Date.UTC(2026, 10, 20, 18))
const core = await zip.file('docProps/core.xml').async('string')
zip.file('docProps/core.xml', core.replace(/\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(\.\d+)?Z/g, '2026-11-20T18:00:00Z'))
for (const f of Object.values(zip.files)) f.date = fixed
writeFileSync(join(out, 'presentation', `${DECK.file}.pptx`),
    await zip.generateAsync({ type: 'nodebuffer', compression: 'DEFLATE', compressionOptions: { level: 9 } }))

const manifest = slides.map(({ slot, title, caption }) => ({ slot, title, caption }))
writeFileSync(join(out, 'presentation', 'slides.json'), `${JSON.stringify({ theme: slideTheme, slides: manifest }, null, 2)}\n`)

if (site) {
    const shots = join(site, 'public/app/shots')
    const pres = join(site, 'public/app/presentation')
    mkdirSync(shots, { recursive: true })
    rmSync(pres, { recursive: true, force: true })
    mkdirSync(pres, { recursive: true })
    // Only the slots the site already shows; adding one there is a site change.
    for (const f of (await import('node:fs')).readdirSync(join(out, 'site'))) {
        if (existsSync(join(shots, f))) {
            // Replace, not overwrite: the checkout's files may belong to another user.
            rmSync(join(shots, f))
            copyFileSync(join(out, 'site', f), join(shots, f))
        }
    }
    for (const s of slides) {
        copyFileSync(join(out, 'slides', `${s.slot}-1280.webp`), join(pres, `${s.slot}-1280.webp`))
        copyFileSync(join(out, 'slides', `${s.slot}-1920.webp`), join(pres, `${s.slot}-1920.webp`))
    }
    for (const f of [`${DECK.file}.pdf`, `${DECK.file}.pptx`, 'slides.json']) copyFileSync(join(out, 'presentation', f), join(pres, f))
}
console.log(`composed ${slides.length} slides, ${report.length} captures${site ? `, copied to ${site}` : ''}`)
