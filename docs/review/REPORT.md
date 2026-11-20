This is the gs-app-gaps review (2026-11-20). It records what was built against the brief and what was moved to the nightly board.

## Built

- **Workshop screen** (`src/routes/workshop.tsx`) now has three tabs:
  - Browse: search, kind, game and sort filters (`~/components/select`), paging, and Subscribe/Unsubscribe.
  - Subscribed: pause/resume, an auto-update toggle, all vs listed scope with a per-game picker (`POST /workshop/scope {game,on}`), Unsubscribe, and a sentence saying why a pack does not load.
  - My packs: edit name and summary, choose where the pack is offered (none, any, or picked games), and delete with a confirmation.
- **Pure helpers** in `src/lib/gamespice/workshop.ts`, with vitest tests.
- **Client calls:** `workshopCatalog`, `workshopOwned`, `workshopItemPatch`, `workshopItemDelete`. The contract was synced from website-city `app-workshop-crud`.
- **available:false root cause:** packs 300 and 301 only have runsCode versions waiting for review. The site now sends `unavailable: awaiting_review`, and the app shows that reason.

## Site branch (website-city app-workshop-crud, not merged)

- `GET /api/app/v1/workshop/catalog` and `GET /workshop/owned`.
- `PATCH` and `DELETE /workshop/item`.
- `SetWorkshopCompat`, shared with tRPC.
- node:test tests.

## Not done (on the nightly board)

- `site-app-workshop-crud-1`: merge and deploy. The new endpoints cannot be called on tmcdev.net until this happens.
- `gamespice-app-workshop-publish-1`: upload from the app through a Rust command.
- `gamespice-app-workshop-social-1`: ratings and comments in the app.
- `gamespice-app-workshop-e2e-1`: Playwright flows for the Workshop, games, mods, sandboxes, the server browser, and avatar save/load.
- `dev-site-avatar-1`: the library API returns 200 on dev. The app page and save/load are not verified yet.

## Store, Stats and Workshop e2e (branch gs-app-store-stats, 2026-11-20)

- **Store** (`src/routes/store.tsx`):
  - A directory of game stores.
  - A per-game store with search, kind, currency and show filters, and sort.
  - Item cards with art, or a per-kind gradient and icon fallback when there is no image.
  - An item dialog: buying with points asks for confirmation and sends an idempotency key; a credits item opens the website in the external browser; an equip button.
  - A loadout panel and an inventory tab.
- **Stats** (`src/routes/stats.tsx`):
  - Totals, 30-day activity, a play-share bar and per-game cards.
  - A per-game drill-down: my stats, achievements (unlocked and locked), and leaderboards with your own row.
  - A summary on `/me`.
- **Site** (website-city `app-store-stats`, 96ba111ea, not merged): `/store/directory` and `/me/stats` with contract schemas.
- **Workshop e2e:** `e2e/review/workshop.mjs` runs on fixtures and passes 11/11. It does not cover game, mod, sandbox, server or avatar flows.
- **Screenshots:**
  - Store: `store.png`, `store_101.png`, `flow-store-confirm.png`, `flow-store-equipped.png`, `store_inventory.png`.
  - Stats: `stats.png`, `stats_101.png`, `stats_101_achievements.png`, `stats_101_leaderboards.png`.
  - `/me`: `me.png`.
  - Workshop: `flow-workshop-browse.png`, `flow-workshop-scope.png`, `flow-workshop-owned.png`.
  - Narrow (760 px): the `-narrow` variants.
- **Visual judge:** not run. The dot-game judge only takes GPU video clips with a weapon/avatar rubric. These screens were reviewed by eye from the screenshots instead.
- **Nightly:**
  - `website-city-app-store-stats-merge`
  - `gamespice-app-workshop-e2e-2`
  - `gamespice-app-match-history`
  - `gamespice-app-i18n`
  - `dot-game-visual-judge-ui-stills`
  - `gamespice-app-review-outfits-fixture`
