# Authentication and the API contract

## Authentication

**OAuth 2.0 Device Authorization Grant (RFC 8628) with PKCE.** The user signs in on the real website in their real browser; the app never sees a password.

```
app                                    website-city
 │ POST /auth/device {client, challenge}
 │ ─────────────────────────────────────▶  AppDeviceGrant (PENDING)
 │ ◀───────────────────────────────────── {deviceCode, userCode, uri}
 │
 │ opens the SYSTEM browser at /login/device?code=XXXX-XXXX
 │                                          user signs in, approves
 │                                          appDevice.approve → APPROVED
 │ POST /auth/token {deviceCode, verifier}
 │ ─────────────────────────────────────▶  atomic APPROVED → CLAIMED
 │ ◀───────────────────────────────────── {accessToken, refreshToken, user}
```

Non-obvious properties, all load-bearing:

- **PKCE is not optional.** A stolen `deviceCode` is inert without the verifier, which never leaves the app's memory. A wrong verifier **destroys** the grant.
- **The grant is claimed atomically** (`updateMany` with `status: 'APPROVED'` in the WHERE). The app polls on a timer *and* on the deep link, so two claims racing is the normal case, not an edge case.
- **Refresh tokens rotate, and reuse kills the family.** A retired token coming back means two parties hold one lineage; every device is revoked and `reuseDetected` is set. `ApiClient::refresh` holds a mutex so the app never does this to itself.
- **The access token is memory-only.** One hour, treated as stale 60s early.
- **`tmc://` is a wake-up, not a credential.** Any app can claim a custom scheme, so nothing is read out of the URL — it only triggers an immediate poll.
- **System browser, never a webview.** An embedded login hides the address bar and the password manager, which is what makes it indistinguishable from phishing.

Website-side files: `src/server/auth/app-token.ts`, `prisma/models/app-device.prisma`, `src/app/api/app/v1/auth/*`, `src/server/api/routers/app-device.ts`, `src/app/[locale]/login/device/page.tsx`.

## The API contract

`website-city/src/types/app-api/contract.ts` is the **source of truth**; `src/lib/api/contract.ts` is a **verbatim copy**. When the server side changes, copy the file across — do not hand-edit the app's copy.

Every response is parsed through these schemas before it reaches a component, so a drift is a loud error on the first request instead of an `undefined` three screens deep. That is the entire reason the duplication is acceptable.

Why a separate REST API rather than the website's tRPC:

- The website's routers return **Prisma payloads** shaped by what a page renders. An installed app is not redeployed with the server, so its wire format has to be versioned and narrow.
- Every content type has a **different** row shape. The app draws one grid; the normalisation into `ContentSummary` has to happen once, server-side.
- Rust calls it too.

Endpoints: `/auth/device`, `/auth/token`, `/auth/refresh`, `/auth/revoke`, `/me` (GET + PATCH), `/browse`, `/content/:kind/:id`, `/facets`, `/avatar` (GET + PUT), `/avatar/schema`, `/avatar/actions` (GET + PUT), `/avatar/wearables` (GET + PUT), `/apps`, `/play/launch`, `/version`.

**The game backbone's player routes are in the contract too**, section "Social": `/friends` (+ `/requests`, `/request`, `/respond`, `/cancel`, `/remove`), `/party/mine`, `/party/invites`, `/party/invite/respond`, `/party/leave`, `/stats/top`, `/stats/me` and `/stats/defs` (the picker; a 404 from an older site falls back to the held keys) — website-city's `docs/api/app-social.md` routes. `client.ts` parses them with the contract's `*Response` schemas; `respond`/`cancel`/`remove` answer `data: null` and are parsed with `z.null()`, and an accepted `party/invite/respond` answers the join (`PartyInviteRespondResponse`). `lib/api/social.ts` keeps only the UI's short type names and display helpers; its test parses the site's documented examples through the contract schemas. Stat `kind` stays a string in the contract (`StatKindVals` is labels only), so a new kind still ranks. The app never POSTs `/presence`: only a game token places its player.

**The Store and Stats screens** read `/store/directory` (every game with a store), `/store/front` (items, wallet, owned, held, subscribed), `/store/loadout`, `/store/purchases` and `/me/stats` (totals, a 30-day activity series, per-game days played and achievement counts), and write `POST /store/purchase` (points only, with an `idemKey` per attempt so a retry never buys twice) and `PUT /store/loadout`. A CREDITS item is never bought here: the dialog opens the site's store page in the external browser through `openUrl`. `/store/directory` and `/me/stats` come from website-city branch `app-store-stats`; until it is deployed, those two answer 404 on tmcdev.net and the screens show their error state.

**`/apps` exists because the website has no page that could answer it.** Its chrome is built around one chosen game, so "what games are there?" is a question it answers by making you choose first; the app opens on a flat grid across the whole catalogue and that IS its front door. The endpoint carries the play vocabulary (`directPlay`, the launch options, which modes an app supports) that `/facets` has no reason to know about, and per-app counts from three `groupBy`s over the page's ids rather than a relation `_count` per row.

**`/play/launch` resolves one launch, server-side, and answers `null` for every refusal.** The app never decides whether a game can be started by reading columns off a catalogue row — it asks — so the button that is drawn and the launch that happens agree by construction. `directPlay` is enforced there as well as hidden in the UI, because a client that ignores the field must not get a serverless launch out of it.

### The browse filters mirror the website's, deliberately

`BrowseQuerySchema` is a copy of `ServerBrowserPublicGetsInput` plus the mod/asset equivalents, field for field, so a filter someone used on the site is reachable here under the same name and with the same meaning. Where the two could disagree, **the website wins** — `hideFull` drops servers declaring no player limit because the site's SQL does, even though that is arguably wrong, because a filter that quietly returns more than the site did is worse than one that matches.

Two things this endpoint pins that the app cannot ask for, both server-decided exactly as they are on the website:

- **`hasPenalties: false`.** A live penalty hides an item everywhere. Missing here, the app was the one surface still listing `SPY_FLAG`'d servers — the redirect farms and fake-player-count boxes. Applied to `/browse` *and* `/content/:kind/:id`, since reaching a row by id never passes a list filter.
- **`SRV_STALE_ONLINE_SEC`.** `online` has no expiry, so a server that stopped being scanned asserts its last reading forever.

Not every declared field is implemented: **`timeRange` is accepted and ignored**, on this endpoint and always has been. The website windows aggregates in-process over a capped candidate set, which an infinite-scroll cursor cannot do — the ranking moves as the window moves, so rows repeat and vanish between pages. `downloads` falls back to `createdAt` for the same reason.

**`/facets` counts with the browse list predicate**, so a facet number equals the `total` the list shows when it is chosen. `online=0` (servers) counts the whole listing from the site rollups. Responses are cached in Redis under `appapi:facets:ver`; `INCR` it to drop them. Categories carry an `icon`, countries a `shortCode` (for the bundled flag SVGs) and `maps` lists popular maps.

**`/avatar/actions` equips emotes, poses and gestures** on the member's built-in figure: GET lists everything they may equip with its slot, PUT replaces one kind's ordered list (at most `max`). Server rows carry `countryCode` for the flag. `online=0` facets count with the list predicate, not the site rollups, which cover a shorter window.

**`/avatar/wearables` dresses the built-in figure**: GET lists every wearable the member may wear (`slot`, `bone`, `licence`, `equipped`, and `locked` when a store sells it and they have not bought it), PUT `{ wearableId, equipped }` wears one in its slot (replacing the one there) or takes it off. The panel's Wearables tab draws a stick figure with a marker on each worn slot; a rendered preview is not built.
