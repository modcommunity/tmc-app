Screenshots of the GameSpice App for the website and the press kit, and the marketing frames, tour slides, PDF and PowerPoint deck built from them. Two steps: `capture` drives the real app in a browser, `marketing` composes everything else from the captures without touching the network.

## Running

```sh
npm --prefix scripts/shots ci
# 1. the harness: the real frontend with the Tauri IPC mocked and seeded, on :1440
TMC_REVIEW_TOKEN=$(cat ~/.config/gamespice-shots/token) TMC_REVIEW_PORT=1440 \
  NODE_OPTIONS=--use-system-ca npx vite --config scripts/shots/vite.config.ts --configLoader runner &
# 2. raw captures, 1600x1000, light and dark, into scripts/shots/out/raw
npm run shots:capture -- --no-gpu                       # everything but the 3D avatar
(cd scripts/shots && ~/wt/gpu.sh cdp-run --serve 1440 -- node capture.mjs --gpu-only)   # avatar on a GPU box
# 3. frames, slides, PDF, pptx; --site copies into a website-gs checkout
npm run shots:marketing -- --site ~/wt/website-gs-shots
```

`capture.mjs --only play,servers --theme dark` re-shoots a few; `out/raw/capture.json` is merged by slot and theme, so partial runs keep the rest. Each entry lists console errors and, for callouts, where the target was. A missing callout target fails the run.

## What is real and what is seeded

- **Real:** the app's own UI, unchanged. Every server call goes to `tmcdev.net` with a dev account's token, which lives only in `~/.config/gamespice-shots/token`. That account is `spicedemo`, and it carries no personal data.
- **Seeded** (`seed.ts`): what lives on the user's machine. That covers sandboxes, game folders, the download queue, playtime and settings.
- **Seeded where dev is too thin to show the feature:**
  - The mod catalogue: the packs, their names and counts, and the demo owners (NovaRift, KiteRunner and others).
  - The server browse: six invented servers on `*.example.net`.
  - The latency trace: a deterministic series, not a measured ping.
- **Avatar model:** the real one, fetched from the host its manifest names (`/__shots-asset` in `vite.config.ts`, dev hosts only). The review harness would send it to the API host, where it is a 404.
- **Clock:** pinned to 2026-11-20 18:00 UTC.
- **Hidden:** the title bar's dev-API badge. It exists so that dev screenshots are not mistaken for bug reports about live data, and these shots are product shots.
- **Art:** the pack art (`art/*.webp`) is 640x360 crops of the banners of our own map packs (`<name>-maps/docs/banner.png`).

## The screen list

`screens.mjs` is the one list: slot, route, what to wait for, an optional action and callouts, and the slide title and caption. `tour` there sets the slide order.

Store and Stats are already in the list with `enabled: false`. They are on branch `gs-app-store-stats`; set `enabled` to true once that branch is on main, and both steps pick them up.

## Outputs (`out/`, gitignored)

| Path | What |
| --- | --- |
| `raw/<slot>-<theme>.png` | the 1600x1000 captures |
| `site/<slot>-<theme>-<800\|1600>.webp` | the /app page shots (`--site` copies the slots the site already has) |
| `frames/<slot>-<theme>@<1x\|2x>.webp` | 960x660 marketing frames: rounded window, ember backdrop, shadow, callouts |
| `slides/<nn>-<slot>.png\|.webp` | 1920x1080 tour slides: intro, one per screen, outro |
| `presentation/gamespice-app-tour.pdf` | the slides as a PDF, with real text |
| `presentation/gamespice-app-tour.pptx` | the deck: each slide is a text-free background image plus native, editable text |
| `presentation/slides.json` | order, titles and captions; the site's slideshow reads it |

The compose step is deterministic: the fonts are the app's own (`src/assets/fonts`, OFL), and there is no clock in the output. Chromium's PDF dates and the pptx zip and docProps times are pinned. Fonts are Plus Jakarta Sans for titles and DM Sans for body text. The mark is `brand/gamespice-mark.svg`, a copy of website-gs `public/brand/gamespice-mark.svg`.
