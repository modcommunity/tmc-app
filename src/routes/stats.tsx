import { useState, type ReactNode } from 'react'
import { Link, useParams } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { FiActivity, FiAward, FiBarChart2, FiCalendar, FiChevronLeft, FiLock, FiShoppingBag, FiStar, FiTrendingUp, FiZap } from 'react-icons/fi'

import { api } from '~/lib/api/client'
import type { AchievementsResponseT, AppSummaryT, MeStatsResponseT } from '~/lib/api/contract'
import { formatStat } from '~/lib/api/social'
import { useAuth } from '~/lib/auth/provider'
import { gameHeadingStyle, gameLook, type GameLook } from '~/lib/gamespice/games'
import { completion, currentStreak, lastPlayedLabel, longestStreak, playShare } from '~/lib/gamespice/stats'
import { ActivityBars, ProgressRing, ShareBar, Sparkline, type ShareSegment } from '~/components/gs-charts'
import { BTN_PRIMARY } from '~/components/play-center'
import { Leaderboards } from '~/routes/game'
import { Empty, ErrorNote, RowSkeleton, Tabs } from '~/routes/store'

/**
 * **Stats** — what a member has played across GameSpice: days played, streaks,
 * the share each game takes, achievements, and per game the stats its servers
 * file plus that game's leaderboards. Playtime is counted in days played
 * (the site keeps one row per member, game and UTC day), never in invented hours.
 */

function useApps(ids: number[]) {
    return useQuery({
        queryKey: ['gamespice', 'app-names', ids.join(',')],
        queryFn: () => api.apps({ ids }),
        enabled: ids.length > 0,
        staleTime: 300_000,
    })
}

function lookFor(app: AppSummaryT | undefined, id: number): GameLook {
    return gameLook(app?.slug ?? app?.name ?? String(id))
}

/** `/stats`: the overview. */
export function StatsRoute() {
    const { status, signIn } = useAuth()
    const on = status === 'signedIn'
    const stats = useQuery({ queryKey: ['stats', 'overview'], queryFn: () => api.meStats(), enabled: on })
    const apps = useApps((stats.data?.games ?? []).map((g) => g.appId))
    const appOf = (id: number) => apps.data?.apps.find((a) => a.id === id)

    return (
        <div className="flex flex-col gap-6 p-4 md:p-6">
            <header className="flex flex-col gap-1.5">
                <p className="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-[0.18em] text-accent">
                    <FiBarChart2 aria-hidden /> Stats
                </p>
                <h1 className="text-2xl font-extrabold md:text-3xl">Your play, game by game</h1>
                <p className="max-w-xl text-sm text-muted">Days played, streaks, achievements and where you rank.</p>
            </header>

            {!on && (
                <Empty icon={<FiLock />} title="Sign in to see your stats">
                    <button type="button" className={`${BTN_PRIMARY} mt-2`} onClick={() => void signIn()}>
                        Sign in
                    </button>
                </Empty>
            )}
            {on && stats.isPending && <OverviewSkeleton />}
            {on && stats.isError && <ErrorNote error={stats.error} />}
            {stats.data && (stats.data.games.length === 0 ? (
                <Empty icon={<FiActivity />} title="No games played yet">
                    Play any GameSpice game and your days, streaks and achievements show up here.
                    <div className="mt-3">
                        <Link to="/games" className={BTN_PRIMARY}>
                            Find a game
                        </Link>
                    </div>
                </Empty>
            ) : (
                <Overview data={stats.data} appOf={appOf} />
            ))}
        </div>
    )
}

function Overview({ data, appOf }: { data: MeStatsResponseT; appOf: (id: number) => AppSummaryT | undefined }) {
    const streak = currentStreak(data.recent)
    const best = longestStreak(data.recent)
    const share = playShare(data.games)
    const segments: ShareSegment[] = share.map((s) => {
        const app = s.appId === null ? undefined : appOf(s.appId)

        return {
            key: String(s.appId ?? 'other'),
            label: s.appId === null ? 'Other games' : (app?.name ?? `Game ${s.appId}`),
            value: s.days,
            share: s.share,
            color: s.appId === null ? 'var(--border)' : lookFor(app, s.appId).accent,
        }
    })
    const games = [...data.games].sort((a, b) => (b.lastPlayed ?? '').localeCompare(a.lastPlayed ?? '') || b.daysPlayed - a.daysPlayed)

    return (
        <>
            <ul className="grid grid-cols-2 gap-3 lg:grid-cols-4" aria-label="Totals" data-testid="stats-totals">
                <Tile icon={<FiCalendar />} label="Days played" value={data.totals.daysPlayed} hint={`${data.totals.activeDays} distinct days`} />
                <Tile icon={<FiZap />} label="Current streak" value={streak} hint={`Best ${best} in 30 days`} unit={streak === 1 ? 'day' : 'days'} />
                <Tile icon={<FiAward />} label="Achievements" value={data.totals.achievements} hint={`${data.totals.achievementPoints.toLocaleString('en-US')} points`} />
                <Tile icon={<FiStar />} label="Games played" value={data.totals.games} hint={`${data.games.filter((g) => g.recent.some((v) => v > 0)).length} in the last 30 days`} />
            </ul>

            <div className="grid grid-cols-1 gap-4 lg:grid-cols-5">
                <Panel title="Last 30 days" icon={<FiActivity />} className="lg:col-span-3">
                    <ActivityBars series={data.recent} since={data.since} />
                </Panel>
                <Panel title="Where your days go" icon={<FiTrendingUp />} className="lg:col-span-2">
                    {segments.length > 0 ? <ShareBar segments={segments} unit="days" /> : <p className="text-sm text-muted">No days yet.</p>}
                </Panel>
            </div>

            <section className="flex flex-col gap-3" aria-label="Games">
                <h2 className="text-lg font-bold">Games</h2>
                <ul className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
                    {games.map((g, i) => {
                        const app = appOf(g.appId)
                        const look = lookFor(app, g.appId)
                        const pct = completion(g.achievements)

                        return (
                            <li key={g.appId} className="gs-rise" style={{ animationDelay: `${Math.min(i, 9) * 40}ms` }}>
                                <Link
                                    to={`/stats/${g.appId}`}
                                    data-testid="stats-game"
                                    className="group flex h-full flex-col gap-3 rounded-2xl border border-border bg-surface p-4 transition-[transform,border-color] duration-200 hover:-translate-y-0.5 hover:border-accent"
                                    style={{ borderLeft: `3px solid ${look.accent}` }}
                                >
                                    <div className="flex items-center gap-3">
                                        <div
                                            className="size-11 shrink-0 overflow-hidden rounded-xl"
                                            style={{ background: `linear-gradient(135deg, hsl(${look.hue} 60% 35%), hsl(${(look.hue + 40) % 360} 50% 18%))` }}
                                        >
                                            {(app?.images.icon ?? app?.images.card) && (
                                                <img src={app?.images.icon ?? app?.images.card ?? ''} alt="" className="size-full object-cover" />
                                            )}
                                        </div>
                                        <div className="min-w-0 flex-1">
                                            <p className="truncate font-extrabold" style={gameHeadingStyle(look)}>
                                                {app?.name ?? `Game ${g.appId}`}
                                            </p>
                                            <p className="text-xs text-muted">Last played {lastPlayedLabel(g.lastPlayed)}</p>
                                        </div>
                                        {g.achievements && g.achievements.total > 0 && (
                                            <ProgressRing value={pct} accent={look.accent} size={48} label="Achievements" />
                                        )}
                                    </div>
                                    <Sparkline series={g.recent} accent={look.accent} label={`${g.recent.filter((v) => v > 0).length} days played in the last 30`} />
                                    <dl className="grid grid-cols-3 gap-2 text-xs">
                                        <Mini label="Days" value={g.daysPlayed.toLocaleString('en-US')} />
                                        <Mini label="Last 30" value={String(g.recent.filter((v) => v > 0).length)} />
                                        <Mini
                                            label="Achievements"
                                            value={g.achievements ? `${g.achievements.unlocked}/${g.achievements.total}` : '-'}
                                        />
                                    </dl>
                                </Link>
                            </li>
                        )
                    })}
                </ul>
            </section>
        </>
    )
}

/** `/stats/:appId`: one game's stats, achievements and leaderboards. */
export function GameStatsRoute() {
    const appId = Number(useParams().appId)
    const valid = Number.isInteger(appId) && appId > 0
    const { status } = useAuth()
    const on = status === 'signedIn'
    const [tab, setTab] = useState('overview')
    const apps = useApps(valid ? [appId] : [])
    const app = apps.data?.apps[0]
    const look = lookFor(app, appId)
    const overview = useQuery({ queryKey: ['stats', 'overview'], queryFn: () => api.meStats(), enabled: on })
    const game = overview.data?.games.find((g) => g.appId === appId)
    const mine = useQuery({ queryKey: ['stats', 'me', appId], queryFn: () => api.statsMe(appId), enabled: on && valid })
    const ach = useQuery({
        queryKey: ['stats', 'achievements', appId],
        queryFn: () => api.statsAchievements(appId),
        enabled: on && valid,
    })
    const art = app?.images.banner ?? app?.images.card

    return (
        <div className="flex flex-col gap-5 p-4 md:p-6">
            <Link to="/stats" className="flex w-fit items-center gap-1 text-xs text-muted hover:text-accent">
                <FiChevronLeft aria-hidden /> All stats
            </Link>

            <header
                className="relative overflow-hidden rounded-2xl border border-border"
                style={{
                    background: `linear-gradient(135deg, hsl(${look.hue} 60% 35%), hsl(${(look.hue + 40) % 360} 50% 18%))`,
                    borderTop: `3px solid ${look.accent}`,
                }}
            >
                {art && <img src={art} alt="" className="absolute inset-0 size-full object-cover opacity-50" />}
                <div className="absolute inset-0 bg-gradient-to-r from-black/85 via-black/55 to-black/10" />
                <div className="relative flex flex-wrap items-end justify-between gap-4 px-5 py-7 md:px-8">
                    <div>
                        <p className="text-[11px] font-semibold uppercase tracking-[0.18em] text-white/70">Stats</p>
                        <h1 className="text-3xl font-extrabold text-white" style={gameHeadingStyle(look)}>
                            {app?.name ?? `Game ${appId}`}
                        </h1>
                        {game && (
                            <p className="text-sm text-white/75">
                                {game.daysPlayed} days played · last {lastPlayedLabel(game.lastPlayed)}
                            </p>
                        )}
                    </div>
                    <div className="flex items-center gap-3">
                        {game?.achievements && game.achievements.total > 0 && (
                            <div className="flex items-center gap-2 rounded-xl bg-black/55 px-3 py-2 text-white backdrop-blur">
                                <ProgressRing value={completion(game.achievements)} accent={look.accent} size={44} label="Achievements" />
                                <span className="text-xs leading-tight">
                                    <span className="block text-white/60">Achievements</span>
                                    <b className="tabular-nums">
                                        {game.achievements.unlocked} / {game.achievements.total}
                                    </b>
                                </span>
                            </div>
                        )}
                        <Link to={`/store/${appId}`} className="flex items-center gap-1.5 rounded-xl bg-black/55 px-3 py-2 text-sm font-semibold text-white backdrop-blur hover:bg-black/70">
                            <FiShoppingBag aria-hidden /> Store
                        </Link>
                    </div>
                </div>
            </header>

            <Tabs
                value={tab}
                onChange={setTab}
                tabs={[
                    { value: 'overview', label: 'Overview', icon: <FiActivity aria-hidden /> },
                    { value: 'achievements', label: 'Achievements', icon: <FiAward aria-hidden /> },
                    { value: 'leaderboards', label: 'Leaderboards', icon: <FiTrendingUp aria-hidden /> },
                ]}
            />

            {tab === 'overview' && (
                <div className="grid grid-cols-1 gap-4 lg:grid-cols-5">
                    <Panel title="Last 30 days" icon={<FiActivity />} className="lg:col-span-3">
                        {game ? (
                            <ActivityBars series={game.recent} since={overview.data?.since ?? ''} accent={look.accent} unit="day" />
                        ) : overview.isPending && on ? (
                            <RowSkeleton rows={2} />
                        ) : (
                            <p className="text-sm text-muted">No play recorded for this game yet.</p>
                        )}
                    </Panel>
                    <Panel title="Your stats" icon={<FiBarChart2 />} className="lg:col-span-2">
                        {!on && <p className="text-sm text-muted">Sign in to see your stats.</p>}
                        {mine.isPending && on && <RowSkeleton rows={3} />}
                        {mine.isError && <ErrorNote error={mine.error} />}
                        {mine.data?.stats.length === 0 && <p className="text-sm text-muted">This game has not filed any stats for you yet.</p>}
                        <dl className="grid grid-cols-2 gap-2" data-testid="stats-mine">
                            {mine.data?.stats.map((s) => (
                                <div key={s.key} className="rounded-xl bg-surface-2 px-3 py-2">
                                    <dt className="truncate text-[11px] text-muted">{s.name}</dt>
                                    <dd className="text-lg font-extrabold tabular-nums">{formatStat(s.value, s.decimals, s.unit)}</dd>
                                </div>
                            ))}
                        </dl>
                    </Panel>
                </div>
            )}

            {tab === 'achievements' && <Achievements q={ach} look={look} on={on} />}
            {tab === 'leaderboards' && valid && (
                <div className="rounded-2xl border border-border bg-surface p-4">
                    <Leaderboards appId={appId} />
                </div>
            )}
        </div>
    )
}

function Achievements({
    q,
    look,
    on,
}: {
    q: { data?: AchievementsResponseT; isPending: boolean; isError: boolean; error: unknown }
    look: GameLook
    on: boolean
}) {
    if (!on) return <Empty icon={<FiLock />} title="Sign in to see your achievements" />
    if (q.isPending) return <RowSkeleton rows={4} />
    if (q.isError) return <ErrorNote error={q.error} />
    if (!q.data) return null

    const { unlocked, locked, points } = q.data

    if (unlocked.length + locked.length === 0)
        return <Empty icon={<FiAward />} title="This game has no achievements yet" />

    return (
        <div className="flex flex-col gap-4" data-testid="stats-achievements">
            <p className="text-sm text-muted">
                <b className="text-foreground">{unlocked.length}</b> of {unlocked.length + locked.length} unlocked ·{' '}
                <b className="text-foreground">{points.toLocaleString('en-US')}</b> points
            </p>
            <ul className="grid grid-cols-1 gap-2 md:grid-cols-2">
                {unlocked.map((a) => (
                    <AchRow
                        key={a.key}
                        look={look}
                        done
                        name={a.name}
                        description={a.description}
                        points={a.points}
                        icon={a.icon}
                        meta={`Unlocked ${new Date(a.unlockedAt).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' })}`}
                    />
                ))}
                {locked.map((a) => (
                    <AchRow
                        key={a.key}
                        look={look}
                        done={false}
                        name={a.secret ? 'Secret achievement' : (a.name ?? a.key)}
                        description={a.secret ? 'Keep playing to find out.' : a.description}
                        points={a.points}
                        icon={a.secret ? null : a.icon}
                        meta={`${a.unlockCnt.toLocaleString('en-US')} ${a.unlockCnt === 1 ? 'player has' : 'players have'} it`}
                    />
                ))}
            </ul>
        </div>
    )
}

function AchRow({
    look,
    done,
    name,
    description,
    points,
    icon,
    meta,
}: {
    look: GameLook
    done: boolean
    name: string
    description: string | null
    points: number
    icon: string | null
    meta: string
}) {
    return (
        <li
            className={`flex items-center gap-3 rounded-xl border p-3 ${done ? 'border-border bg-surface' : 'border-dashed border-border bg-transparent'}`}
        >
            <div
                className="flex size-11 shrink-0 items-center justify-center overflow-hidden rounded-xl text-lg"
                style={done ? { background: look.accent, color: '#fff' } : undefined}
            >
                {icon ? (
                    <img src={icon} alt="" className={`size-full object-cover ${done ? '' : 'opacity-40 grayscale'}`} />
                ) : done ? (
                    <FiAward aria-hidden />
                ) : (
                    <span className="flex size-full items-center justify-center bg-surface-2 text-muted">
                        <FiLock aria-hidden />
                    </span>
                )}
            </div>
            <div className="min-w-0 flex-1">
                <p className={`truncate font-semibold ${done ? '' : 'text-muted'}`}>{name}</p>
                {description && <p className="line-clamp-1 text-xs text-muted">{description}</p>}
                <p className="text-[11px] text-muted">{meta}</p>
            </div>
            <span className={`rounded-md px-2 py-0.5 text-xs font-bold tabular-nums ${done ? 'bg-accent/15 text-accent' : 'bg-surface-2 text-muted'}`}>
                {points}
            </span>
        </li>
    )
}

function Tile({ icon, label, value, hint, unit }: { icon: ReactNode; label: string; value: number; hint: string; unit?: string }) {
    return (
        <li className="gs-rise flex flex-col gap-1 rounded-2xl border border-border bg-surface p-4">
            <span className="flex items-center gap-1.5 text-xs text-muted">
                <span className="text-accent">{icon}</span>
                {label}
            </span>
            <span className="text-3xl font-extrabold tabular-nums">
                {value.toLocaleString('en-US')}
                {unit && <span className="ml-1 text-sm font-semibold text-muted">{unit}</span>}
            </span>
            <span className="text-[11px] text-muted">{hint}</span>
        </li>
    )
}

function Panel({ title, icon, className = '', children }: { title: string; icon: ReactNode; className?: string; children: ReactNode }) {
    return (
        <section className={`flex flex-col gap-4 rounded-2xl border border-border bg-surface p-4 md:p-5 ${className}`}>
            <h2 className="flex items-center gap-1.5 text-sm font-semibold">
                <span className="text-accent">{icon}</span>
                {title}
            </h2>
            {children}
        </section>
    )
}

function Mini({ label, value }: { label: string; value: string }) {
    return (
        <div className="rounded-lg bg-surface-2 px-2 py-1.5">
            <dt className="text-[10px] uppercase tracking-wide text-muted">{label}</dt>
            <dd className="font-bold tabular-nums">{value}</dd>
        </div>
    )
}

function OverviewSkeleton() {
    return (
        <div className="flex flex-col gap-4" aria-busy="true">
            <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
                {[0, 1, 2, 3].map((i) => (
                    <div key={i} className="h-28 animate-pulse rounded-2xl bg-surface" />
                ))}
            </div>
            <div className="h-48 animate-pulse rounded-2xl bg-surface" />
        </div>
    )
}

/** The /me strip: four figures and a link to the Stats screen. */
export function StatsStrip() {
    const { status } = useAuth()
    const q = useQuery({ queryKey: ['stats', 'overview'], queryFn: () => api.meStats(), enabled: status === 'signedIn' })

    if (!q.data) return null

    const d = q.data

    return (
        <section className="flex flex-col gap-3 rounded-2xl border border-border bg-surface p-4" aria-label="Stats summary" data-testid="me-stats">
            <div className="flex items-center justify-between">
                <h2 className="flex items-center gap-1.5 text-sm font-semibold">
                    <FiBarChart2 aria-hidden className="text-accent" /> Your stats
                </h2>
                <div className="flex gap-3 text-xs">
                    <Link to="/store" className="text-muted hover:text-accent">
                        Store
                    </Link>
                    <Link to="/stats" className="font-semibold text-accent">
                        All stats
                    </Link>
                </div>
            </div>
            <dl className="grid grid-cols-2 gap-2 sm:grid-cols-4">
                <Mini label="Days played" value={d.totals.daysPlayed.toLocaleString('en-US')} />
                <Mini label="Streak" value={`${currentStreak(d.recent)} days`} />
                <Mini label="Achievements" value={d.totals.achievements.toLocaleString('en-US')} />
                <Mini label="Games" value={String(d.totals.games)} />
            </dl>
            <Sparkline series={d.recent} accent="var(--accent)" height={32} label="Games played per day, last 30 days" />
        </section>
    )
}
