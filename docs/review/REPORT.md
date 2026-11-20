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
