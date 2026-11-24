# Maestro
A desktop console for Claude Code: every session side by side, what the agent reads before it works, and the sessions from a
phone. Rust and Tauri 2, one Rust file and one page. Open source (MIT).

## Current State
**Last updated:** 2026-11-23
- Released: 0.4.1. Unreleased on `feature/blackout`: dark screens, a release name beside the Cargo version.
- On `feature/takeover`, installed on the maintainer's machine: the computer's screens on a paired device and its pointer
  and keys, permissions given device by device, a daily token change, an "only the secure address" setting, and a fix for
  a window that could no longer be dragged. Checked against a throwaway server in a desktop browser; clicks, typing and
  touch on a real phone are still to be tried.

## Roadmap
- [ ] M001: Takeover — see and use the computer's screens from a paired device
  - [ ] Phase 1: see the screens (TAKE-01, TAKE-02, TAKE-03, TAKE-04)
    - [x] Capture each display on Windows, send it as JPEG over the remote WebSocket
    - [x] Screens view on the remote page: every display, one alone, zoom, fill the screen
    - [x] Per-device permission, off by default; the window says while a device looks; log lines
    - [x] Send only the parts of a display that changed
    - [ ] Tried on a real phone over Tailscale, and with dark screens on
  - [ ] Phase 2: use the screens (TAKE-05, TAKE-06, SEC-01, SEC-02, SEC-03, SEC-04)
    - [x] Pointer and keys through `SendInput`, as a permission of its own that needs the screens
    - [x] Trackpad and touch modes, the phone's keyboard with a key row, a computer's mouse and keyboard
    - [x] Permissions for an hour, a day or until taken back; a device that only watches
    - [x] A token replaced daily, a replaced one shown again ends the pairing
    - [x] "Answer only at the secure address"
    - [ ] Clicks, typing and the gestures tried on a real phone; the secure-only setting tried with the secure address on
  - [ ] Phase 3: beyond the first cut (TAKE-07)
    - [ ] macOS and Linux capture and input
    - [ ] A device proves itself with a key that cannot be copied out of its browser (needs the secure address)
    - [ ] Using the pointer from a device without ending dark screens
    - [ ] Video (H.264) where a secure address allows it, if pictures prove too slow

## Current Focus
M001 → Phases 1 and 2 are built: the phone test of both, then a release.

## Architecture
- `src-tauri/src/main.rs`: everything that touches the disk, git, WSL, the terminals, the remote server and the screens.
  Each `#[tauri::command]` is one call the window's page can make.
- `ui/index.html`: the whole desktop interface, embedded at build time. `ui/remote.html`: the page a paired device opens,
  served by the app on one port with one WebSocket (text for everything, binary for pictures of the screens).
- `wsl/`: what keeps sessions alive in tmux inside WSL. `starter/`: harness templates. `tools/preview/`: the interface in a
  browser against invented data. `docs/`: what users read; `docs/remote.md` is also the security statement of remote access.

## Key Patterns
See `CONTRIBUTING.md` (conventions, releasing) and `.claude/rules/`. In short: the page never touches the disk; every path
from a page is checked; a new command gets a test, a stub in `tools/preview/stub.js` and a place in `generate_handler!`.

## Key Decisions
- **Remote access answers only Tailscale and this machine, and adds no encryption of its own.** Tailscale carries it; a
  device pairs once and holds a token of which only the hash is kept.
- **A paired device that types is as trusted as someone at the keyboard.** For it, permissions such as the screens guard
  against accident and make things visible. They are a wall only for a device set to only watch (`type: false`).
- **A device's token changes daily and a replaced one is a tripwire** (`device_turn`): the page must read its token from
  storage at every connection, or two tabs would trip it.
- **Screens are captured with GDI, not Desktop Duplication.** No heavy dependency, same raw `user32` style as the rest;
  changed parts are found by comparing with what the device already has. Revisit only if frame rate proves short.
- **The remote page's script is allowed by its SHA-256**, so the page must keep exactly its scripts in `<script>` blocks and
  no inline handlers.

## Session Workflow
At the end of each session, update Current State and the Roadmap, and append to `.claude/METRICS.md`.
