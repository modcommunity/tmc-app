/**
 * How a game is named on screen.
 *
 * The API sends one name per app and it is the FULL one — `mapApp` in
 * website-city's `lib/app-api/content.ts` and the `/facets` handler both send
 * `App.name` rather than `App.nameShort`. That is deliberate and it is the
 * whole rule: the app has no chrome built around a chosen game, so it has no
 * use for an abbreviation, and `nameShort` is unset or blank for most rows —
 * which is what filled the browse filter's game picker with unreadable, and
 * selectable, empty options.
 *
 * This exists for the case the contract cannot rule out: an installed build
 * talking to an older server, or a catalogue row whose name is genuinely blank.
 * A dropdown option with no text is one a user can neither read nor undo, so
 * something identifying always goes on it.
 */
export function appLabel(app: { id: number; name: string }): string {
    const name = app.name.trim()

    return name.length > 0 ? name : `Game #${app.id}`
}
