#!/usr/bin/env python3
"""Write a game's rules under `plugins/app/<slug>/` from one row of a table.

    scripts/gen-app-rules.py            # every game that has no folder yet
    scripts/gen-app-rules.py skyrim-se  # just this one
    scripts/gen-app-rules.py --force …  # overwrite what is there

WHY THIS EXISTS, AND WHAT IT IS NOT
-----------------------------------
Adding a game is four small JSON files that are ninety percent the same as the
last game's, and the ten percent that differs is the part worth reviewing. A
generator makes the boilerplate uniform so a reader's attention lands on the
FAMILY and the paths — the two things that are actually a decision.

It is not a build step. The files it writes are committed, are the source of
truth, and are meant to be hand-edited afterwards; `core/build.rs` compiles the
directory, never this script. It refuses to overwrite an existing folder without
`--force` for exactly that reason.

WHERE THE FACTS COME FROM
-------------------------
`provenance` on every row. `vortex` means the mod path, Steam id and marker were
read out of Nexus Mods' own game extension in
`external-study/modding/mod-managers/Vortex/extensions/games/`; `thunderstore`
means r2modman's `ecosystem.json`. `known` means neither had it and it is
written from the game's own modding documentation — those are the rows to check
first when something lands in the wrong folder.

THE SLUG IS THE WEBSITE'S, NOT OURS
-----------------------------------
The directory name has to equal the game's URL slug on moddingcommunity.com. A
rule under a slug the site does not list loads fine and can never be applied:
the folder scan finds the game, offers it, and `detect_apply` has no app id to
key a game directory on. The slugs below are the obvious spelling for each game
and are a guess until the catalogue has the row.
"""

import json
import os
import sys

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..')

DOWNLOAD = {
    "action": "download",
    "url": "{fileUrl}",
    "to": {"root": "pluginData", "path": "staging/{itemId}/{fileName}"},
}

STAGED = {"root": "pluginData", "path": "staging/{itemId}/{fileName}"}
SCRATCH = {"root": "pluginData", "path": "", "write": True}


def grant(path, write=True):
    return {"root": "gameDir", "path": path, "write": write}


# --------------------------------------------------------------- the table
#
# family — decides the shape of the rules:
#   bethesda   loose files and archives into Data/. The archive IS the mod
#              layout, so it is extracted flat and the executor's journal is
#              what makes uninstalling precise.
#   folder     one directory per mod under a mods folder, which is how the
#              loader enumerates them.
#   bepinex    BepInEx/plugins, loose .dll or packaged .zip.
#   modloader  GTA's modloader/ plus loose .asi/.cleo in the game root.
#   userdata   the mod folder is under Documents, not under the install.

GAMES = [
    # ---------------------------------------------------------- Creation Engine
    dict(slug="skyrim-se", name="The Elder Scrolls V: Skyrim Special Edition",
         family="bethesda", data="Data", steam=["489830"], extender="SKSE/Plugins",
         markers=["SkyrimSE.exe"], provenance="vortex",
         note="Special Edition and the original are different games with incompatible plugin formats. A mod built for one will not load in the other, and the app cannot tell them apart from the file alone — it is the sandbox's game folder that decides."),
    dict(slug="skyrim", name="The Elder Scrolls V: Skyrim",
         family="bethesda", data="Data", steam=["72850"], extender="SKSE/Plugins",
         markers=["TESV.exe"], provenance="vortex"),
    dict(slug="skyrim-vr", name="The Elder Scrolls V: Skyrim VR",
         family="bethesda", data="Data", steam=["611670"], extender="SKSE/Plugins",
         markers=["SkyrimVR.exe"], provenance="vortex"),
    dict(slug="fallout-4", name="Fallout 4",
         family="bethesda", data="Data", steam=["377160"], extender="F4SE/Plugins",
         markers=["Fallout4.exe"], provenance="vortex"),
    dict(slug="fallout-new-vegas", name="Fallout: New Vegas",
         family="bethesda", data="Data", steam=["22380", "22490"], extender="NVSE/Plugins",
         markers=["FalloutNV.exe"], provenance="known"),
    dict(slug="fallout-3", name="Fallout 3",
         family="bethesda", data="Data", steam=["22300", "22370"], extender="FOSE/Plugins",
         markers=["Fallout3.exe"], provenance="vortex"),
    dict(slug="oblivion", name="The Elder Scrolls IV: Oblivion",
         family="bethesda", data="Data", steam=["22330"], extender="OBSE/Plugins",
         markers=["Oblivion.exe"], provenance="vortex"),
    dict(slug="morrowind", name="The Elder Scrolls III: Morrowind",
         family="bethesda", data="Data Files", steam=["22320"], extender=None,
         markers=["Morrowind.exe"], provenance="vortex",
         note="Morrowind's folder is 'Data Files', with a space, and not 'Data'. Every later Bethesda game changed it."),
    dict(slug="starfield", name="Starfield",
         family="bethesda", data="Data", steam=["1716740"], extender="SFSE/Plugins",
         markers=["Starfield.exe"], provenance="vortex"),

    # ------------------------------------------------------------------ Sims
    dict(slug="sims-4", name="The Sims 4", family="userdata",
         mods="Mods", steam=["1222670"], markers=["Options.ini", "Mods"],
         userpaths={
             "windows": ["~/Documents/Electronic Arts/The Sims 4"],
             "macos": ["~/Documents/Electronic Arts/The Sims 4"],
         },
         extensions=["package", "ts4script"], provenance="vortex",
         note="Point this sandbox at Documents/Electronic Arts/The Sims 4 — the folder with Options.ini in it — and NOT at the Steam or EA install. The Sims 4 reads mods from your documents folder; the install folder holds no mods at all.",
         note2="Script mods (.ts4script) do nothing until they are enabled in the game: Game Options → Other → Enable Custom Content and Mods, and Script Mods Allowed. That is a setting inside Options.ini, which the config editor can open.",
         note3="The folder is localised — 'Die Sims 4' on a German install, 'Los Sims 4' on a Spanish one. Pick whichever exists on your machine; nothing here assumes the English name."),
    dict(slug="sims-3", name="The Sims 3", family="userdata",
         mods="Mods/Packages", steam=["47890"], markers=["Options.ini", "Mods"],
         userpaths={
             "windows": ["~/Documents/Electronic Arts/The Sims 3"],
             "macos": ["~/Documents/Electronic Arts/The Sims 3"],
         },
         extensions=["package"], provenance="vortex",
         note="Point this sandbox at Documents/Electronic Arts/The Sims 3, not at the install folder.",
         note2="The Sims 3 will not read Mods/Packages until a Resource.cfg exists beside it. The game does not create one; the framework install from ModTheSims does. Without it the folder fills up and nothing appears in game."),

    # ------------------------------------------------------------------- GTA
    dict(slug="gta-4", name="Grand Theft Auto IV", family="modloader",
         steam=["12210"], markers=["GTAIV.exe", "EFLC.exe"], loader_dir=None,
         provenance="known",
         note="GTA IV has no modloader equivalent, so script mods go loose in the game root and into scripts/. That is a wide grant and it is why this rule is worth reading before approving."),
    dict(slug="gta-san-andreas", name="Grand Theft Auto: San Andreas", family="modloader",
         steam=["12120"], markers=["gta_sa.exe"], loader_dir="modloader",
         provenance="known"),
    dict(slug="gta-vice-city", name="Grand Theft Auto: Vice City", family="modloader",
         steam=["12110"], markers=["gta-vc.exe"], loader_dir="modloader",
         provenance="known"),
    dict(slug="gta-3", name="Grand Theft Auto III", family="modloader",
         steam=["12100"], markers=["gta3.exe"], loader_dir="modloader",
         provenance="known"),

    # ------------------------------------------------------- one folder per mod
    dict(slug="stardew-valley", name="Stardew Valley", family="folder",
         mods="Mods", steam=["413150"], markers=["Stardew Valley.exe", "StardewValley"],
         provenance="known",
         note="Every mod here is a folder with a manifest.json in it, loaded by SMAPI. SMAPI itself is not a mod and is not installed by this app — without it the Mods folder is never read."),
    dict(slug="rimworld", name="RimWorld", family="folder",
         mods="Mods", steam=["294100"], markers=["RimWorldWin64.exe", "RimWorldLinux"],
         provenance="vortex",
         note="Load order is what RimWorld cares about most: a mod patching another has to come after it. The sandbox's order is the order they are written, so drag them into the order the mod pages tell you."),
    dict(slug="kerbal-space-program", name="Kerbal Space Program", family="folder",
         mods="GameData", steam=["220200"], markers=["KSP_x64.exe", "KSP.x86_64"],
         provenance="vortex"),
    dict(slug="mount-and-blade-2", name="Mount & Blade II: Bannerlord", family="folder",
         mods="Modules", steam=["261550"], markers=["bin/Win64_Shipping_Client/Bannerlord.exe"],
         provenance="vortex"),
    dict(slug="palworld", name="Palworld", family="folder",
         mods="Pal/Binaries/Win64/Mods", steam=["1623730"], markers=["Palworld.exe"],
         provenance="vortex"),
    dict(slug="baldurs-gate-3", name="Baldur's Gate 3", family="userdata",
         mods="Mods", steam=["1086940"], markers=["Mods", "PlayerProfiles"],
         userpaths={
             "windows": ["<localAppData>/Larian Studios/Baldur's Gate 3"],
             "macos": ["~/Documents/Larian Studios/Baldur's Gate 3"],
             "linux": ["~/.local/share/Larian Studios/Baldur's Gate 3"],
         },
         extensions=["pak"], provenance="known",
         note="Point this sandbox at the Larian Studios/Baldur's Gate 3 folder in your local app data, not at the install. That is where .pak mods go.",
         note2="A .pak in the folder is only half of it: modsettings.lsx has to list the mod as well, in the same order. Nothing here edits that file — use BG3 Mod Manager for the load order and this app for the files."),

    # -------------------------------------------------------------- The rest
    dict(slug="witcher-3", name="The Witcher 3: Wild Hunt", family="folder",
         mods="Mods", steam=["292030", "499450"], markers=["bin/x64/witcher3.exe"],
         provenance="vortex",
         note="Mods that add menu entries also need their .xml registering in bin/config/r4game/user_config_matrix/pc, and script mods usually need merging with the Script Merger. Neither is something a declarative rule can do — the files land correctly and the merge is still yours."),
    dict(slug="cyberpunk-2077", name="Cyberpunk 2077", family="folder",
         mods="archive/pc/mod", steam=["1091500"], markers=["bin/x64/Cyberpunk2077.exe"],
         provenance="vortex",
         note="archive/pc/mod is the folder for packed game content. Script and plugin mods go elsewhere — r6/scripts for redscript, bin/x64/plugins for RED4ext — and a mod that ships those needs its archive laying out from the game root instead."),
]


# ------------------------------------------------------------- rule builders

def bethesda(g):
    """Loose plugin files and archives, both into Data/."""
    data = g["data"]
    files = {}

    files["manage_mod.json"] = {
        "manifestVersion": 1,
        "label": f"{g['name']} plugin file",
        "description": f"A single plugin or archive file, straight into {data}. This is the shape a small mod ships in — one .esp, or one .bsa beside it.",
        "match": {"extensions": ["esp", "esm", "esl", "bsa", "ba2"]},
        "permissions": {"fs": [SCRATCH, grant(data)]},
        "manage": {
            "install": [
                DOWNLOAD,
                {"action": "mkdir", "path": {"root": "gameDir", "path": data}},
                {"action": "move", "from": STAGED,
                 "to": {"root": "gameDir", "path": f"{data}/{{fileName}}"}},
            ],
            "uninstall": [
                {"action": "remove", "path": {"root": "gameDir", "path": f"{data}/{{fileName}}"}},
            ],
        },
    }

    files["manage_mod.archive.json"] = {
        "manifestVersion": 1,
        "label": f"{g['name']} mod archive (.zip)",
        "description": (
            f"Unpacked FLAT into {data}, because that is the layout the archive already has — "
            "a Nexus mod's zip contains meshes/, textures/ and its .esp at the top level, and the "
            "game reads them from exactly those paths. There is no folder-per-mod to retreat to: "
            "the engine would simply not find anything inside one. What makes uninstalling precise "
            "instead is the executor's journal, which records every file the extract wrote."
        ),
        "match": {"extensions": ["zip", "7z", "rar"]},
        "permissions": {"fs": [SCRATCH, grant(data)]},
        "manage": {
            "install": [
                DOWNLOAD,
                {"action": "mkdir", "path": {"root": "gameDir", "path": data}},
                {"action": "extract", "from": STAGED,
                 "to": {"root": "gameDir", "path": data}},
            ],
            "uninstall": [
                {"action": "remove", "path": {"root": "pluginData", "path": "staging/{itemId}"}},
            ],
        },
    }

    if g.get("extender"):
        ext = g["extender"]
        files["manage_mod.extender.json"] = {
            "manifestVersion": 1,
            "label": f"{g['name']} script extender plugin (.dll)",
            "description": f"A native plugin for the script extender, into {data}/{ext}. The extender itself is not a mod and is not installed here.",
            "match": {"extensions": ["dll"]},
            "permissions": {"fs": [SCRATCH, grant(f"{data}/{ext}")]},
            "manage": {
                "install": [
                    DOWNLOAD,
                    {"action": "mkdir", "path": {"root": "gameDir", "path": f"{data}/{ext}"}},
                    {"action": "move", "from": STAGED,
                     "to": {"root": "gameDir", "path": f"{data}/{ext}/{{fileName}}"}},
                ],
                "uninstall": [
                    {"action": "remove",
                     "path": {"root": "gameDir", "path": f"{data}/{ext}/{{fileName}}"}},
                ],
            },
        }

    notes = [
        f"Everything lands in {data}. Load order is the sandbox's order, and for these games it is not cosmetic — a patch that loads before the mod it patches does nothing, and two mods editing one record are resolved by whichever loads last.",
        "This app does not write the plugin list the game reads (plugins.txt / loadorder.txt). It puts the files where they belong; enabling them in the launcher, or with a load-order tool, is still yours to do.",
        "Hard links are the default so several sandboxes can share one copy of a 4 GB texture pack. Copying is there for a staging folder on a different drive.",
    ]
    if g.get("note"):
        notes.insert(0, g["note"])

    return files, {
        "defaultStrategy": "hardlink",
        "supportedStrategies": ["hardlink", "symlink", "direct"],
        "modTargets": [
            {"type": "plugin", "relPath": data},
            {"type": "archive", "relPath": data},
        ] + ([{"type": "extender_plugin", "relPath": f"{data}/{g['extender']}"}] if g.get("extender") else []),
        "notes": notes,
    }, [
        {"id": "default", "label": "Modded", "description": "Plugins and archives in the game's data folder.", "environment": "client"},
        {"id": "clean", "label": "Clean", "description": "Nothing deployed — the vanilla game, for checking whether a bug is yours.",
         "environment": "client", "strategy": "direct"},
    ], [
        {"key": "launchArgs", "label": "Additional launch arguments", "type": "text",
         "description": "Passed verbatim. One argument per space; nothing here goes through a shell."},
    ]


def folder(g):
    """One directory per mod, which is how the loader enumerates them."""
    mods = g["mods"]

    files = {"manage_mod.json": {
        "manifestVersion": 1,
        "label": f"{g['name']} mod",
        "description": f"Unpacked into a folder of its own under {mods}. The loader walks that directory and treats each subfolder as one mod, so a mod IS its folder — which also makes uninstalling one directory removal.",
        "match": {"extensions": ["zip", "7z"]},
        "permissions": {"fs": [SCRATCH, grant(mods)]},
        "manage": {
            "install": [
                DOWNLOAD,
                {"action": "extract", "from": STAGED,
                 "to": {"root": "gameDir", "path": f"{mods}/{{itemId}}"}},
            ],
            "uninstall": [
                {"action": "remove", "path": {"root": "gameDir", "path": f"{mods}/{{itemId}}"}},
            ],
        },
    }}

    notes = [f"Mods live in {mods}, one folder each."]
    if g.get("note"):
        notes.append(g["note"])
    notes.append("Hard links are the default: nothing is copied twice when two sandboxes share a mod, and every file is still a real file to the game.")

    return files, {
        "defaultStrategy": "hardlink",
        "supportedStrategies": ["hardlink", "symlink", "direct"],
        "modTargets": [{"type": "game_mod", "relPath": mods}],
        "notes": notes,
    }, [
        {"id": "default", "label": "Modded", "environment": "client"},
        {"id": "clean", "label": "Clean", "description": "Nothing deployed.", "environment": "client", "strategy": "direct"},
    ], [
        {"key": "launchArgs", "label": "Additional launch arguments", "type": "text"},
    ]


def modloader(g):
    """GTA: modloader/ where the game has one, the root where it does not."""
    files = {}
    ld = g.get("loader_dir")

    if ld:
        files["manage_mod.json"] = {
            "manifestVersion": 1,
            "label": f"{g['name']} mod (modloader)",
            "description": f"Unpacked into {ld}, which loads each subfolder without touching the game's own files. It is the reason this game can be modded without a reinstall being the uninstaller.",
            "match": {"extensions": ["zip", "7z"]},
            "permissions": {"fs": [SCRATCH, grant(ld)]},
            "manage": {
                "install": [
                    DOWNLOAD,
                    {"action": "extract", "from": STAGED,
                     "to": {"root": "gameDir", "path": f"{ld}/{{itemId}}"}},
                ],
                "uninstall": [
                    {"action": "remove", "path": {"root": "gameDir", "path": f"{ld}/{{itemId}}"}},
                ],
            },
        }
        targets = [{"type": "modloader_mod", "relPath": ld},
                   {"type": "native_plugin", "relPath": ""}]
        notes = [
            f"{ld} is the safe half and is where anything that can go there should go. The game reads it without its own files being replaced, so removing a mod really does remove it.",
            "The .asi and CLEO rules write into the game ROOT, because that is where their loaders scan and there is no folder to narrow them to. That is the widest grant in this game's rules and it is worth reading before approving.",
        ]
    else:
        targets = [{"type": "native_plugin", "relPath": ""},
                   {"type": "script", "relPath": "scripts"}]
        notes = [g.get("note", "")]

    files["manage_mod.native.json"] = {
        "manifestVersion": 1,
        "label": f"{g['name']} native plugin (.asi / .cleo / .dll)",
        "description": "A loader plugin, beside the executable where its loader scans for it. Nothing is extracted: these ship as single files.",
        "match": {"extensions": ["asi", "cleo", "dll"]},
        "permissions": {"fs": [SCRATCH, grant("")]},
        "manage": {
            "install": [
                DOWNLOAD,
                {"action": "move", "from": STAGED,
                 "to": {"root": "gameDir", "path": "{fileName}"}},
            ],
            "uninstall": [
                {"action": "remove", "path": {"root": "gameDir", "path": "{fileName}"}},
            ],
        },
    }

    # Where a script package goes differs by era: III/VC/SA script mods are
    # CLEO scripts, IV's are ScriptHook .NET scripts in `scripts/`.
    if ld:
        files["manage_mod.script.json"] = {
            "manifestVersion": 1,
            "label": f"{g['name']} CLEO script (.zip)",
            "description": "A CLEO script package, unpacked into CLEO/ under a folder of its own.",
            "match": {"extensions": ["zip"], "nameContains": ["cleo"]},
            "permissions": {"fs": [SCRATCH, grant("CLEO")]},
            "manage": {
                "install": [
                    DOWNLOAD,
                    {"action": "mkdir", "path": {"root": "gameDir", "path": "CLEO"}},
                    {"action": "extract", "from": STAGED,
                     "to": {"root": "gameDir", "path": "CLEO/{itemId}"}},
                ],
                "uninstall": [
                    {"action": "remove", "path": {"root": "gameDir", "path": "CLEO/{itemId}"}},
                ],
            },
        }
    else:
        files["manage_mod.script.json"] = {
            "manifestVersion": 1,
            "label": f"{g['name']} script (.zip)",
            "description": "A script mod, unpacked into scripts/ under a folder of its own.",
            "match": {"extensions": ["zip"]},
            "permissions": {"fs": [SCRATCH, grant("scripts")]},
            "manage": {
                "install": [
                    DOWNLOAD,
                    {"action": "mkdir", "path": {"root": "gameDir", "path": "scripts"}},
                    {"action": "extract", "from": STAGED,
                     "to": {"root": "gameDir", "path": "scripts/{itemId}"}},
                ],
                "uninstall": [
                    {"action": "remove", "path": {"root": "gameDir", "path": "scripts/{itemId}"}},
                ],
            },
        }

    notes = [n for n in notes if n]
    notes.append("Direct copying is the default. These games are old enough that their loaders stat files in ways links have been reported to confuse, and the folders are small.")

    return files, {
        "defaultStrategy": "direct",
        "supportedStrategies": ["direct", "hardlink"],
        "modTargets": targets,
        "notes": notes,
    }, [
        {"id": "default", "label": "Modded", "environment": "client"},
        {"id": "clean", "label": "Clean", "description": "Nothing deployed.", "environment": "client", "strategy": "direct"},
    ], [
        {"key": "launchArgs", "label": "Additional launch arguments", "type": "text"},
    ]


def userdata(g):
    """The mod folder is under Documents, not under the install."""
    mods = g["mods"]
    exts = g["extensions"]

    files = {"manage_mod.json": {
        "manifestVersion": 1,
        "label": f"{g['name']} mod",
        "description": f"A single mod file into {mods}. The game reads mods out of your documents folder rather than out of the install, which is why this sandbox's folder is not the one Steam knows about.",
        "match": {"extensions": exts},
        "permissions": {"fs": [SCRATCH, grant(mods)]},
        "manage": {
            "install": [
                DOWNLOAD,
                {"action": "mkdir", "path": {"root": "gameDir", "path": mods}},
                {"action": "move", "from": STAGED,
                 "to": {"root": "gameDir", "path": f"{mods}/{{fileName}}"}},
            ],
            "uninstall": [
                {"action": "remove", "path": {"root": "gameDir", "path": f"{mods}/{{fileName}}"}},
            ],
        },
    }, "manage_mod.archive.json": {
        "manifestVersion": 1,
        "label": f"{g['name']} mod package (.zip)",
        "description": f"A packaged mod, unpacked into a folder of its own under {mods}. One folder deep is deliberate: it is what the game's default scan depth reaches, and it keeps a mod's files together so removing one is a single directory removal.",
        "match": {"extensions": ["zip", "7z"]},
        "permissions": {"fs": [SCRATCH, grant(mods)]},
        "manage": {
            "install": [
                DOWNLOAD,
                {"action": "extract", "from": STAGED,
                 "to": {"root": "gameDir", "path": f"{mods}/{{itemId}}"}},
            ],
            "uninstall": [
                {"action": "remove", "path": {"root": "gameDir", "path": f"{mods}/{{itemId}}"}},
            ],
        },
    }}

    notes = [g[k] for k in ("note", "note2", "note3") if g.get(k)]
    notes.append("Copying is the default here and the link strategies are not offered. This folder is in your documents and is very often on a different drive from the app's staging folder, which is the one case a hard link cannot cross.")

    return files, {
        "defaultStrategy": "direct",
        "supportedStrategies": ["direct"],
        "modTargets": [{"type": "game_mod", "relPath": mods}],
        "notes": notes,
    }, [
        {"id": "default", "label": "Modded", "environment": "client"},
        {"id": "clean", "label": "Clean", "description": "Nothing deployed.", "environment": "client", "strategy": "direct"},
    ], [
        {"key": "launchArgs", "label": "Additional launch arguments", "type": "text"},
    ]


BUILDERS = {"bethesda": bethesda, "folder": folder, "modloader": modloader, "userdata": userdata}


def write(slug, name, obj):
    d = os.path.join(ROOT, 'plugins', 'app', slug)
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, name), 'w') as f:
        json.dump(obj, f, indent=4)
        f.write('\n')


def build(g):
    files, deploy, presets, options = BUILDERS[g["family"]](g)

    for fname, obj in files.items():
        write(g["slug"], fname, obj)

    if g["steam"]:
        write(g["slug"], "launch.json", {
            "manifestVersion": 1,
            "label": f"{g['name']} (Steam)",
            "description": "Through Steam rather than the executable directly, so the overlay, the controller configuration and cloud saves behave as they normally would.",
            "permissions": {},
            "launch": {"uri": f"steam://rungameid/{g['steam'][0]}"},
        })

    detect = {
        "steamAppIds": g["steam"],
        "names": [g["name"]],
        "markers": g["markers"],
    }

    # A user-data game must NOT be found by its Steam id: that would offer the
    # install folder, which holds no mods, as the sandbox's game directory.
    if g["family"] == "userdata":
        detect = {"names": [g["name"]], "markers": g["markers"], "paths": g["userpaths"]}

    write(g["slug"], "sandbox.json", {
        "manifestVersion": 1,
        "label": f"{g['name']} sandboxes",
        "description": f"How {g['name']} sandboxes deploy, and how to find the game on this machine. Paths: {g['provenance']}.",
        "sandbox": {
            "deploy": deploy,
            "detect": detect,
            "presets": presets,
            "options": options,
        },
    })


def main():
    args = [a for a in sys.argv[1:] if not a.startswith('-')]
    force = '--force' in sys.argv

    wanted = [g for g in GAMES if not args or g["slug"] in args]

    if args and not wanted:
        print(f"no such game: {args}", file=sys.stderr)
        return 2

    for g in wanted:
        d = os.path.join(ROOT, 'plugins', 'app', g["slug"])
        if os.path.exists(d) and not force:
            print(f"  skip {g['slug']} (exists; --force to overwrite)")
            continue
        build(g)
        print(f"  wrote {g['slug']:<22} {g['family']:<10} {g['provenance']}")

    return 0


if __name__ == '__main__':
    sys.exit(main())
