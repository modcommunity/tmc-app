/** A pulsing placeholder block; size it with className. */
export function Skeleton({ className = '' }: { className?: string }) {
    return (
        <div
            aria-hidden
            className={`animate-pulse rounded-lg bg-surface-secondary ${className}`}
        />
    )
}

/** A grid of card-shaped placeholders for a list that is still loading. */
export function SkeletonGrid({
    count = 6,
    className = '',
}: {
    count?: number
    className?: string
}) {
    return (
        <div
            role="status"
            aria-label="Loading"
            className={`grid gap-3 sm:grid-cols-2 xl:grid-cols-3 ${className}`}
        >
            {Array.from({ length: count }, (_, i) => (
                <div
                    key={i}
                    className="flex gap-3 rounded-xl border border-border bg-surface p-3"
                >
                    <Skeleton className="size-14 shrink-0" />
                    <div className="flex flex-1 flex-col gap-2 py-1">
                        <Skeleton className="h-3.5 w-2/3" />
                        <Skeleton className="h-3 w-full" />
                        <Skeleton className="h-3 w-1/2" />
                    </div>
                </div>
            ))}
        </div>
    )
}
