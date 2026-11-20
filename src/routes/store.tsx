import { useMemo, useRef, useState, type ReactNode } from 'react'
import { Link, useParams, useSearchParams } from 'react-router-dom'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { openUrl } from '@tauri-apps/plugin-opener'
import {
    FiCheck,
    FiChevronLeft,
    FiClock,
    FiExternalLink,
    FiGift,
    FiLock,
    FiPackage,
    FiSearch,
    FiShoppingBag,
    FiStar,
    FiTag,
    FiX,
    FiZap,
} from 'react-icons/fi'

import { api } from '~/lib/api/client'
import type { AppSummaryT, StoreFrontItemT, StoreFrontResponseT, StorePurchasesResponseT } from '~/lib/api/contract'
import { useApiEnv, webUrl } from '~/lib/api/env'
import { useAuth } from '~/lib/auth/provider'
import { messageOf } from '~/lib/ipc'
import { gameHeadingStyle, gameLook, type GameLook } from '~/lib/gamespice/games'
import {
    DEFAULT_STORE_FILTER,
    amount,
    balanceIn,
    currencyLabel,
    equippedSlot,
    filterItems,
    itemState,
    kindLabel,
    kindsOf,
    newIdemKey,
    saleOff,
    slotFor,
    type ItemStateT,
    type StoreFilterT,
    type StoreSortT,
} from '~/lib/gamespice/store'
import Select from '~/components/select'
import Gallery from '~/components/gallery'
import { BTN_GHOST, BTN_PRIMARY } from '~/components/play-center'

/**
 * **Store** — every game's store in one place, bought from with points.
 *
 * Points buys happen here, through `POST store/purchase` with an idempotency
 * key made when the member confirms. Anything paid in credits is a checkout,
 * and a checkout only ever happens on the website in the member's own browser
 * (`openUrl`), never inside this webview: card details are not typed into a
 * window a mod description can script.
 */

const SORTS: { value: StoreSortT; label: string }[] = [
    { value: 'featured', label: 'Featured' },
    { value: 'newest', label: 'Newest' },
    { value: 'price-asc', label: 'Price: low to high' },
    { value: 'price-desc', label: 'Price: high to low' },
    { value: 'name', label: 'Name' },
]

function useAppNames(ids: number[]) {
    return useQuery({
        queryKey: ['gamespice', 'app-names', ids.join(',')],
        queryFn: () => api.apps({ ids }),
        enabled: ids.length > 0,
        staleTime: 300_000,
    })
}

function lookOf(app: AppSummaryT | undefined, fallback: string): GameLook {
    return gameLook(app?.slug ?? app?.name ?? fallback)
}

function gradient(look: GameLook): string {
    return `linear-gradient(135deg, hsl(${look.hue} 60% 35%), hsl(${(look.hue + 40) % 360} 50% 18%))`
}

/** `/store`: the directory of open stores and, signed in, the member's inventory. */
export function StoreDirectoryRoute() {
    const { status } = useAuth()
    const [params, setParams] = useSearchParams()
    const tab = params.get('tab') === 'inventory' ? 'inventory' : 'browse'
    const dir = useQuery({ queryKey: ['store', 'directory'], queryFn: () => api.storeDirectory() })
    const ids = (dir.data?.stores ?? []).map((s) => s.appId)
    const names = useAppNames(ids)
    const appOf = (id: number) => names.data?.apps.find((a) => a.id === id)
    const points = useQuery({
        queryKey: ['gamespice', 'points'],
        queryFn: () => api.myPoints(),
        enabled: status === 'signedIn',
    })

    const setTab = (t: string) => setParams(t === 'browse' ? {} : { tab: t }, { replace: true })

    return (
        <div className="flex flex-col gap-6 p-4 md:p-6">
            <header className="relative overflow-hidden rounded-2xl border border-border bg-surface px-5 py-6 md:px-8 md:py-8">
                <div
                    aria-hidden
                    className="pointer-events-none absolute -right-16 -top-24 size-72 rounded-full opacity-30 blur-3xl"
                    style={{ background: 'var(--accent)' }}
                />
                <div className="relative flex flex-wrap items-end justify-between gap-4">
                    <div className="flex flex-col gap-1.5">
                        <p className="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-[0.18em] text-accent">
                            <FiShoppingBag aria-hidden /> GameSpice Store
                        </p>
                        <h1 className="text-2xl font-extrabold md:text-3xl">Gear up across every game</h1>
                        <p className="max-w-xl text-sm text-muted">
                            Skins, emotes and equipment from each game's own store. Spend the points you earn by playing; anything
                            paid opens on the website.
                        </p>
                    </div>
                    {points.data && (
                        <div className="flex items-center gap-3 rounded-xl border border-border bg-surface-2 px-4 py-3">
                            <FiStar aria-hidden className="text-accent" />
                            <div className="leading-tight">
                                <p className="text-[11px] uppercase tracking-wide text-muted">Your points</p>
                                <p className="text-xl font-extrabold tabular-nums">{amount(points.data.total)}</p>
                            </div>
                        </div>
                    )}
                </div>
            </header>

            <Tabs
                value={tab}
                onChange={setTab}
                tabs={[
                    { value: 'browse', label: 'Stores', icon: <FiShoppingBag aria-hidden /> },
                    { value: 'inventory', label: 'Inventory', icon: <FiPackage aria-hidden /> },
                ]}
            />

            {tab === 'browse' ? (
                <section aria-label="Stores" className="flex flex-col gap-3">
                    {dir.isPending && (
                        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3">
                            {[0, 1, 2].map((i) => (
                                <div key={i} className="h-56 animate-pulse rounded-2xl border border-border bg-surface" />
                            ))}
                        </div>
                    )}
                    {dir.isError && <ErrorNote error={dir.error} />}
                    {dir.data?.stores.length === 0 && (
                        <Empty icon={<FiShoppingBag />} title="No stores are open yet">
                            When a game opens its store it shows up here.
                        </Empty>
                    )}
                    <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3">
                        {dir.data?.stores.map((s, i) => {
                            const app = appOf(s.appId)
                            const look = lookOf(app, s.name)
                            const art = app?.images.banner ?? app?.images.card

                            return (
                                <Link
                                    key={s.appId}
                                    to={`/store/${s.appId}`}
                                    data-testid="store-card"
                                    className="gs-rise group flex flex-col overflow-hidden rounded-2xl border border-border bg-surface transition-transform duration-200 hover:-translate-y-0.5 hover:border-accent focus-visible:border-accent"
                                    style={{ borderTop: `3px solid ${look.accent}`, animationDelay: `${i * 50}ms` }}
                                >
                                    <div className="relative aspect-[16/8] overflow-hidden" style={{ background: gradient(look) }}>
                                        {art && (
                                            <img
                                                src={art}
                                                alt=""
                                                loading="lazy"
                                                className="size-full object-cover transition-transform duration-500 group-hover:scale-105"
                                            />
                                        )}
                                        <div className="absolute inset-0 bg-gradient-to-t from-black/70 via-black/10 to-transparent" />
                                        <span className="absolute bottom-3 right-3 rounded-full bg-black/60 px-2.5 py-1 text-[11px] font-semibold text-white backdrop-blur">
                                            {s.items} {s.items === 1 ? 'item' : 'items'}
                                        </span>
                                    </div>
                                    <div className="flex items-center justify-between gap-3 px-4 py-3">
                                        <h2 className="truncate text-lg font-extrabold" style={gameHeadingStyle(look)}>
                                            {app?.name ?? s.name}
                                        </h2>
                                        <span className="text-xs font-semibold text-muted transition-colors group-hover:text-accent">
                                            Shop
                                        </span>
                                    </div>
                                </Link>
                            )
                        })}
                    </div>
                </section>
            ) : (
                <Inventory names={names.data?.apps ?? []} />
            )}
        </div>
    )
}

function Inventory({ names }: { names: AppSummaryT[] }) {
    const { status, signIn } = useAuth()
    const on = status === 'signedIn'
    const list = useQuery({ queryKey: ['store', 'purchases'], queryFn: () => api.storePurchases(), enabled: on })
    const otherIds = [...new Set((list.data?.purchases ?? []).map((p) => p.appId))].filter(
        (id) => !names.some((a) => a.id === id)
    )
    const more = useAppNames(otherIds)
    const all = [...names, ...(more.data?.apps ?? [])]

    if (!on)
        return (
            <Empty icon={<FiLock />} title="Sign in to see your inventory">
                <button type="button" onClick={() => void signIn()} className={`${BTN_PRIMARY} mt-2`}>
                    Sign in
                </button>
            </Empty>
        )
    if (list.isPending) return <RowSkeleton rows={4} />
    if (list.isError) return <ErrorNote error={list.error} />
    if (list.data.purchases.length === 0)
        return (
            <Empty icon={<FiPackage />} title="Nothing here yet">
                What you buy in any game's store shows up here.
            </Empty>
        )

    return (
        <ul className="flex flex-col gap-2" aria-label="Inventory">
            {list.data.purchases.map((p) => (
                <InventoryRow key={p.id} p={p} app={all.find((a) => a.id === p.appId)} />
            ))}
        </ul>
    )
}

function InventoryRow({ p, app }: { p: StorePurchasesResponseT['purchases'][number]; app: AppSummaryT | undefined }) {
    const look = lookOf(app, String(p.appId))

    return (
        <li className="flex items-center gap-3 rounded-xl border border-border bg-surface p-2.5 pr-4">
            <div className="size-12 shrink-0 overflow-hidden rounded-lg" style={{ background: gradient(look) }}>
                {p.item.imageUrl && <img src={p.item.imageUrl} alt="" className="size-full object-cover" loading="lazy" />}
            </div>
            <div className="min-w-0 flex-1">
                <p className="truncate font-semibold">{p.item.name}</p>
                <p className="truncate text-xs text-muted">
                    <span style={{ color: look.accent }}>{app?.name ?? `Game ${p.appId}`}</span> · {kindLabel(p.item.kind)} ·{' '}
                    {new Date(p.createdAt).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' })}
                </p>
            </div>
            <span
                className={`rounded-full px-2 py-0.5 text-[11px] font-semibold ${
                    p.live ? 'bg-success/15 text-success' : 'bg-surface-2 text-muted'
                }`}
            >
                {p.live ? (p.expiresAt ? 'Active' : 'Owned') : p.status.toLowerCase()}
            </span>
            <Link to={`/store/${p.appId}`} className="hidden text-xs text-muted hover:text-accent sm:inline">
                Store
            </Link>
        </li>
    )
}

/** `/store/:appId`: one game's store. */
export function GameStoreRoute() {
    const appId = Number(useParams().appId)
    const { status } = useAuth()
    const on = status === 'signedIn'
    const [filter, setFilter] = useState<StoreFilterT>(DEFAULT_STORE_FILTER)
    const [sort, setSort] = useState<StoreSortT>('featured')
    const [openId, setOpenId] = useState<number | null>(null)
    const front = useQuery({
        queryKey: ['store', 'front', appId],
        queryFn: () => api.storeFront(appId),
        enabled: Number.isInteger(appId) && appId > 0,
    })
    const loadout = useQuery({ queryKey: ['store', 'loadout', appId], queryFn: () => api.storeLoadout(appId), enabled: on })
    const names = useAppNames(Number.isInteger(appId) && appId > 0 ? [appId] : [])
    const app = names.data?.apps[0]
    const look = lookOf(app, front.data?.owner.name ?? String(appId))
    const items = useMemo(
        () => (front.data ? filterItems(front.data.items, front.data, filter, sort) : []),
        [front.data, filter, sort]
    )
    const open = front.data?.items.find((i) => i.id === openId) ?? null

    if (front.isPending)
        return (
            <div className="flex flex-col gap-5 p-4 md:p-6">
                <div className="h-48 animate-pulse rounded-2xl bg-surface" />
                <GridSkeleton />
            </div>
        )
    if (front.isError)
        return (
            <div className="p-4 md:p-6">
                <BackLink />
                <ErrorNote error={front.error} />
            </div>
        )

    const f = front.data
    const art = app?.images.banner ?? app?.images.card
    const kinds = kindsOf(f.items)
    const currencies = [...new Set(f.items.map((i) => i.currency))]

    return (
        <div className="flex flex-col gap-5 p-4 md:p-6">
            <BackLink />

            <header
                className="relative overflow-hidden rounded-2xl border border-border"
                style={{ background: gradient(look), borderTop: `3px solid ${look.accent}` }}
            >
                {art && <img src={art} alt="" className="absolute inset-0 size-full object-cover opacity-60" />}
                <div className="absolute inset-0 bg-gradient-to-r from-black/85 via-black/55 to-black/10" />
                <div className="relative flex flex-wrap items-end justify-between gap-4 px-5 py-7 md:px-8 md:py-10">
                    <div className="flex flex-col gap-1">
                        <p className="text-[11px] font-semibold uppercase tracking-[0.18em] text-white/70">Store</p>
                        <h1 className="text-3xl font-extrabold md:text-4xl" style={gameHeadingStyle(look)}>
                            {app?.name ?? f.owner.name}
                        </h1>
                        <p className="text-sm text-white/75">
                            {f.items.length} {f.items.length === 1 ? 'item' : 'items'}
                            {f.owned.length > 0 && ` · ${f.owned.length} owned`}
                        </p>
                    </div>
                    <Wallet front={f} />
                </div>
            </header>

            {!f.availability.open && (
                <p role="status" className="flex items-center gap-2 rounded-xl border border-warning/40 bg-warning/10 px-4 py-3 text-sm">
                    <FiLock aria-hidden className="text-warning" />
                    This store is closed{f.availability.reason ? `: ${f.availability.reason}` : ''}. You can still browse it.
                </p>
            )}

            <div className="flex flex-col gap-5 lg:flex-row lg:items-start">
                <div className="flex min-w-0 flex-1 flex-col gap-4">
                    <div className="flex flex-wrap items-end gap-2" role="search">
                        <label className="relative min-w-48 flex-1">
                            <span className="sr-only">Search this store</span>
                            <FiSearch aria-hidden className="absolute left-3 top-1/2 -translate-y-1/2 text-muted" />
                            <input
                                value={filter.q}
                                onChange={(e) => setFilter({ ...filter, q: e.target.value })}
                                placeholder="Search items"
                                className="h-9 w-full rounded-lg border border-border bg-surface pl-9 pr-3 text-sm outline-none focus:border-accent"
                            />
                        </label>
                        {kinds.length > 1 && (
                            <Select
                                label="Type"
                                value={filter.kind}
                                onChange={(v) => setFilter({ ...filter, kind: v })}
                                options={[{ value: 'all', label: 'All types' }, ...kinds.map((k) => ({ value: k, label: kindLabel(k) }))]}
                            />
                        )}
                        {currencies.length > 1 && (
                            <Select
                                label="Currency"
                                value={filter.currency}
                                onChange={(v) => setFilter({ ...filter, currency: v })}
                                options={[
                                    { value: 'all', label: 'Any currency' },
                                    ...currencies.map((c) => ({ value: c, label: currencyLabel(f, c) })),
                                ]}
                            />
                        )}
                        <Select
                            label="Show"
                            value={filter.show}
                            onChange={(v) => setFilter({ ...filter, show: v as StoreFilterT['show'] })}
                            options={[
                                { value: 'all', label: 'Everything' },
                                { value: 'sale', label: 'On sale' },
                                ...(f.wallet
                                    ? [
                                          { value: 'affordable', label: 'I can afford' },
                                          { value: 'owned', label: 'Owned' },
                                      ]
                                    : []),
                            ]}
                        />
                        <Select label="Sort" value={sort} onChange={(v) => setSort(v)} options={SORTS} />
                    </div>

                    {f.items.length === 0 ? (
                        <Empty icon={<FiShoppingBag />} title="Nothing on the shelves yet">
                            This game has not listed any items.
                        </Empty>
                    ) : items.length === 0 ? (
                        <Empty icon={<FiSearch />} title="No items match">
                            <button
                                type="button"
                                className={`${BTN_GHOST} mt-2`}
                                onClick={() => setFilter(DEFAULT_STORE_FILTER)}
                            >
                                Clear filters
                            </button>
                        </Empty>
                    ) : (
                        <ul className="grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-4" aria-label="Items">
                            {items.map((item, i) => (
                                <li key={item.id} className="gs-rise" style={{ animationDelay: `${Math.min(i, 12) * 30}ms` }}>
                                    <ItemCard
                                        item={item}
                                        front={f}
                                        look={look}
                                        equipped={equippedSlot(loadout.data, appId, item.key) !== null}
                                        onOpen={() => setOpenId(item.id)}
                                    />
                                </li>
                            ))}
                        </ul>
                    )}
                </div>

                {on && <LoadoutPanel appId={appId} front={f} loadout={loadout} look={look} slug={app?.slug ?? null} />}
            </div>

            {open && (
                <ItemDialog
                    item={open}
                    front={f}
                    look={look}
                    appId={appId}
                    slug={app?.slug ?? null}
                    loadout={loadout.data}
                    onClose={() => setOpenId(null)}
                />
            )}
        </div>
    )
}

function BackLink() {
    return (
        <Link to="/store" className="flex w-fit items-center gap-1 text-xs text-muted hover:text-accent">
            <FiChevronLeft aria-hidden /> All stores
        </Link>
    )
}

function Wallet({ front }: { front: StoreFrontResponseT }) {
    const { status, signIn } = useAuth()

    if (!front.wallet)
        return status === 'signedIn' ? null : (
            <button type="button" onClick={() => void signIn()} className={BTN_PRIMARY}>
                Sign in to buy
            </button>
        )

    const chips: { label: string; value: number; icon: ReactNode }[] = [
        { label: front.labels.points, value: front.wallet.points, icon: <FiStar aria-hidden /> },
    ]

    if (front.availability.appPoints && front.wallet.appPoints !== null)
        chips.push({ label: front.labels.appPoints, value: front.wallet.appPoints, icon: <FiZap aria-hidden /> })
    if (front.availability.credits && front.wallet.credits !== null)
        chips.push({ label: front.labels.credits, value: front.wallet.credits, icon: <FiGift aria-hidden /> })

    return (
        <ul className="flex flex-wrap gap-2" aria-label="Wallet" data-testid="store-wallet">
            {chips.map((c) => (
                <li key={c.label} className="flex items-center gap-2 rounded-xl bg-black/55 px-3 py-2 text-white backdrop-blur">
                    <span className="text-white/70">{c.icon}</span>
                    <span className="leading-tight">
                        <span className="block text-[10px] uppercase tracking-wide text-white/60">{c.label}</span>
                        <span className="block text-base font-extrabold tabular-nums">{amount(c.value)}</span>
                    </span>
                </li>
            ))}
        </ul>
    )
}

const STATE_TEXT: Record<ItemStateT, string> = {
    owned: 'Owned',
    subscribed: 'Subscribed',
    buy: 'Buy',
    short: 'Not enough',
    checkout: 'Buy on website',
    soon: 'Coming soon',
    soldout: 'Sold out',
    closed: 'Store closed',
    signedout: 'Sign in to buy',
}

function Price({ item, front, large }: { item: StoreFrontItemT; front: StoreFrontResponseT; large?: boolean }) {
    const off = saleOff(item)

    return (
        <span className="flex items-baseline gap-1.5">
            <span className={`font-extrabold tabular-nums ${large ? 'text-2xl' : 'text-base'}`}>
                {item.price === 0 ? 'Free' : amount(item.price)}
            </span>
            {item.price > 0 && <span className="text-xs text-muted">{currencyLabel(front, item.currency)}</span>}
            {off > 0 && <span className="text-xs text-muted line-through tabular-nums">{amount(item.listPrice)}</span>}
            {item.billing === 'SUBSCRIPTION' && item.periodDays && (
                <span className="text-xs text-muted">/ {item.periodDays} days</span>
            )}
        </span>
    )
}

function ItemArt({ item, look, className = '' }: { item: StoreFrontItemT; look: GameLook; className?: string }) {
    const src = item.imageUrl ?? item.images[0]

    return (
        <div className={`relative overflow-hidden ${className}`} style={{ background: gradient(look) }}>
            {src ? (
                <img src={src} alt="" loading="lazy" className="size-full object-cover transition-transform duration-500 group-hover:scale-105" />
            ) : (
                <div className="flex size-full items-center justify-center">
                    <span className="text-5xl font-black text-white/25" style={{ fontFamily: gameHeadingStyle(look).fontFamily }}>
                        {item.name.charAt(0).toUpperCase()}
                    </span>
                </div>
            )}
        </div>
    )
}

function ItemCard({
    item,
    front,
    look,
    equipped,
    onOpen,
}: {
    item: StoreFrontItemT
    front: StoreFrontResponseT
    look: GameLook
    equipped: boolean
    onOpen: () => void
}) {
    const state = itemState(item, front)
    const off = saleOff(item)
    const mine = state === 'owned' || state === 'subscribed'

    return (
        <button
            type="button"
            onClick={onOpen}
            data-testid="store-item"
            className="group flex size-full flex-col overflow-hidden rounded-xl border border-border bg-surface text-left transition-[transform,border-color,box-shadow] duration-200 hover:-translate-y-0.5 hover:border-accent hover:shadow-lg focus-visible:border-accent"
        >
            <div className="relative">
                <ItemArt item={item} look={look} className="aspect-square" />
                <div className="absolute left-2 top-2 flex flex-wrap gap-1">
                    {off > 0 && (
                        <span className="rounded-md bg-danger px-1.5 py-0.5 text-[11px] font-bold text-white">-{off}%</span>
                    )}
                    {item.window === 'soon' && (
                        <span className="rounded-md bg-black/65 px-1.5 py-0.5 text-[11px] font-semibold text-white">Soon</span>
                    )}
                </div>
                {mine && (
                    <span className="absolute right-2 top-2 flex items-center gap-1 rounded-md bg-success px-1.5 py-0.5 text-[11px] font-bold text-white">
                        <FiCheck aria-hidden /> {equipped ? 'Equipped' : STATE_TEXT[state]}
                    </span>
                )}
                {item.stockLeft !== null && item.stockLeft > 0 && item.stockLeft <= 10 && (
                    <span className="absolute bottom-2 left-2 rounded-md bg-black/65 px-1.5 py-0.5 text-[11px] text-white">
                        {item.stockLeft} left
                    </span>
                )}
            </div>
            <div className="flex flex-1 flex-col gap-1 p-3">
                <p className="text-[10px] font-semibold uppercase tracking-wider" style={{ color: look.accent }}>
                    {item.category ?? kindLabel(item.kind)}
                </p>
                <p className="line-clamp-1 font-semibold">{item.name}</p>
                <div className="mt-auto flex items-center justify-between gap-2 pt-1">
                    <Price item={item} front={front} />
                    {state === 'checkout' && <FiExternalLink aria-label="Opens the website" className="text-muted" />}
                    {state === 'short' && <span className="text-[10px] text-warning">Need more</span>}
                </div>
            </div>
        </button>
    )
}

function ItemDialog({
    item,
    front,
    look,
    appId,
    slug,
    loadout,
    onClose,
}: {
    item: StoreFrontItemT
    front: StoreFrontResponseT
    look: GameLook
    appId: number
    slug: string | null
    loadout: Parameters<typeof equippedSlot>[0]
    onClose: () => void
}) {
    const env = useApiEnv()
    const qc = useQueryClient()
    const { signIn } = useAuth()
    const state = itemState(item, front)
    const [confirming, setConfirming] = useState(false)
    const idem = useRef<string | null>(null)
    const balance = balanceIn(front, item.currency)
    const slot = slotFor(item)
    const equipped = equippedSlot(loadout, appId, item.key)

    const buy = useMutation({
        mutationFn: () => {
            idem.current ??= newIdemKey()

            return api.storePurchase({ itemId: item.id, idemKey: idem.current, price: item.price, currency: item.currency })
        },
        onSuccess: () => {
            idem.current = null
            setConfirming(false)
            void qc.invalidateQueries({ queryKey: ['store', 'front', appId] })
            void qc.invalidateQueries({ queryKey: ['store', 'purchases'] })
            void qc.invalidateQueries({ queryKey: ['gamespice', 'points'] })
        },
    })
    const equip = useMutation({
        mutationFn: (key: string | null) =>
            api.storeLoadoutPut({ appId, slot: equipped ?? slot ?? 'other', key, store: `app:${appId}` }),
        onSuccess: (data) => {
            qc.setQueryData(['store', 'loadout', appId], data)
            void qc.invalidateQueries({ queryKey: ['store', 'front', appId] })
        },
    })

    const gallery = [...new Set([item.imageUrl, ...item.images].filter((u): u is string => !!u))].slice(1)
    const mine = state === 'owned' || state === 'subscribed'

    return (
        <div
            className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm"
            onClick={(e) => e.target === e.currentTarget && onClose()}
            onKeyDown={(e) => e.key === 'Escape' && onClose()}
        >
            <div
                role="dialog"
                aria-modal="true"
                aria-label={item.name}
                className="gs-rise flex max-h-[90vh] w-full max-w-2xl flex-col overflow-hidden rounded-2xl border border-border bg-surface shadow-2xl md:flex-row"
            >
                <ItemArt item={item} look={look} className="aspect-square w-full shrink-0 md:w-72" />
                <div className="flex min-w-0 flex-1 flex-col gap-3 overflow-y-auto p-5">
                    <div className="flex items-start justify-between gap-3">
                        <div>
                            <p className="text-[11px] font-semibold uppercase tracking-wider" style={{ color: look.accent }}>
                                {kindLabel(item.kind)}
                                {item.category ? ` · ${item.category}` : ''}
                            </p>
                            <h2 className="text-xl font-extrabold">{item.name}</h2>
                        </div>
                        <button type="button" onClick={onClose} aria-label="Close" className="rounded-lg p-1.5 text-muted hover:bg-surface-hover">
                            <FiX />
                        </button>
                    </div>

                    <Price item={item} front={front} large />
                    {item.onSale && item.saleEndsAt && (
                        <p className="flex items-center gap-1.5 text-xs text-danger">
                            <FiClock aria-hidden /> Sale ends {new Date(item.saleEndsAt).toLocaleDateString('en-US', { month: 'short', day: 'numeric' })}
                        </p>
                    )}

                    {item.description && <p className="whitespace-pre-line text-sm text-muted">{item.description}</p>}

                    {item.bundle.length > 0 && (
                        <div className="flex flex-col gap-1">
                            <p className="text-xs font-semibold">Includes</p>
                            <ul className="flex flex-wrap gap-1.5">
                                {item.bundle.map((b) => (
                                    <li key={b.id} className="flex items-center gap-1 rounded-md bg-surface-2 px-2 py-0.5 text-xs">
                                        <FiTag aria-hidden className="text-muted" /> {b.name}
                                    </li>
                                ))}
                            </ul>
                        </div>
                    )}

                    {gallery.length > 0 && (
                        <Gallery items={gallery.map((url, i) => ({ id: String(i), url, title: item.name }))} />
                    )}

                    <dl className="grid grid-cols-2 gap-2 text-xs">
                        {item.stockLeft !== null && <Fact label="In stock" value={amount(item.stockLeft)} />}
                        {item.maxPerPlayer > 1 && <Fact label="Limit" value={`${item.maxPerPlayer} each`} />}
                        {item.availableUntil && (
                            <Fact label="Available until" value={new Date(item.availableUntil).toLocaleDateString('en-US')} />
                        )}
                        {balance !== null && item.currency !== 'CREDITS' && (
                            <Fact label={`Your ${currencyLabel(front, item.currency)}`} value={amount(balance)} />
                        )}
                    </dl>

                    <div className="mt-auto flex flex-col gap-2 border-t border-border pt-3">
                        {(buy.error || equip.error) && <ErrorNote error={buy.error ?? equip.error} />}
                        {buy.isSuccess && (
                            <p role="status" className="flex items-center gap-1.5 text-sm font-semibold text-success">
                                <FiCheck aria-hidden /> {buy.data.replay ? 'Already yours.' : 'Bought. It is in your inventory.'}
                            </p>
                        )}

                        {state === 'buy' && !confirming && (
                            <button type="button" className={BTN_PRIMARY} onClick={() => setConfirming(true)}>
                                {item.price === 0 ? 'Claim' : `Buy for ${amount(item.price)} ${currencyLabel(front, item.currency)}`}
                            </button>
                        )}
                        {state === 'buy' && confirming && (
                            <div className="flex flex-col gap-2 rounded-xl border border-accent/50 bg-accent/10 p-3" data-testid="store-confirm">
                                <p className="text-sm">
                                    Spend <b>{amount(item.price)} {currencyLabel(front, item.currency)}</b>? You will have{' '}
                                    <b>{amount((balance ?? 0) - item.price)}</b> left.
                                </p>
                                <div className="flex gap-2">
                                    <button type="button" className={BTN_PRIMARY} disabled={buy.isPending} onClick={() => buy.mutate()}>
                                        {buy.isPending ? 'Buying…' : 'Confirm'}
                                    </button>
                                    <button type="button" className={BTN_GHOST} disabled={buy.isPending} onClick={() => setConfirming(false)}>
                                        Cancel
                                    </button>
                                </div>
                            </div>
                        )}
                        {state === 'short' && (
                            <p className="text-sm text-warning">
                                You need {amount(item.price - (balance ?? 0))} more {currencyLabel(front, item.currency)}. Play to earn
                                more.
                            </p>
                        )}
                        {state === 'checkout' && (
                            <button
                                type="button"
                                className={BTN_PRIMARY}
                                onClick={() => void openUrl(webUrl(env, front.storeUrl))}
                            >
                                <FiExternalLink aria-hidden /> Buy on the website
                            </button>
                        )}
                        {state === 'signedout' && (
                            <button type="button" className={BTN_PRIMARY} onClick={() => void signIn()}>
                                Sign in to buy
                            </button>
                        )}
                        {(state === 'soon' || state === 'soldout' || state === 'closed') && (
                            <p className="text-sm text-muted">{STATE_TEXT[state]}.</p>
                        )}

                        {mine && slot !== null && (
                            <button
                                type="button"
                                className={equipped ? BTN_GHOST : BTN_PRIMARY}
                                disabled={equip.isPending}
                                onClick={() => equip.mutate(equipped ? null : item.key)}
                            >
                                {equip.isPending ? 'Saving…' : equipped ? `Unequip from ${equipped}` : `Equip in ${slot}`}
                            </button>
                        )}
                        {mine && slot === null && (
                            <Link to={slug ? `/games/${slug}/avatar` : '/avatar'} className={BTN_PRIMARY}>
                                Wear it in the avatar studio
                            </Link>
                        )}
                        {state === 'checkout' && (
                            <p className="text-[11px] text-muted">Paid items check out on the website in your own browser.</p>
                        )}
                    </div>
                </div>
            </div>
        </div>
    )
}

function LoadoutPanel({
    appId,
    front,
    loadout,
    look,
    slug,
}: {
    appId: number
    front: StoreFrontResponseT
    loadout: ReturnType<typeof useQuery<Awaited<ReturnType<typeof api.storeLoadout>>>>
    look: GameLook
    slug: string | null
}) {
    const qc = useQueryClient()
    const clear = useMutation({
        mutationFn: (slot: string) => api.storeLoadoutPut({ appId, slot, key: null }),
        onSuccess: (data) => qc.setQueryData(['store', 'loadout', appId], data),
    })
    const byKey = (key: string) => front.items.find((i) => i.key === key)

    return (
        <aside
            aria-label="Loadout"
            data-testid="store-loadout"
            className="flex w-full shrink-0 flex-col gap-3 rounded-2xl border border-border bg-surface p-4 lg:sticky lg:top-4 lg:w-72"
        >
            <div className="flex items-center justify-between">
                <h2 className="font-bold">Loadout</h2>
                {loadout.data && (
                    <span className="text-[11px] text-muted tabular-nums">
                        {loadout.data.loadout.length} / {loadout.data.maxSlots}
                    </span>
                )}
            </div>
            {loadout.isPending && <RowSkeleton rows={2} />}
            {loadout.isError && <ErrorNote error={loadout.error} />}
            {loadout.data?.loadout.length === 0 && (
                <p className="text-sm text-muted">Nothing equipped. Open an item you own and equip it.</p>
            )}
            <ul className="flex flex-col gap-2">
                {loadout.data?.loadout.map((l) => {
                    const item = byKey(l.key)

                    return (
                        <li key={l.slot} className="flex items-center gap-2.5 rounded-xl bg-surface-2 p-2">
                            {item ? (
                                <ItemArt item={item} look={look} className="size-10 shrink-0 rounded-lg" />
                            ) : (
                                <div className="size-10 shrink-0 rounded-lg" style={{ background: gradient(look) }} />
                            )}
                            <div className="min-w-0 flex-1 leading-tight">
                                <p className="text-[10px] uppercase tracking-wide text-muted">{l.slot}</p>
                                <p className="truncate text-sm font-semibold">{item?.name ?? l.key}</p>
                            </div>
                            <button
                                type="button"
                                aria-label={`Unequip ${l.slot}`}
                                disabled={clear.isPending}
                                onClick={() => clear.mutate(l.slot)}
                                className="rounded-md p-1 text-muted hover:bg-surface-hover hover:text-danger"
                            >
                                <FiX />
                            </button>
                        </li>
                    )
                })}
            </ul>
            <Link to={slug ? `/games/${slug}/avatar` : '/avatar'} className="text-xs text-muted hover:text-accent">
                Wearables go on your avatar
            </Link>
        </aside>
    )
}

function Fact({ label, value }: { label: string; value: string }) {
    return (
        <div className="rounded-lg bg-surface-2 px-2.5 py-1.5">
            <dt className="text-[10px] uppercase tracking-wide text-muted">{label}</dt>
            <dd className="font-semibold tabular-nums">{value}</dd>
        </div>
    )
}

export function Tabs({
    value,
    onChange,
    tabs,
}: {
    value: string
    onChange: (v: string) => void
    tabs: { value: string; label: string; icon?: ReactNode }[]
}) {
    return (
        <div role="tablist" className="flex w-fit gap-1 rounded-xl border border-border bg-surface p-1">
            {tabs.map((t) => (
                <button
                    key={t.value}
                    type="button"
                    role="tab"
                    aria-selected={t.value === value}
                    onClick={() => onChange(t.value)}
                    className={`flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-sm font-semibold transition-colors ${
                        t.value === value ? 'bg-accent text-accent-foreground' : 'text-muted hover:text-foreground'
                    }`}
                >
                    {t.icon}
                    {t.label}
                </button>
            ))}
        </div>
    )
}

export function Empty({ icon, title, children }: { icon: ReactNode; title: string; children?: ReactNode }) {
    return (
        <div className="flex flex-col items-center gap-2 rounded-2xl border border-dashed border-border px-6 py-12 text-center">
            <span className="flex size-12 items-center justify-center rounded-full bg-accent/10 text-xl text-accent">{icon}</span>
            <p className="font-semibold">{title}</p>
            {children && <div className="text-sm text-muted">{children}</div>}
        </div>
    )
}

export function ErrorNote({ error }: { error: unknown }) {
    return (
        <p role="alert" className="rounded-lg bg-danger/10 px-3 py-2 text-sm text-danger">
            {messageOf(error)}
        </p>
    )
}

export function RowSkeleton({ rows }: { rows: number }) {
    return (
        <div className="flex flex-col gap-2" aria-busy="true">
            {Array.from({ length: rows }, (_, i) => (
                <div key={i} className="h-14 animate-pulse rounded-xl bg-surface-2" />
            ))}
        </div>
    )
}

function GridSkeleton() {
    return (
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-4" aria-busy="true">
            {Array.from({ length: 8 }, (_, i) => (
                <div key={i} className="aspect-[3/4] animate-pulse rounded-xl bg-surface" />
            ))}
        </div>
    )
}
