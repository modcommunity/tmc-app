# TMC App

The official [The Modding Community](https://moddingcommunity.com) app, built
with [Tauri 2](https://tauri.app/), Rust and React.

One codebase, five targets: **Windows, macOS, Linux, Android and iOS**.

### What it does

- Browse and view assets, mods, servers, server maps, articles, communities,
  collections and members
- Sign in through your own browser — the app never sees your password
- One-click installs via declarative, sandboxed plugins
- Real latency pings measured from your device
- Live server queries over game-specific protocols
- Settings split between this device and your account

### Getting started

```bash
npm install
npm run desktop      # or: android / ios
```

Linux desktop builds need `webkit2gtk-4.1`, `libsoup-3.0` and `pkg-config`.

### Contributing plugins

A plugin is a folder with a `plugin.json` in it. See `examples/plugins/` for a
working installer, server-query and theme plugin, and `CLAUDE.md` for the format
and the sandbox rules.

Plugins are **data, not code** — they describe steps the app carries out inside
a jail the user grants, and every action is written to the activity log.

### Still in development & not finished
