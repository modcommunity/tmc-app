/**
 * What a left click on Play does once the facts are in: the best build wins.
 *
 *   * `native`  — installed: launch it (a failure then falls back to web, warned).
 *   * `chooser` — not installed but installable here: show the dialog with
 *                 install-then-run preselected and the web build one click away.
 *   * `web`     — native cannot exist on this platform: web, with a warning.
 *   * `dialog`  — nothing automatic is possible: the ordinary dialog.
 */
export type QuickAction = 'native' | 'chooser' | 'web' | 'dialog'

export function quickAction(f: {
    canNative: boolean
    canInstall: boolean
    canWeb: boolean
}): QuickAction {
    if (f.canNative) return 'native'
    if (f.canInstall) return 'chooser'
    if (f.canWeb) return 'web'

    return 'dialog'
}
