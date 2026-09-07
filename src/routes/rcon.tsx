import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import {
    FiKey,
    FiPlus,
    FiSend,
    FiServer,
    FiTrash2,
    FiWifi,
    FiWifiOff,
} from 'react-icons/fi'

import { ipc } from '~/lib/ipc/commands'
import { isIpcError, messageOf } from '~/lib/ipc'
import type { RconHistoryT, RconServerT } from '~/lib/ipc/schemas'
import Select from '~/components/select'

/**
 * **RCON** — a console for a server you administer.
 *
 * WHAT THIS SCREEN PROMISES
 * -------------------------
 * That the password never leaves the machine. It is typed here, encrypted in
 * Rust and stored on this device; no command can read it back, and nothing
 * about it is sent to the website. The copy under the form says so, because a
 * user handing an app the keys to their server is entitled to know where they
 * are going.
 *
 * WHY THE HISTORY IS PERSISTED
 * ----------------------------
 * Because "what did I run on this box last week" is most of why an admin wants
 * a console at all. Output is kept alongside the command, bounded per server,
 * and it lives in the same local database as everything else — never in the
 * cloud, since a command's output routinely contains player names and IPs.
 */

const PROTOCOLS = [
    {
        value: 'source',
        label: 'Source RCON',
        hint: 'Minecraft, Rust, ARK, CS2, Squad, Palworld…',
    },
    {
        value: 'frostbite',
        label: 'Frostbite',
        hint: 'Battlefield 3, 4, Hardline, Bad Company 2',
    },
] as const

export default function RconRoute() {
    const { id } = useParams<{ id?: string }>()
    const navigate = useNavigate()

    const [servers, setServers] = useState<RconServerT[]>([])
    const [adding, setAdding] = useState(false)
    const [error, setError] = useState<string | null>(null)

    const selectedId = id ? Number(id) : null

    const load = useCallback(async () => {
        try {
            const listed = await ipc.rconList()

            setServers(listed)
            setError(null)
        } catch (err) {
            setError(messageOf(err))
        }
    }, [])

    useEffect(() => {
        void load()
    }, [load])

    const selected = servers.find((s) => s.id === selectedId) ?? null

    return (
        <div className="flex flex-col gap-4 p-4">
            <header className="flex flex-wrap items-center justify-between gap-3">
                <div>
                    <h1 className="text-lg font-bold">Server console</h1>
                    <p className="text-xs text-muted">
                        Run commands on servers you administer. Passwords are
                        encrypted on this device and never sent to The Modding
                        Community.
                    </p>
                </div>

                <button
                    type="button"
                    onClick={() => setAdding(true)}
                    className="flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-sm text-accent-foreground"
                >
                    <FiPlus className="size-3.5" />
                    Add a server
                </button>
            </header>

            {error && (
                <p className="selectable rounded-lg border border-danger/50 px-3 py-2 text-xs text-danger">
                    {error}
                </p>
            )}

            {adding && (
                <AddServer
                    onCancel={() => setAdding(false)}
                    onAdded={async (newId) => {
                        setAdding(false)
                        await load()
                        void navigate(`/rcon/${newId}`)
                    }}
                />
            )}

            {servers.length === 0 && !adding && (
                <div className="rounded-xl border border-dashed border-border p-8 text-center">
                    <FiServer aria-hidden className="mx-auto size-6 text-muted" />
                    <p className="mt-2 text-sm">No servers saved yet</p>
                    <p className="mx-auto mt-1 max-w-md text-xs text-muted">
                        Add one with its RCON address and password. A server on your
                        own network is fine — this is the one part of the app that
                        reaches a local address, because that is where most
                        people&rsquo;s servers are.
                    </p>
                </div>
            )}

            <div className="grid gap-3 lg:grid-cols-[minmax(0,18rem)_minmax(0,1fr)]">
                {servers.length > 0 && (
                    <nav className="flex flex-col gap-1.5">
                        {servers.map((server) => (
                            <button
                                key={server.id}
                                type="button"
                                onClick={() => navigate(`/rcon/${server.id}`)}
                                className={`rounded-xl border p-3 text-left transition-colors ${
                                    server.id === selectedId
                                        ? 'border-accent bg-surface'
                                        : 'border-border bg-surface hover:border-accent'
                                }`}
                            >
                                <p className="truncate text-sm font-medium">
                                    {server.name}
                                </p>
                                <p className="selectable truncate text-[0.7rem] text-muted">
                                    {server.host}:{server.port}
                                </p>
                                {!server.hasPassword && (
                                    <p className="mt-1 flex items-center gap-1 text-[0.7rem] text-warning">
                                        <FiKey className="size-3" />
                                        No password saved
                                    </p>
                                )}
                            </button>
                        ))}
                    </nav>
                )}

                {selected ? (
                    <Console
                        key={selected.id}
                        server={selected}
                        onChanged={load}
                        onDeleted={async () => {
                            await load()
                            void navigate('/rcon')
                        }}
                    />
                ) : (
                    servers.length > 0 && (
                        <div className="rounded-xl border border-dashed border-border p-8 text-center text-sm text-muted">
                            Choose a server to open its console.
                        </div>
                    )
                )}
            </div>
        </div>
    )
}

// ----------------------------------------------------------------- Console

type Line = {
    id: number
    command: string
    output: string
    ok: boolean
    at: string
    /** Still in flight. */
    pending?: boolean
}

function Console({
    server,
    onChanged,
    onDeleted,
}: {
    server: RconServerT
    onChanged: () => Promise<void>
    onDeleted: () => Promise<void>
}) {
    const [lines, setLines] = useState<Line[]>([])
    const [command, setCommand] = useState('')
    const [connected, setConnected] = useState(false)
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const [settingPassword, setSettingPassword] = useState(false)

    /*
     * Command history, as a shell has it. Up and down walk it; the draft is
     * kept so arrowing back down returns what was half-typed rather than
     * clearing the field.
     */
    const history = useRef<string[]>([])
    const cursor = useRef(-1)
    const draft = useRef('')

    const bottom = useRef<HTMLDivElement>(null)
    const nextId = useRef(1)

    useEffect(() => {
        void (async () => {
            const [stored, live] = await Promise.all([
                ipc.rconHistory(server.id, 100).catch(() => [] as RconHistoryT[]),
                ipc.rconIsConnected(server.id).catch(() => false),
            ])

            setConnected(live)

            // Oldest first: a console reads downward.
            setLines(
                [...stored].reverse().map((row) => ({
                    id: nextId.current++,
                    command: row.command,
                    output: row.output,
                    ok: row.ok,
                    at: row.at,
                }))
            )

            history.current = [...stored]
                .reverse()
                .map((row) => row.command)
                .filter(Boolean)
        })()
    }, [server.id])

    // Scroll to the newest line. `useLayoutEffect` so it happens before paint —
    // with `useEffect` the console visibly jumps after each command.
    useLayoutEffect(() => {
        bottom.current?.scrollIntoView({ block: 'end' })
    }, [lines])

    const connect = async () => {
        setBusy(true)
        setError(null)

        try {
            await ipc.rconConnect(server.id)
            setConnected(true)
        } catch (err) {
            setError(messageOf(err))
            setConnected(false)
        } finally {
            setBusy(false)
        }
    }

    const send = async () => {
        const text = command.trim()

        if (!text || busy) return

        const id = nextId.current++

        setLines((was) => [
            ...was,
            {
                id,
                command: text,
                output: '',
                ok: true,
                at: new Date().toISOString(),
                pending: true,
            },
        ])

        history.current.push(text)
        cursor.current = -1
        draft.current = ''

        setCommand('')
        setBusy(true)
        setError(null)

        try {
            const reply = await ipc.rconExec(server.id, text)

            setConnected(true)
            setLines((was) =>
                was.map((line) =>
                    line.id === id
                        ? { ...line, output: reply.output, pending: false }
                        : line
                )
            )
        } catch (err) {
            const message = messageOf(err)

            setLines((was) =>
                was.map((line) =>
                    line.id === id
                        ? { ...line, output: message, ok: false, pending: false }
                        : line
                )
            )

            // An auth failure means the session is gone; anything else may
            // just be a command the server did not like.
            if (message.toLowerCase().includes('sign in') || isAuthError(err))
                setConnected(false)
        } finally {
            setBusy(false)
            await onChanged()
        }
    }

    const walkHistory = (delta: number) => {
        if (history.current.length === 0) return

        if (cursor.current === -1) draft.current = command

        const next = cursor.current + delta

        if (next < 0) {
            cursor.current = -1
            setCommand(draft.current)

            return
        }

        if (next >= history.current.length) return

        cursor.current = next
        setCommand(history.current[history.current.length - 1 - next] ?? '')
    }

    return (
        <div className="flex min-w-0 flex-col gap-3">
            <header className="flex flex-wrap items-center gap-2 rounded-xl border border-border bg-surface p-3">
                <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium">{server.name}</p>
                    <p className="selectable truncate text-[0.7rem] text-muted">
                        {server.host}:{server.port} ·{' '}
                        {server.protocol === 'frostbite'
                            ? 'Frostbite'
                            : 'Source RCON'}
                    </p>
                </div>

                <span
                    className={`flex items-center gap-1 text-[0.7rem] ${
                        connected ? 'text-success' : 'text-muted'
                    }`}
                >
                    {connected ? (
                        <FiWifi className="size-3" />
                    ) : (
                        <FiWifiOff className="size-3" />
                    )}
                    {connected ? 'Connected' : 'Not connected'}
                </span>

                {!connected && (
                    <button
                        type="button"
                        disabled={busy || !server.hasPassword}
                        onClick={() => void connect()}
                        className="rounded-lg border border-border px-2 py-1 text-xs hover:border-accent disabled:opacity-50"
                    >
                        {busy ? 'Connecting…' : 'Connect'}
                    </button>
                )}

                <button
                    type="button"
                    onClick={() => setSettingPassword(true)}
                    className="flex items-center gap-1 rounded-lg border border-border px-2 py-1 text-xs hover:border-accent"
                >
                    <FiKey className="size-3" />
                    {server.hasPassword ? 'Change password' : 'Set password'}
                </button>

                <button
                    type="button"
                    aria-label="Delete this server"
                    onClick={() => void ipc.rconDelete(server.id).then(onDeleted)}
                    className="rounded-lg border border-border p-1.5 text-muted hover:border-danger hover:text-danger"
                >
                    <FiTrash2 className="size-3" />
                </button>
            </header>

            {settingPassword && (
                <PasswordForm
                    server={server}
                    onDone={async () => {
                        setSettingPassword(false)
                        await onChanged()
                    }}
                    onCancel={() => setSettingPassword(false)}
                />
            )}

            {!server.hasPassword && (
                <p className="rounded-lg border border-warning/50 px-3 py-2 text-xs text-warning">
                    Set the RCON password before connecting. It is encrypted on this
                    device and never leaves it.
                </p>
            )}

            {error && (
                <p className="selectable rounded-lg border border-danger/50 px-3 py-2 text-xs text-danger">
                    {error}
                </p>
            )}

            <div className="flex h-[26rem] flex-col overflow-hidden rounded-xl border border-border bg-surface-2">
                <div className="flex-1 overflow-y-auto p-3 font-mono text-xs">
                    {lines.length === 0 && (
                        <p className="text-muted">
                            Nothing yet. Try <code>help</code>.
                        </p>
                    )}

                    {lines.map((line) => (
                        <div key={line.id} className="mb-2">
                            <p className="selectable text-accent">
                                <span className="text-muted">&gt;</span>{' '}
                                {line.command}
                            </p>

                            {line.pending ? (
                                <p className="text-muted">…</p>
                            ) : (
                                line.output && (
                                    <pre
                                        className={`selectable whitespace-pre-wrap break-words ${
                                            line.ok ? '' : 'text-danger'
                                        }`}
                                    >
                                        {line.output}
                                    </pre>
                                )
                            )}
                        </div>
                    ))}

                    <div ref={bottom} />
                </div>

                <form
                    className="flex items-center gap-2 border-t border-border p-2"
                    onSubmit={(e) => {
                        e.preventDefault()
                        void send()
                    }}
                >
                    <span className="pl-1 font-mono text-xs text-muted">&gt;</span>
                    <input
                        value={command}
                        disabled={!server.hasPassword}
                        placeholder="status"
                        spellCheck={false}
                        autoComplete="off"
                        onChange={(e) => setCommand(e.target.value)}
                        onKeyDown={(e) => {
                            if (e.key === 'ArrowUp') {
                                e.preventDefault()
                                walkHistory(1)
                            }
                            if (e.key === 'ArrowDown') {
                                e.preventDefault()
                                walkHistory(-1)
                            }
                        }}
                        className="min-w-0 flex-1 bg-transparent font-mono text-xs outline-none"
                    />
                    <button
                        type="submit"
                        aria-label="Run"
                        disabled={busy || !command.trim()}
                        className="rounded-lg border border-border p-1.5 text-muted hover:border-accent hover:text-foreground disabled:opacity-40"
                    >
                        <FiSend className="size-3.5" />
                    </button>
                </form>
            </div>

            <div className="flex items-center justify-between text-[0.7rem] text-muted">
                <span>Up and down arrows walk your command history.</span>
                <button
                    type="button"
                    onClick={() => {
                        void ipc.rconClearHistory(server.id)
                        setLines([])
                    }}
                    className="hover:text-foreground"
                >
                    Clear the log
                </button>
            </div>
        </div>
    )
}

// -------------------------------------------------------------------- Forms

function PasswordForm({
    server,
    onDone,
    onCancel,
}: {
    server: RconServerT
    onDone: () => Promise<void>
    onCancel: () => void
}) {
    const [password, setPassword] = useState('')
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<string | null>(null)

    const save = async (value: string | null) => {
        setBusy(true)
        setError(null)

        try {
            await ipc.rconSetPassword(server.id, value)
            await onDone()
        } catch (err) {
            setError(messageOf(err))
        } finally {
            setBusy(false)
        }
    }

    return (
        <form
            className="flex flex-col gap-2 rounded-xl border border-border bg-surface p-3"
            onSubmit={(e) => {
                e.preventDefault()
                void save(password)
            }}
        >
            <label className="text-xs">
                RCON password
                <input
                    type="password"
                    value={password}
                    autoFocus
                    autoComplete="off"
                    onChange={(e) => setPassword(e.target.value)}
                    className="mt-1 w-full rounded-lg border border-border bg-background px-2 py-1.5 text-sm"
                />
            </label>

            <p className="text-[0.7rem] text-muted">
                Encrypted with a key held in your operating system&rsquo;s
                credential store. It is never sent to The Modding Community and no
                part of the app can read it back.
            </p>

            {error && <p className="text-xs text-danger">{error}</p>}

            <div className="flex gap-2">
                <button
                    type="submit"
                    disabled={busy || !password}
                    className="rounded-lg bg-accent px-3 py-1.5 text-sm text-accent-foreground disabled:opacity-50"
                >
                    Save
                </button>

                {server.hasPassword && (
                    <button
                        type="button"
                        disabled={busy}
                        onClick={() => void save(null)}
                        className="rounded-lg border border-border px-3 py-1.5 text-sm"
                    >
                        Remove it
                    </button>
                )}

                <button
                    type="button"
                    onClick={onCancel}
                    className="rounded-lg border border-border px-3 py-1.5 text-sm"
                >
                    Cancel
                </button>
            </div>
        </form>
    )
}

function AddServer({
    onCancel,
    onAdded,
}: {
    onCancel: () => void
    onAdded: (id: number) => Promise<void>
}) {
    const [name, setName] = useState('')
    const [host, setHost] = useState('')
    const [port, setPort] = useState('27015')
    const [protocol, setProtocol] = useState<'source' | 'frostbite'>('source')
    const [password, setPassword] = useState('')
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<string | null>(null)

    const submit = async () => {
        setBusy(true)
        setError(null)

        try {
            const id = await ipc.rconCreate({
                name: name.trim(),
                host: host.trim(),
                port: Number(port),
                protocol,
                password,
            })

            await onAdded(id)
        } catch (err) {
            setError(messageOf(err))
        } finally {
            setBusy(false)
        }
    }

    return (
        <form
            className="flex flex-col gap-3 rounded-xl border border-border bg-surface p-4"
            onSubmit={(e) => {
                e.preventDefault()
                void submit()
            }}
        >
            <h2 className="text-sm font-semibold">Add a server</h2>

            {error && <p className="text-xs text-danger">{error}</p>}

            <div className="grid gap-3 sm:grid-cols-2">
                <label className="text-xs">
                    Name
                    <input
                        value={name}
                        maxLength={96}
                        placeholder="My survival server"
                        onChange={(e) => setName(e.target.value)}
                        className="mt-1 w-full rounded-lg border border-border bg-background px-2 py-1.5 text-sm"
                    />
                </label>

                <div className="text-xs">
                    <span className="mb-1 block">Protocol</span>
                    <Select
                        label="RCON protocol"
                        fullWidth
                        value={protocol}
                        onChange={(next) => setProtocol(next)}
                        options={PROTOCOLS.map((p) => ({
                            value: p.value,
                            label: p.label,
                            hint: p.hint,
                        }))}
                    />
                </div>

                <label className="text-xs">
                    Address
                    <input
                        value={host}
                        placeholder="192.168.1.10"
                        autoComplete="off"
                        onChange={(e) => setHost(e.target.value)}
                        className="mt-1 w-full rounded-lg border border-border bg-background px-2 py-1.5 text-sm"
                    />
                </label>

                <label className="text-xs">
                    RCON port
                    <input
                        value={port}
                        type="number"
                        min={1}
                        max={65535}
                        onChange={(e) => setPort(e.target.value)}
                        className="mt-1 w-full rounded-lg border border-border bg-background px-2 py-1.5 text-sm"
                    />
                </label>
            </div>

            <label className="text-xs">
                Password
                <input
                    type="password"
                    value={password}
                    autoComplete="off"
                    onChange={(e) => setPassword(e.target.value)}
                    className="mt-1 w-full rounded-lg border border-border bg-background px-2 py-1.5 text-sm"
                />
                <span className="mt-1 block text-[0.7rem] text-muted">
                    Stored encrypted on this device only. Leave it blank to add the
                    server now and set one later.
                </span>
            </label>

            <div className="flex gap-2">
                <button
                    type="submit"
                    disabled={busy || !name.trim() || !host.trim()}
                    className="rounded-lg bg-accent px-3 py-1.5 text-sm text-accent-foreground disabled:opacity-50"
                >
                    {busy ? 'Saving…' : 'Save'}
                </button>
                <button
                    type="button"
                    onClick={onCancel}
                    className="rounded-lg border border-border px-3 py-1.5 text-sm"
                >
                    Cancel
                </button>
            </div>
        </form>
    )
}

function isAuthError(err: unknown): boolean {
    return isIpcError(err) && err.code === 'auth_rejected'
}
