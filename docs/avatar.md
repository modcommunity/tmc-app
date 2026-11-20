# Avatar studio

The app edits avatars with the same code the website runs:
`@modcommunity/shared/avatar` (`AvatarEditor` for the member's global figure,
`AppAvatarEditor` for one game). This page is the app's side of it.

| Piece | Where |
| --- | --- |
| Pages (`/avatar`, `/games/:slug/avatar`), lazy-loaded | `src/routes/avatar-member.tsx`, `avatar-games.tsx` |
| Host (what the studio is given) and the two pages | `src/components/avatar-studio.tsx` |
| Loaders and the write mapping | `src/lib/avatar/studio-data.ts` |
| Studio class names mapped onto the app theme, scoped to `.avatar-studio` | `src/styles/avatar-studio.css` |
| IPC wrappers | `ipc.avatar*` in `src/lib/ipc/commands.ts`; `api.studioGet/studioPut` |
| Rust commands | `src-tauri/src/commands/avatar.rs`; logic in `src-tauri/core/src/avatar.rs` |

## The one rule still holds

The webview has no `fetch` and no token. Every read and write is an IPC call:

- `avatar_manifest`, `avatar_library(app_id)`: public TMC documents.
- `avatar_me(app_id)`, `avatar_me_put(body)`: the member's choice in one game.
- The global avatar, wearables, actions, outfits and points go through the
  existing authenticated `/api/app/v1` client (`api.studioGet/studioPut`).
- `avatar_asset(url, app_id, me)` returns a model file's bytes
  (`tauri::ipc::Response`). Rust rebuilds an allowlist from TMC's manifest,
  the game's library and (with `me`) the member's wearables, and refuses any
  other URL before a request leaves. This is what lets the studio's
  `loadModel` hook work without opening `connect-src` to TMC.

## CSP

`connect-src` gained `blob:` and nothing else (see `SECURITY.md`): three.js
decodes glTF textures through `blob:` URLs.

## Behaviour

- Signed out: a sign-in prompt, no studio.
- Avatars off for a game (library 404): "switched off" note.
- No WebGL: the studio's own stage shows its fallback message and the editor
  keeps working (three is imported lazily inside the studio, so app start-up and
  the Safari 13 build target are unaffected).
- A failed write reports 401 (signed out), 0 (no network) or 400 to the studio,
  which shows its own text.

## Not done here

- `shared` 4.18.0 must be published and the lockfile refreshed
  (`npm install @modcommunity/shared@^4.18.0`); until then use
  `npm run shared:local`.
- `PLATFORM_GAMES` (`src/lib/gamespice/games.ts`) is still a copy of the
  website's list.
