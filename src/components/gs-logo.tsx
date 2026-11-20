/**
 * The GameSpice mark (public/brand/gamespice-mark.svg: a potion bottle full of
 * spice, labelled GS) inline, so the header draws it with the page and no
 * extra request. Gradient and clip ids carry `id` so two marks on one page do
 * not share them.
 */
export function GsLogo({
    size = 32,
    id = 'logo',
    className,
}: {
    size?: number
    id?: string
    className?: string
}) {
    return (
        <svg
            viewBox="0 0 512 512"
            width={size}
            height={size}
            aria-hidden="true"
            className={className}
        >
            <defs>
                <linearGradient
                    id={`gs-${id}-potion`}
                    x1="0"
                    y1="0"
                    x2="0"
                    y2="1"
                >
                    <stop offset="0" stopColor="#ffb02e" />
                    <stop offset="0.5" stopColor="#f2542d" />
                    <stop offset="1" stopColor="#b8193c" />
                </linearGradient>
                <clipPath id={`gs-${id}-body`}>
                    <path d="M222 92V178C150 206 112 264 112 332A144 144 0 0 0 400 332C400 264 362 206 290 178V92Z" />
                </clipPath>
            </defs>
            <g transform="translate(0 4)">
                <g data-sparks="">
                    <path
                        fill="#ffc83d"
                        d="M410 96l8 20 20 8-20 8-8 20-8-20-20-8 20-8z"
                    />
                    <path
                        fill="#ff7a2d"
                        d="M96 150l6 15 15 6-15 6-6 15-6-15-15-6 15-6z"
                    />
                    <circle cx="370" cy="40" r="8" fill="#ffc83d" />
                    <circle cx="140" cy="76" r="7" fill="#ffc83d" />
                    <circle cx="452" cy="196" r="6" fill="#ff7a2d" />
                    <circle cx="60" cy="240" r="6" fill="#ffc83d" />
                </g>
                <g data-bottle="">
                    <path
                        d="M222 92V178C150 206 112 264 112 332A144 144 0 0 0 400 332C400 264 362 206 290 178V92Z"
                        fill="none"
                        stroke="#2b1410"
                        strokeWidth="26"
                        strokeLinejoin="round"
                    />
                    <path
                        d="M222 92V178C150 206 112 264 112 332A144 144 0 0 0 400 332C400 264 362 206 290 178V92Z"
                        fill="#e9f5ff"
                        fillOpacity="0.92"
                    />
                    <path
                        d="M90 270C130 250 160 288 200 272C240 256 270 288 310 272C350 256 380 282 420 268V480H90Z"
                        fill={`url(#gs-${id}-potion)`}
                        clipPath={`url(#gs-${id}-body)`}
                    />
                    <circle
                        cx="352"
                        cy="420"
                        r="11"
                        fill="#fff"
                        fillOpacity="0.55"
                    />
                    <circle
                        cx="318"
                        cy="448"
                        r="7"
                        fill="#fff"
                        fillOpacity="0.55"
                    />
                    <circle
                        cx="146"
                        cy="400"
                        r="8"
                        fill="#fff"
                        fillOpacity="0.55"
                    />
                    <path
                        d="M222 92V178C150 206 112 264 112 332A144 144 0 0 0 400 332C400 264 362 206 290 178V92Z"
                        fill="none"
                        stroke="#fff6e6"
                        strokeWidth="10"
                        strokeLinejoin="round"
                    />
                    <path
                        d="M150 304C156 272 176 248 204 234"
                        fill="none"
                        stroke="#fff"
                        strokeOpacity="0.8"
                        strokeWidth="11"
                        strokeLinecap="round"
                    />
                    <rect
                        x="174"
                        y="288"
                        width="164"
                        height="94"
                        rx="16"
                        fill="#fff6e6"
                        stroke="#2b1410"
                        strokeWidth="8"
                    />
                    <path
                        d="M247 321A22 22 0 1 0 248 337H228M305 319C299 308 270 307 268 321C266 335 305 331 303 346C301 360 272 359 266 348"
                        fill="none"
                        stroke="#7a1f14"
                        strokeWidth="13"
                        strokeLinecap="round"
                        strokeLinejoin="round"
                    />
                    <rect
                        x="204"
                        y="34"
                        width="104"
                        height="60"
                        rx="16"
                        fill="#f0a62b"
                        stroke="#2b1410"
                        strokeWidth="12"
                    />
                    <path
                        d="M222 52h68"
                        stroke="#ffd37a"
                        strokeWidth="8"
                        strokeLinecap="round"
                    />
                </g>
            </g>
        </svg>
    )
}
