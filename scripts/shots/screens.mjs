/**
 * The screen list: what capture.mjs shoots and the order compose.mjs tells the
 * story in. Add a screen here (route, copy, what to wait for) and both steps
 * pick it up. `enabled: false` keeps a slot in the tour without shooting it,
 * for screens that are not on this branch yet (Store, Stats).
 *
 * slot     file stem: <slot>-<light|dark>.png in out/raw, and the website's
 *          public/app/shots/<slot>-<scheme>-<800|1600>.webp
 * route    hash route in the app
 * wait     a selector that is visible once the screen has its data
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
        slot: 'hub', route: '/play', wait: 'main', tour: false,
        title: 'Home', caption: 'Everything you play and build, in one place.',
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
    // On branch gs-app-store-stats; enable once it is on main.
    { slot: 'store', route: '/store', wait: 'main', enabled: false, title: 'Store', caption: 'Buy and gift games and packs.' },
    { slot: 'stats', route: '/stats', wait: 'main', enabled: false, title: 'Stats', caption: 'Your playtime and progress across games.' },
]

/** The narrative order of the presentation (slots; missing or disabled are skipped). */
export const tour = ['play', 'mods', 'modpage', 'workshop', 'sandboxes', 'servers', 'downloads', 'library', 'avatar', 'friends', 'store', 'stats', 'settings']

export const themes = ['light', 'dark']
export const SIZE = { width: 1600, height: 1000 }
