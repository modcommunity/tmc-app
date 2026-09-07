import { useMemo, useState, type ReactNode } from 'react'
import { FiChevronDown, FiChevronRight } from 'react-icons/fi'

import {
    BrowseEnvironmentVals,
    ServerOsVals,
    type BrowseQueryT,
    type ContentKindT,
    type FacetsResponseT,
} from '~/lib/api/contract'
import { appLabel } from '~/lib/api/labels'
import Select from '~/components/select'

/**
 * The browse filter panel.
 *
 * Every filter here exists on the website's own browsers — this mirrors
 * `ServerBrowserPublicGetsInput` and the mod/asset equivalents, so a filter
 * someone used on the site is reachable in the app under the same name and with
 * the same meaning. Where the two could disagree, the website wins: `hideFull`
 * drops servers declaring no limit because the site's SQL does, and `tagsOr`
 * defaults off because the site's chips read as "all of these".
 *
 * WHY GROUPS, COLLAPSED
 * ---------------------
 * Servers went from three controls to eighteen. One flat column is fine on a
 * 1400px sidebar and unusable in a phone sheet, where it becomes a scroll with
 * no landmarks. Grouping gives the eye somewhere to aim, and collapsing keeps
 * the panel one screen tall — with the count badge doing the work the open
 * section used to: telling you a filter is on without making you find it.
 *
 * A group holding an ACTIVE filter starts open, so nothing is ever both applied
 * and hidden.
 */

/** A filter's value as it lives in the URL, before the contract coerces it. */
export type FilterState = Partial<
    Record<keyof BrowseQueryT, string | string[] | null>
>

type GroupProps = {
    title: string
    /** How many filters in this group are set. Drives the badge and the default. */
    active: number
    children: ReactNode
}

function Group({ title, active, children }: GroupProps) {
    // Open when it holds something, and thereafter whatever the user last did.
    const [open, setOpen] = useState(active > 0)

    return (
        <div className="border-b border-border last:border-0">
            <button
                type="button"
                onClick={() => setOpen((o) => !o)}
                aria-expanded={open}
                className="flex w-full items-center gap-2 py-2.5 text-left text-xs font-medium uppercase tracking-wide text-muted transition-colors hover:text-foreground"
            >
                {open ? (
                    <FiChevronDown className="size-3.5 shrink-0" />
                ) : (
                    <FiChevronRight className="size-3.5 shrink-0" />
                )}

                <span className="flex-1">{title}</span>

                {active > 0 && (
                    <span className="rounded-full bg-accent px-1.5 text-[0.65rem] font-semibold text-accent-foreground">
                        {active}
                    </span>
                )}
            </button>

            {open && <div className="flex flex-col gap-3 pb-3">{children}</div>}
        </div>
    )
}

function Field({ label, children }: { label: string; children: ReactNode }) {
    return (
        <label className="flex flex-col gap-1">
            <span className="text-[0.7rem] text-muted">{label}</span>
            {children}
        </label>
    )
}

function Check({
    label,
    hint,
    checked,
    onChange,
}: {
    label: string
    hint?: string
    checked: boolean
    onChange: (next: boolean) => void
}) {
    return (
        <label className="flex flex-col gap-0.5">
            <span className="flex items-center gap-2 text-sm">
                <input
                    type="checkbox"
                    checked={checked}
                    onChange={(e) => onChange(e.target.checked)}
                />
                {label}
            </span>
            {hint && <span className="pl-6 text-[0.7rem] text-muted">{hint}</span>}
        </label>
    )
}

const inputClass =
    'w-full rounded-lg border border-border bg-surface px-2 py-1.5 text-sm'

/** A `min`–`max` pair, which is two fields but one idea. */
function Range({
    label,
    hint,
    min,
    max,
    onMin,
    onMax,
}: {
    label: string
    hint?: string
    min: string
    max: string
    onMin: (v: string) => void
    onMax: (v: string) => void
}) {
    return (
        <div className="flex flex-col gap-1">
            <span className="text-[0.7rem] text-muted">{label}</span>
            <div className="flex items-center gap-1.5">
                <input
                    type="number"
                    min={0}
                    inputMode="numeric"
                    value={min}
                    onChange={(e) => onMin(e.target.value)}
                    placeholder="min"
                    aria-label={`${label} minimum`}
                    className={inputClass}
                />
                <span className="text-muted">–</span>
                <input
                    type="number"
                    min={0}
                    inputMode="numeric"
                    value={max}
                    onChange={(e) => onMax(e.target.value)}
                    placeholder="max"
                    aria-label={`${label} maximum`}
                    className={inputClass}
                />
            </div>
            {hint && <span className="text-[0.7rem] text-muted">{hint}</span>}
        </div>
    )
}

const OS_LABELS: Record<(typeof ServerOsVals)[number], string> = {
    WINDOWS: 'Windows',
    LINUX: 'Linux',
    MAC: 'macOS',
}

const ENV_LABELS: Record<(typeof BrowseEnvironmentVals)[number], string> = {
    ALL: 'Any',
    SERVER: 'Server-side',
    CLIENT: 'Client-side',
}

export default function BrowseFilters({
    kind,
    get,
    set,
    facets,
    signedIn,
    onClear,
    activeCount,
}: {
    kind: ContentKindT
    /** Read one filter's raw URL value. */
    get: (key: string) => string | null
    /** Write one filter, or clear it with `null`. */
    set: (key: string, value: string | null) => void
    facets: FacetsResponseT | undefined
    signedIn: boolean
    onClear: () => void
    activeCount: number
}) {
    const isServer = kind === 'server'
    const hasEnvironment = kind === 'mod' || kind === 'asset'
    const hasApp = kind !== 'community' && kind !== 'collection' && kind !== 'user'

    const bool = (key: string) => get(key) === '1'
    const setBool = (key: string, on: boolean) => set(key, on ? '1' : null)

    /** Multi-select ids live in the URL as a comma list. */
    const ids = (key: string): number[] =>
        (get(key) ?? '')
            .split(',')
            .map((v) => Number(v))
            .filter((v) => Number.isFinite(v) && v > 0)

    const toggleId = (key: string, id: number) => {
        const current = ids(key)
        const next = current.includes(id)
            ? current.filter((v) => v !== id)
            : [...current, id]

        set(key, next.length > 0 ? next.join(',') : null)
    }

    // Counts per group, so a collapsed group still says it is doing something.
    const counts = useMemo(() => {
        const n = (...keys: string[]) =>
            keys.filter((k) => {
                const v = get(k)

                return v !== null && v !== ''
            }).length

        return {
            game: n('app', 'countries', 'official'),
            players: n(
                'empty',
                'full',
                'minUsers',
                'maxUsers',
                'minSlots',
                'maxSlots'
            ),
            type:
                n('password', 'secure', 'os', 'map', 'wasOnline') +
                // Only counts when turned OFF — being on is the default, and a
                // badge on every fresh browse would mean nothing.
                (get('online') === '0' ? 1 : 0),
            content: n('cats', 'tags', 'tagsOr', 'nsfw', 'archived', 'env', 'mine'),
        }
    }, [get])

    return (
        <div className="flex flex-col text-sm">
            {activeCount > 0 && (
                <button
                    type="button"
                    onClick={onClear}
                    className="mb-1 self-start text-xs text-accent hover:underline"
                >
                    Clear {activeCount} filter{activeCount === 1 ? '' : 's'}
                </button>
            )}

            {/* ------------------------------------------------ Game & region */}
            {(hasApp || isServer) && (
                <Group
                    title={isServer ? 'Game & region' : 'Game'}
                    active={counts.game}
                >
                    {hasApp && (facets?.apps.length ?? 0) > 0 && (
                        <Field label="Game">
                            <Select
                                label="Game"
                                fullWidth
                                value={get('app') ?? ''}
                                onChange={(next) => set('app', next || null)}
                                options={[
                                    { value: '', label: 'All games' },
                                    ...(facets?.apps ?? []).map((game) => ({
                                        value: String(game.id),
                                        label: appLabel(game),
                                        hint: `${game.count.toLocaleString()} items`,
                                        icon: game.icon ? (
                                            <img
                                                src={game.icon}
                                                alt=""
                                                loading="lazy"
                                                className="size-4 rounded-sm object-cover"
                                            />
                                        ) : undefined,
                                    })),
                                ]}
                            />
                        </Field>
                    )}

                    {isServer && (facets?.countries.length ?? 0) > 0 && (
                        <div className="flex flex-col gap-1">
                            <span className="text-[0.7rem] text-muted">
                                Country
                            </span>
                            <div className="flex max-h-40 flex-col gap-1 overflow-y-auto rounded-lg border border-border p-2">
                                {facets?.countries.map((country) => (
                                    <label
                                        key={country.id}
                                        className="flex items-center gap-2 text-xs"
                                    >
                                        <input
                                            type="checkbox"
                                            checked={ids('countries').includes(
                                                country.id
                                            )}
                                            onChange={() =>
                                                toggleId('countries', country.id)
                                            }
                                        />
                                        <span className="truncate">
                                            {country.name}
                                        </span>
                                        <span className="ml-auto shrink-0 text-muted">
                                            {country.count}
                                        </span>
                                    </label>
                                ))}
                            </div>
                        </div>
                    )}

                    {isServer && (
                        <Check
                            label="Official only"
                            checked={bool('official')}
                            onChange={(v) => setBool('official', v)}
                        />
                    )}
                </Group>
            )}

            {/* ------------------------------------------------------ Players */}
            {isServer && (
                <Group title="Players" active={counts.players}>
                    <Check
                        label="Hide empty"
                        checked={bool('empty')}
                        onChange={(v) => setBool('empty', v)}
                    />
                    <Check
                        label="Hide full"
                        hint="Matches the website, which also hides servers that declare no player limit."
                        checked={bool('full')}
                        onChange={(v) => setBool('full', v)}
                    />

                    <Range
                        label="Players online"
                        min={get('minUsers') ?? ''}
                        max={get('maxUsers') ?? ''}
                        onMin={(v) => set('minUsers', v || null)}
                        onMax={(v) => set('maxUsers', v || null)}
                    />

                    <Range
                        label="Slots"
                        hint="The server's capacity, not how many are on it."
                        min={get('minSlots') ?? ''}
                        max={get('maxSlots') ?? ''}
                        onMin={(v) => set('minSlots', v || null)}
                        onMax={(v) => set('maxSlots', v || null)}
                    />
                </Group>
            )}

            {/* -------------------------------------------------- Server type */}
            {isServer && (
                <Group title="Server" active={counts.type}>
                    {/* Default ON, matching the website. `online=0` is the
                        off state, so the URL distinguishes "not chosen" from
                        "deliberately showing offline servers". */}
                    <Check
                        label="Online only"
                        checked={get('online') !== '0'}
                        onChange={(v) => set('online', v ? null : '0')}
                    />
                    <Check
                        label="Seen recently"
                        hint="Online at some point inside the inactivity window."
                        checked={bool('wasOnline')}
                        onChange={(v) => setBool('wasOnline', v)}
                    />
                    <Check
                        label="Password protected"
                        checked={bool('password')}
                        onChange={(v) => setBool('password', v)}
                    />
                    <Check
                        label="Anti-cheat enabled"
                        checked={bool('secure')}
                        onChange={(v) => setBool('secure', v)}
                    />

                    <Field label="Operating system">
                        <Select
                            label="Operating system"
                            fullWidth
                            value={get('os') ?? ''}
                            onChange={(next) => set('os', next || null)}
                            options={[
                                { value: '', label: 'Any' },
                                ...ServerOsVals.map((os) => ({
                                    value: os,
                                    label: OS_LABELS[os],
                                })),
                            ]}
                        />
                    </Field>

                    <Field label="Map">
                        <input
                            value={get('map') ?? ''}
                            onChange={(e) => set('map', e.target.value || null)}
                            placeholder="de_dust2"
                            className={inputClass}
                        />
                    </Field>
                </Group>
            )}

            {/* ------------------------------------------------------ Content */}
            <Group title="Content" active={counts.content}>
                {hasEnvironment && (
                    <Field label="Runs on">
                        <Select
                            label="Runs on"
                            fullWidth
                            value={get('env') ?? 'ALL'}
                            onChange={(next) =>
                                set('env', next === 'ALL' ? null : next)
                            }
                            options={BrowseEnvironmentVals.map((env) => ({
                                value: env,
                                label: ENV_LABELS[env],
                            }))}
                        />
                    </Field>
                )}

                {(facets?.categories.length ?? 0) > 0 && (
                    <div className="flex flex-col gap-1">
                        <span className="text-[0.7rem] text-muted">Categories</span>
                        <div className="flex max-h-44 flex-col gap-1 overflow-y-auto rounded-lg border border-border p-2">
                            {facets?.categories.map((cat) => (
                                <label
                                    key={cat.id}
                                    className={`flex items-center gap-2 text-xs ${
                                        cat.parentId != null ? 'pl-4' : ''
                                    }`}
                                >
                                    <input
                                        type="checkbox"
                                        checked={ids('cats').includes(cat.id)}
                                        onChange={() => toggleId('cats', cat.id)}
                                    />
                                    <span className="truncate">{cat.name}</span>
                                </label>
                            ))}
                        </div>
                    </div>
                )}

                {/* Only meaningful once tags are actually selected, and the
                    app has no tag picker — tags arrive from a deep link into a
                    tag chip. Showing the combinator with nothing to combine
                    would be a control that provably does nothing. */}
                {get('tags') && (
                    <Check
                        label="Match any tag"
                        hint="Off means an item must carry every tag."
                        checked={bool('tagsOr')}
                        onChange={(v) => setBool('tagsOr', v)}
                    />
                )}

                {signedIn && (
                    <Check
                        label="Only mine"
                        checked={bool('mine')}
                        onChange={(v) => setBool('mine', v)}
                    />
                )}

                <Check
                    label="Include archived"
                    checked={bool('archived')}
                    onChange={(v) => setBool('archived', v)}
                />
            </Group>
        </div>
    )
}
