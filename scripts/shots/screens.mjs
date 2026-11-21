/**
 * The screen list: what capture.mjs shoots and the order compose.mjs tells the
 * story in. Add a screen here (route, copy, what to wait for) and both steps
 * pick it up. `enabled: false` keeps a slot in the tour without shooting it,
 * for a screen that is not on this branch yet.
 *
 * slot     file stem: <slot>-<light|dark>.png in out/raw, and the website's
 *          public/app/shots/<slot>-<scheme>-<800|1600>.webp
 * route    hash route in the app
 * wait     a selector that is visible once the screen has its data
 * size     optional { width, height } viewport instead of SIZE (keep 16:10)
 * gpu      needs real WebGL (the 3D avatar): capture on a GPU box
 * act      optional: async (page) => {} run after load, before the shot
 * callouts optional: [{ at: locator, label }]; capture records where `at` is, compose draws the label there
 */
export const screens = [
    {
        slot: 'play', route: '/play', wait: 'main',
        title: 'Play', caption: 'Your games, sandboxes and friends online, one click from launch.',
    },
    {
        // The website's hero; a different screen from `play` so the /app page
        // does not show the same picture twice.
        slot: 'hub', route: '/games', wait: 'main', tour: false,
        title: 'Games', caption: 'Every GameSpice game, its servers and your avatar, one click away.',
    },
    {
        slot: 'mods', route: '/browse/mod', wait: 'main',
        title: 'Discover mods', caption: 'Browse maps, mods and packs from every community.',
    },
    {
        slot: 'modpage', route: '/view/mod/2', wait: 'main',
        title: 'Install in one click', caption: 'Every mod page shows what it changes and installs into the right sandbox.',
        callouts: [{ at: 'text=Install into a sandbox >> xpath=ancestor::*[.//button][1]', label: 'Pick a sandbox, press Install' }],
    },
    {
        slot: 'workshop', route: '/workshop', wait: 'main',
        title: 'Workshop', caption: 'Publish your own maps and mods, with releases and changelogs.',
    },
    {
        slot: 'sandboxes', route: '/sandboxes', wait: 'main',
        title: 'Sandboxes', caption: 'Keep mod sets apart: one game, as many setups as you like.',
        act: async (page) => { await page.getByText('Party night', { exact: true }).first().click() },
    },
    {
        slot: 'servers', route: '/browse/server', wait: 'main',
        title: 'Servers with live latency', caption: 'Real game queries and a running ping trace for every server.',
        act: async (page) => { await page.getByText('Ember Ridge | Casual 24/7').first().click() },
        callouts: [{ at: '[role="img"][aria-label^="Latency over"]', label: 'Ping measured from your machine' }],
    },
    {
        // The real CORE-SS :: US Coop (Left 4 Dead 2, prod item 1552); the graph and roster are seeded (seed.ts).
        // 1440x900 keeps the 16:10 shape compose.mjs assumes.
        slot: 'monitor', route: '/monitor/addr?host=74.91.124.246&port=27015&app=1&name=CORE-SS%20%3A%3A%20US%20Coop',
        wait: 'text=Players (20)', size: { width: 1440, height: 900 },
        title: 'Server monitor', caption: 'A live latency graph, the player list and RCON in one window.',
    },
    {
        slot: 'downloads', route: '/downloads', wait: 'main',
        title: 'Downloads', caption: 'A queue you control: priorities, pauses and speed limits.',
    },
    {
        slot: 'library', route: '/library', wait: 'main',
        title: 'Library', caption: 'Installed mods stay up to date on their own.',
    },
    {
        slot: 'avatar', route: '/avatar', wait: 'canvas', gpu: true,
        title: 'Avatar studio', caption: 'Build your look once and wear it in every game.',
    },
    {
        slot: 'friends', route: '/friends', wait: 'main',
        title: 'Friends', caption: 'See who is online and what they are playing.',
    },
    {
        slot: 'settings', route: '/settings', wait: 'main',
        title: 'Settings', caption: 'Themes, folders, plugins and logging, all in one place.',
    },
    { slot: 'store', route: '/store', wait: 'main', title: 'Store', caption: 'Every game\'s store: outfits, packs and points in one place.' },
    { slot: 'stats', route: '/stats', wait: 'main', title: 'Stats', caption: 'Your playtime and progress across games.' },
]

/** The narrative order of the presentation (slots; missing or disabled are skipped). */
export const tour = ['play', 'mods', 'modpage', 'workshop', 'sandboxes', 'servers', 'monitor', 'downloads', 'library', 'avatar', 'friends', 'store', 'stats', 'settings']

export const themes = ['light', 'dark']
export const SIZE = { width: 1600, height: 1000 }
