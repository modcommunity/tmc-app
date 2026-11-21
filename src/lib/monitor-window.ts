import {
    MONITOR_SIZE,
    monitorLabel,
    monitorUrl,
    type MonitorTarget,
} from '~/lib/monitor'

/** True inside the Tauri shell, where a real OS window can be created. */
function inTauri(): boolean {
    return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
}

/**
 * Open (or focus) the monitor window for a server.
 *
 * One label per server: asking for a server that already has a window brings
 * that window forward instead of stacking a duplicate, while a different
 * server gets its own. Outside the desktop shell (a browser dev build) it
 * falls back to a plain popup with the same route so the screen stays usable.
 */
export async function openMonitor(target: MonitorTarget): Promise<void> {
    const url = monitorUrl(target)

    if (!inTauri()) {
        window.open(
            `${window.location.pathname}#${url.split('#')[1]}`,
            monitorLabel(target),
            `popup,width=${MONITOR_SIZE.width},height=${MONITOR_SIZE.height}`
        )

        return
    }

    const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow')
    const label = monitorLabel(target)
    const existing = await WebviewWindow.getByLabel(label)

    if (existing) {
        try {
            await existing.unminimize()
            await existing.show()
            await existing.setFocus()

            return
        } catch {
            // A window that is mid-close: fall through and make a new one.
        }
    }

    const title = target.name ? `${target.name} — Monitor` : 'Server monitor'

    const win = new WebviewWindow(label, {
        url,
        title,
        width: MONITOR_SIZE.width,
        height: MONITOR_SIZE.height,
        minWidth: MONITOR_SIZE.minWidth,
        minHeight: MONITOR_SIZE.minHeight,
        resizable: true,
        center: true,
        focus: true,
    })

    await new Promise<void>((resolve, reject) => {
        void win.once('tauri://created', () => resolve())
        void win.once('tauri://error', (e) =>
            reject(new Error(String((e as { payload?: unknown }).payload)))
        )
    })
}
