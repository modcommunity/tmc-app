import { useMemo } from 'react'
import { openUrl } from '@tauri-apps/plugin-opener'

/**
 * Renders a content item's body.
 *
 * **This is untrusted text.** It is written by whoever published the mod, and
 * it renders inside a webview that can call `invoke`. A markdown library that
 * emits HTML — every popular one does, and most allow raw HTML by default —
 * would put `<img onerror>` one careless setting away from the IPC surface.
 *
 * So there is no HTML path at all. A small block/inline tokeniser produces
 * React elements directly; anything it does not recognise renders as text. The
 * result supports the subset that mod descriptions actually use, and cannot
 * emit a node type this file does not construct itself.
 *
 * (The other half of the defence is in Rust: no command returns a credential,
 * so even a successful injection has nothing to exfiltrate. Both layers, not
 * either one.)
 */

type Block =
    | { kind: 'heading'; level: 2 | 3 | 4; text: string }
    | { kind: 'paragraph'; text: string }
    | { kind: 'code'; text: string }
    | { kind: 'quote'; text: string }
    | { kind: 'list'; ordered: boolean; items: string[] }
    | { kind: 'rule' }

/** Bound on what we will lay out. A 500 KB description is a denial of service. */
const MAX_SOURCE = 200_000

function tokenise(source: string): Block[] {
    const lines = source.slice(0, MAX_SOURCE).replace(/\r\n?/g, '\n').split('\n')
    const blocks: Block[] = []

    let i = 0

    while (i < lines.length) {
        const line = lines[i] ?? ''

        if (!line.trim()) {
            i += 1
            continue
        }

        // Fenced code. Everything inside is literal, including markup.
        if (line.startsWith('```')) {
            const body: string[] = []
            i += 1

            while (i < lines.length && !(lines[i] ?? '').startsWith('```')) {
                body.push(lines[i] ?? '')
                i += 1
            }

            i += 1
            blocks.push({ kind: 'code', text: body.join('\n') })
            continue
        }

        if (/^\s*([-*_])\1{2,}\s*$/.test(line)) {
            blocks.push({ kind: 'rule' })
            i += 1
            continue
        }

        const heading = /^(#{1,6})\s+(.*)$/.exec(line)

        if (heading) {
            // Clamped to h2–h4: the page already owns h1, and a document
            // outline is not something a mod description gets to define.
            const level = Math.min(4, Math.max(2, heading[1]!.length + 1)) as 2 | 3 | 4

            blocks.push({ kind: 'heading', level, text: heading[2]!.trim() })
            i += 1
            continue
        }

        if (line.startsWith('>')) {
            const body: string[] = []

            while (i < lines.length && (lines[i] ?? '').startsWith('>')) {
                body.push((lines[i] ?? '').replace(/^>\s?/, ''))
                i += 1
            }

            blocks.push({ kind: 'quote', text: body.join(' ') })
            continue
        }

        const bullet = /^\s*[-*+]\s+/
        const numbered = /^\s*\d+[.)]\s+/

        if (bullet.test(line) || numbered.test(line)) {
            const ordered = numbered.test(line)
            const pattern = ordered ? numbered : bullet
            const items: string[] = []

            while (i < lines.length && pattern.test(lines[i] ?? '')) {
                items.push((lines[i] ?? '').replace(pattern, ''))
                i += 1
            }

            blocks.push({ kind: 'list', ordered, items })
            continue
        }

        const body: string[] = []

        while (
            i < lines.length &&
            (lines[i] ?? '').trim() &&
            !(lines[i] ?? '').startsWith('```') &&
            !/^#{1,6}\s/.test(lines[i] ?? '') &&
            !(lines[i] ?? '').startsWith('>') &&
            !bullet.test(lines[i] ?? '') &&
            !numbered.test(lines[i] ?? '')
        ) {
            body.push(lines[i] ?? '')
            i += 1
        }

        blocks.push({ kind: 'paragraph', text: body.join(' ') })
    }

    return blocks
}

/** `https:` and `http:` only — no `javascript:`, no `data:`, no `file:`. */
function safeHref(raw: string): string | null {
    try {
        const url = new URL(raw)

        return url.protocol === 'https:' || url.protocol === 'http:'
            ? url.toString()
            : null
    } catch {
        return null
    }
}

/**
 * Inline markup → React nodes.
 *
 * The regex alternation is ordered so the longest delimiters win (`**` before
 * `*`), and every branch produces a specific element. There is no fallthrough
 * that emits markup.
 */
function inline(text: string, keyBase: string): React.ReactNode[] {
    const out: React.ReactNode[] = []
    const pattern = /(\*\*[^*]+\*\*|\*[^*]+\*|`[^`]+`|\[[^\]]+\]\([^)\s]+\))/g

    let last = 0
    let match: RegExpExecArray | null
    let n = 0

    while ((match = pattern.exec(text)) !== null) {
        if (match.index > last) out.push(text.slice(last, match.index))

        const token = match[0]
        const key = `${keyBase}-${n++}`

        if (token.startsWith('**')) {
            out.push(<strong key={key}>{token.slice(2, -2)}</strong>)
        } else if (token.startsWith('`')) {
            out.push(
                <code
                    key={key}
                    className="rounded bg-surface-secondary px-1 py-0.5 text-[0.85em]"
                >
                    {token.slice(1, -1)}
                </code>
            )
        } else if (token.startsWith('[')) {
            const link = /^\[([^\]]+)\]\(([^)\s]+)\)$/.exec(token)
            const href = link ? safeHref(link[2]!) : null

            if (href) {
                out.push(
                    <button
                        key={key}
                        type="button"
                        // Opens in the SYSTEM browser. An in-app navigation to
                        // a third-party page would put untrusted script on the
                        // same origin as the IPC bridge.
                        onClick={() => void openUrl(href)}
                        className="text-accent underline underline-offset-2"
                    >
                        {link![1]}
                    </button>
                )
            } else {
                // A refused scheme renders as its own text, so the reader can
                // see what was there rather than the link silently vanishing.
                out.push(token)
            }
        } else {
            out.push(<em key={key}>{token.slice(1, -1)}</em>)
        }

        last = match.index + token.length
    }

    if (last < text.length) out.push(text.slice(last))

    return out
}

const HEADING_TAGS = { 2: 'h2', 3: 'h3', 4: 'h4' } as const

export default function Markdown({ source }: { source: string }) {
    const blocks = useMemo(() => tokenise(source), [source])

    return (
        <div className="flex flex-col gap-3 text-sm leading-relaxed">
            {blocks.map((block, index) => {
                const key = `b-${index}`

                switch (block.kind) {
                    case 'heading': {
                        // The union, not `string`: JSX needs to know this is an
                        // intrinsic element to accept children.
                        const Tag = HEADING_TAGS[block.level]

                        return (
                            <Tag
                                key={key}
                                className={
                                    block.level === 2
                                        ? 'mt-2 text-lg font-bold'
                                        : 'mt-1 text-base font-semibold'
                                }
                            >
                                {inline(block.text, key)}
                            </Tag>
                        )
                    }

                    case 'code':
                        return (
                            <pre
                                key={key}
                                className="overflow-x-auto rounded-lg bg-surface-secondary p-3 text-xs"
                            >
                                <code>{block.text}</code>
                            </pre>
                        )

                    case 'quote':
                        return (
                            <blockquote
                                key={key}
                                className="border-l-2 border-accent pl-3 text-muted"
                            >
                                {inline(block.text, key)}
                            </blockquote>
                        )

                    case 'list': {
                        const Tag = block.ordered ? 'ol' : 'ul'

                        return (
                            <Tag
                                key={key}
                                className={`ml-5 flex flex-col gap-1 ${
                                    block.ordered ? 'list-decimal' : 'list-disc'
                                }`}
                            >
                                {block.items.map((item, j) => (
                                    <li key={`${key}-${j}`}>
                                        {inline(item, `${key}-${j}`)}
                                    </li>
                                ))}
                            </Tag>
                        )
                    }

                    case 'rule':
                        return <hr key={key} className="border-border" />

                    default:
                        return <p key={key}>{inline(block.text, key)}</p>
                }
            })}
        </div>
    )
}
