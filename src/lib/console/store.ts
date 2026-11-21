import { ipc } from '~/lib/ipc/commands'
import { messageOf, subscribe } from '~/lib/ipc'
import { ConsoleEventSchema, type ConsoleEventT } from '~/lib/ipc/schemas'

import {
    appendLine,
    appendOutput,
    emptyTranscript,
    type Transcript,
} from './transcript'

/**
 * The Console's state, held above the screen.
 *
 * Module-level rather than component state because a run outlives the route:
 * start `file upload`, open Downloads, come back, and the transcript and the
 * Stop button must still be there. The app is one window, so one store is the
 * right number — the same reasoning that puts the download queue above the
 * router.
 *
 * Notifications are coalesced to one per animation frame: a command that prints
 * a table arrives as many chunks, and a render per chunk is the slowest thing
 * on the screen.
 */

export type Running = { runId: number; display: string; startedAt: number }

export type Finished = {
    code: number | null
    stopped: boolean
    millis: number
}

class ConsoleStore {
    transcript: Transcript = emptyTranscript()
    running: Running | null = null
    finished: Finished | null = null
    version = 0

    /** Events that arrive before `consoleRun` resolves, so the command line is printed first. */
    private starting = false
    private queued: ConsoleEventT[] = []
    private exited = new Set<number>()
    private listeners = new Set<() => void>()
    private frame = 0
    private attached: Promise<void> | null = null

    subscribe = (fn: () => void) => {
        this.listeners.add(fn)
        void this.attach()

        return () => {
            this.listeners.delete(fn)
        }
    }

    getVersion = () => this.version

    /** Listen for Rust's events, once for the life of the window. */
    private attach(): Promise<void> {
        this.attached ??= subscribe('tmc://console', ConsoleEventSchema, (e) =>
            this.handle(e)
        )
            .then(() => undefined)
            .catch(() => {
                // No Tauri bridge (a plain browser): the screen shows its own state.
                this.attached = null
            })

        return this.attached
    }

    private bump() {
        this.version++

        if (this.frame) return

        const fire = () => {
            this.frame = 0
            this.listeners.forEach((fn) => fn())
        }

        if (typeof requestAnimationFrame === 'function') {
            this.frame = requestAnimationFrame(fire)
        } else {
            this.frame = 1
            queueMicrotask(fire)
        }
    }

    private handle(e: ConsoleEventT) {
        if (this.starting) {
            this.queued.push(e)

            return
        }

        this.apply(e)
    }

    private apply(e: ConsoleEventT) {
        if (e.type === 'output') {
            appendOutput(this.transcript, e.stream, e.text)
        } else {
            this.exited.add(e.runId)

            if (this.running?.runId === e.runId) this.running = null

            this.finished = { code: e.code, stopped: e.stopped, millis: e.millis }
        }

        this.bump()
    }

    async run(line: string): Promise<void> {
        if (this.running || this.starting) return

        this.starting = true
        this.queued = []
        this.finished = null

        try {
            const started = await ipc.consoleRun(line)

            appendLine(this.transcript, 'cmd', started.display)

            if (!this.exited.has(started.runId)) {
                this.running = {
                    runId: started.runId,
                    display: started.display,
                    startedAt: Date.now(),
                }
            }
        } catch (err) {
            appendLine(this.transcript, 'note', messageOf(err))
        } finally {
            this.starting = false

            const queued = this.queued

            this.queued = []

            // Applied after the command line is in. An exit that came first has
            // already been remembered, so `running` is not set for a run that ended.
            queued.forEach((e) => this.apply(e))
            this.bump()
        }
    }

    /** Send text to the running process. `echo` false for a secret. */
    async sendInput(text: string, echo: boolean): Promise<void> {
        if (!this.running) return

        try {
            await ipc.consoleStdin(text, true)

            appendLine(this.transcript, 'input', echo ? text : '(input hidden)')
        } catch (err) {
            appendLine(this.transcript, 'note', messageOf(err))
        }

        this.bump()
    }

    async closeInput(): Promise<void> {
        if (!this.running) return

        try {
            await ipc.consoleStdinClose()
            appendLine(this.transcript, 'note', 'end of input sent')
        } catch (err) {
            appendLine(this.transcript, 'note', messageOf(err))
        }

        this.bump()
    }

    async stop(): Promise<void> {
        if (!this.running) return

        try {
            await ipc.consoleStop()
        } catch (err) {
            appendLine(this.transcript, 'note', messageOf(err))
            this.bump()
        }
    }

    clear() {
        this.transcript = emptyTranscript()
        this.finished = null
        this.bump()
    }

    /** Pick up a run that started before this window's state existed (a reload). */
    async reattach(): Promise<void> {
        try {
            const id = await ipc.consoleRunning()

            if (id !== null && !this.running) {
                this.running = {
                    runId: id,
                    display: '(running)',
                    startedAt: Date.now(),
                }
                this.bump()
            }
        } catch {
            // Not on a desktop build: nothing to re-attach.
        }
    }
}

export const consoleStore = new ConsoleStore()
