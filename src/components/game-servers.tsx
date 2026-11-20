import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'

import type { BrowseSortT } from '~/lib/api/contract'
import { api } from '~/lib/api/client'
import { messageOf } from '~/lib/ipc'
import ServerTable from '~/components/server-table'

/**
 * Live servers of one GameSpice game, inline.
 *
 * The Browse server table, narrowed to the given apps and to servers seen online. The
 * rows measure latency from this device themselves (`useLiveServer`), so the
 * ping column is the player's own, not the site's. Sorting is server-side, on
 * the same keys as Browse; "All servers" hands over to Browse for the full
 * filter panel.
 */
export default function GameServers({ appIds, limit = 12 }: { appIds: number[]; limit?: number }) {
    const [sort, setSort] = useState<BrowseSortT>('players')
    const [dir, setDir] = useState<'asc' | 'desc'>('desc')

    const list = useQuery({
        queryKey: ['gamespice', 'servers', appIds.join(','), sort, dir, limit],
        queryFn: () =>
            api.browse({
                kind: 'server',
                apps: appIds,
                onlineOnly: true,
                sort,
                sortDir: dir,
                limit,
            }),
        enabled: appIds.length > 0,
        staleTime: 15_000,
        refetchInterval: 30_000,
    })

    const items = list.data?.items ?? []

    return (
        <section className="flex flex-col gap-2" aria-label="Live servers">
            <div className="flex items-center justify-between">
                <h2 className="text-sm font-semibold">Live servers</h2>
                <Link to={`/browse/server?app=${appIds.join(',')}`} className="text-xs text-muted underline">
                    All servers
                </Link>
            </div>
            {list.isPending && appIds.length > 0 && <p className="text-sm text-muted">Loading servers…</p>}
            {list.isError && (
                <p role="alert" className="text-sm text-danger">
                    {messageOf(list.error)}
                </p>
            )}
            {list.isSuccess && items.length === 0 && (
                <p className="text-sm text-muted">No servers are online right now.</p>
            )}
            {items.length > 0 && (
                <ServerTable
                    items={items}
                    sort={sort}
                    sortDir={dir}
                    onSort={(key, d) => {
                        setSort(key)
                        setDir(d)
                    }}
                />
            )}
        </section>
    )
}
