import {
    FiBox,
    FiDownload,
    FiDownloadCloud,
    FiFileText,
    FiFolder,
    FiGlobe,
    FiGrid,
    FiHardDrive,
    FiCompass,
    FiLayers,
    FiMoreHorizontal,
    FiPackage,
    FiPlay,
    FiServer,
    FiSettings,
    FiTerminal,
    FiUser,
    FiUsers,
    FiBookOpen,
    FiAward,
    FiSmile,
} from 'react-icons/fi'
import type { IconType } from 'react-icons'

/**
 * The primary navigation as data.
 *
 * One tree feeds three consumers: the rail, the bottom bar, and the Discover
 * and Library landing pages. The landing pages exist because the bottom bar
 * cannot nest, so they must be able to reach every child, and generating their
 * cards from the same tree is what keeps "the rail has it" and "the phone can
 * reach it" from drifting apart.
 */
export type NavNode = {
    /** Stable key, also the localStorage key for a collapsible node. */
    id: string
    to: string
    label: string
    icon: IconType
    /** One line, shown on the landing page card. */
    description: string
    /** Also highlight for these path prefixes. */
    match?: string[]
    /**
     * Show the count of active downloads on this node.
     *
     * Only Downloads carries it, and it is the reason downloads are in the
     * rail at all: a queue nobody can see from another screen is one people
     * check by opening it, which is the thing a badge exists to save.
     */
    badge?: 'downloads'
    /** Children. Their parent is shown softly active while one of them is. */
    items?: NavNode[]
    /**
     * Whether the children fold away behind a chevron.
     *
     * Servers has a child (RCON) that is always shown, because one line is not
     * worth a toggle; Discover, Library and More hold enough to need one.
     */
    collapsible?: boolean
    /**
     * A grouping with no screen of its own. Its header is only a toggle and
     * the landing page lists its children inline instead of linking to it.
     */
    virtual?: boolean
}

export const PLAY: NavNode = {
    id: 'play',
    to: '/play',
    label: 'Play',
    icon: FiPlay,
    description: 'Jump into a game.',
}

export const GAMES: NavNode = {
    id: 'games',
    to: '/games',
    label: 'Games',
    icon: FiGrid,
    description: 'GameSpice games: play, join servers, set up your avatar.',
}

/** Slot for the avatar studio (gs-app-avatar-1 replaces the route's element). */
export const AVATAR: NavNode = {
    id: 'avatar',
    to: '/avatar',
    label: 'Avatar',
    icon: FiSmile,
    description: 'Your figure and outfits.',
}

export const GAMESPICE_ACCOUNT: NavNode = {
    id: 'gamespice-account',
    to: '/me',
    label: 'My GameSpice',
    icon: FiAward,
    description: 'Points and saved outfits.',
}

export const DISCOVER: NavNode = {
    id: 'discover',
    to: '/discover',
    label: 'Discover',
    icon: FiCompass,
    description: 'Everything GameSpice hosts, from games to people.',
    collapsible: true,
    items: [
        {
            // The front door's old tenant: a flat catalogue across every game,
            // which the site cannot offer because it makes you pick a game first.
            id: 'apps',
            to: '/apps',
            label: 'Apps',
            icon: FiGrid,
            description: 'Every game, engine and platform, and what you can do with each.',
        },
        {
            id: 'mods',
            to: '/browse/mod',
            label: 'Mods',
            icon: FiPackage,
            description: 'Gameplay changes, maps and total conversions.',
            match: ['/view/mod'],
        },
        {
            id: 'assets',
            to: '/browse/asset',
            label: 'Assets',
            icon: FiBox,
            description: 'Models, textures, sounds and other building blocks.',
            match: ['/view/asset'],
        },
        {
            id: 'servers',
            to: '/browse/server',
            label: 'Servers',
            icon: FiServer,
            description: 'Live servers with real latency measured from this device.',
            match: ['/view/server', '/view/serverMap', '/browse/serverMap'],
            items: [
                {
                    // Under Servers because RCON is something you do to a server
                    // you run; it used to be labelled Console, which now belongs
                    // to the TMC command-line tool.
                    id: 'rcon',
                    to: '/rcon',
                    label: 'RCON',
                    icon: FiTerminal,
                    description: 'Run commands on servers you administer.',
                },
            ],
        },
        {
            id: 'more',
            to: '/discover',
            label: 'More',
            icon: FiMoreHorizontal,
            description: 'Communities, articles, collections and people.',
            collapsible: true,
            virtual: true,
            items: [
                {
                    id: 'communities',
                    to: '/browse/community',
                    label: 'Communities',
                    icon: FiGlobe,
                    description: 'Groups built around a game or a project.',
                    match: ['/view/community'],
                },
                {
                    id: 'articles',
                    to: '/browse/article',
                    label: 'Articles',
                    icon: FiFileText,
                    description: 'Guides, news and write-ups.',
                    match: ['/view/article'],
                },
                {
                    id: 'collections',
                    to: '/browse/collection',
                    label: 'Collections',
                    icon: FiFolder,
                    description: 'Hand-picked sets of mods and assets.',
                    match: ['/view/collection'],
                },
                {
                    id: 'users',
                    to: '/browse/user',
                    label: 'Users',
                    icon: FiUser,
                    description: 'Authors and members.',
                    match: ['/view/user'],
                },
            ],
        },
    ],
}

export const LIBRARY: NavNode = {
    id: 'library',
    to: '/library',
    label: 'Library',
    icon: FiDownloadCloud,
    description: 'What is on this device: installs, sandboxes and downloads.',
    match: ['/installs'],
    collapsible: true,
    items: [
        {
            // The mod manager proper: where somebody SPENDS time reordering,
            // deploying and checking conflicts, so it is its own entry.
            id: 'sandboxes',
            to: '/sandboxes',
            label: 'Sandboxes',
            icon: FiLayers,
            description: 'Mod profiles you can deploy, share and switch between.',
        },
        {
            id: 'workshop',
            to: '/workshop',
            label: 'Workshop',
            icon: FiPackage,
            description: 'Packs you subscribed to, across every GameSpice game.',
        },
        {
            id: 'downloads',
            to: '/downloads',
            label: 'Downloads',
            icon: FiDownload,
            description: 'The queue, its speed and what finished.',
            badge: 'downloads',
        },
        {
            id: 'stores',
            to: '/stores',
            label: 'Stores',
            icon: FiHardDrive,
            description: 'Where downloads are kept, temporary or permanent.',
        },
    ],
}

export const FRIENDS: NavNode = {
    // Friends, their presence and the party you are in: the same routes a Dot
    // game's own friends panel speaks, so the two agree.
    id: 'friends',
    to: '/friends',
    label: 'Friends',
    icon: FiUsers,
    description: 'Who is online and the party you are in.',
}

export const CONSOLE: NavNode = {
    id: 'console',
    to: '/console',
    label: 'Console',
    icon: FiBookOpen,
    description: 'The TMC command-line tool.',
}

export const SETTINGS: NavNode = {
    id: 'settings',
    to: '/settings',
    label: 'Settings',
    icon: FiSettings,
    description: 'The app, your account, games and plugins.',
}

/** Top to bottom, as the rail draws them. */
export const RAIL: NavNode[] = [
    PLAY,
    GAMES,
    DISCOVER,
    LIBRARY,
    AVATAR,
    GAMESPICE_ACCOUNT,
    FRIENDS,
    CONSOLE,
    SETTINGS,
]

/**
 * The bottom bar: at most five, because past that the labels stop being
 * legible and the fix is fewer items rather than smaller text. Discover and
 * Library go to their landing pages, which link to every child.
 */
export const COMPACT: NavNode[] = [PLAY, DISCOVER, LIBRARY, FRIENDS, SETTINGS]

/** Whether the path is this node's own screen (not a child's). */
export function isOwn(node: NavNode, pathname: string): boolean {
    if (node.virtual) return false
    if (pathname === node.to || pathname.startsWith(`${node.to}/`)) return true

    return (node.match ?? []).some((prefix) => pathname.startsWith(prefix))
}

/** Whether the path is inside any descendant of the node. */
export function isWithin(node: NavNode, pathname: string): boolean {
    return (node.items ?? []).some(
        (child) => isOwn(child, pathname) || isWithin(child, pathname)
    )
}

/** Own screen or a descendant's: what a bottom-bar item highlights for. */
export function isActive(node: NavNode, pathname: string): boolean {
    return isOwn(node, pathname) || isWithin(node, pathname)
}

/**
 * Every leaf below a node, in rail order, with virtual groups flattened away.
 * This is what a landing page draws a card for.
 */
export function leaves(node: NavNode): NavNode[] {
    return (node.items ?? []).flatMap((child) => [
        ...(child.virtual ? [] : [child]),
        ...leaves(child),
    ])
}
