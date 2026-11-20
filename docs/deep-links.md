# Deep links

`gs://` is the GameSpice App's scheme and what new links use; `tmc://`, the original, stays registered so every link already out there keeps working. Both have the same grammar (`SCHEMES` in `tmc_core::deeplink`) and both are declared for desktop and mobile in `tauri.conf.json` (`plugins.deep-link`; the mobile entry is a custom scheme with `appLink: false`, so the Android and iOS projects `tauri android init`/`tauri ios init` generate get both). The table below shows `gs://`; every row works with `tmc://` too. One rule governs all of it:

> **A link can ask the app to SHOW something. It can never ask the app to DO something.**

Any program on the machine can claim a custom scheme and any web page can navigate to one without a click, so a link is an untrusted request from an unknown party and the most it achieves is a screen with a button on it.

| Link | Does |
| --- | --- |
| `gs://auth` | Polls the API now instead of waiting out the interval. Carries nothing |
| `gs://install/<kind>/<id>` | Opens that item's page with the install controls ringed |
| `gs://view/<kind>/<id>` | Opens that item's page |
| `gs://sandbox/<id>` | Opens one sandbox |
| `gs://play/<host>:<port>` | Opens a **join screen** for that address |
| `gs://play/app/<slug\|id>[?solo=<owner>/<repo>[&solo-version=<v>]]` | Opens that game's launch dialog; `solo` names the game pack to run with no server |

**`gs://play/<host>:<port>` is the website's Join button**, and it is the only link whose payload is not an id of ours. It is still only a screen: `/join` resolves the address through `/servers/lookup`, shows which game is running there with the latency measured from this device, and offers the launch modes this machine actually has. The rule is load-bearing here rather than theoretical, because this is the link an ordinary web page can navigate to without a click — an auto-joining version would be a remote primitive for making somebody's machine connect to an address a stranger chose.

`solo`/`solo-version` (the site sends the hyphen; the legacy `solo_version` is still read) are the one query the grammar accepts, and only on `play/app`: each is checked by `games::build::SoloPack::new` (a closed character set, no leading `-`, since both become process arguments) and silently dropped when it fails, so the link still opens the dialog. They ride to the launch dialog as `/apps?app=..&start=1&solo=..&soloVersion=..` and still wait for a click; the site's own `boot.solo` wins over the link at launch.

Properties of the grammar:

- **The address is in the PATH, not a query string.** Custom-scheme openers across five platforms disagree about almost everything and agree about path segments, and a grammar with two segments is far easier to keep honest than one accepting arbitrary parameters — every parameter a link can carry is a parameter somebody eventually reads.
- **Exactly the two shapes, with nothing after them.** `gs://auth` ignores a trailing segment, and that is right for a wake-up with no arguments. Here it would be wrong for the opposite reason: this action HAS a grammar, and accepting `gs://play/host:1/join` today is how `/join` quietly becomes meaningful the first time somebody adds a segment to the match.
- **The port is optional**, because `gs://play/example.com` is a perfectly good way to name a box, and `gs://play/:` — the shape an empty `{host}` substitution used to produce — parses to nothing. The website declines to build that link now; this is the other half of the same fix, on the side that would have to act on it.

`gs://install/mod/1234` is the **guest** flow: a signed-in user gets a subscription, which follows them to their other devices and keeps the mod updated, but a guest has no account to hang one on.

Everything else parses to nothing, and an unrecognised link does nothing at all — it does not fall through to the auth wake-up, because a default action is a default action an attacker gets to trigger. `deeplink.rs`'s test names the shapes that must never start working: `gs://deploy/1`, `gs://settings/gameDir?path=/`, `gs://rcon/1/exec?command=quit`.
