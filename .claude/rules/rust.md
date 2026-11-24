---
paths:
  - "src-tauri/**"
---
# The Rust side

- From a session Maestro itself started in WSL, Windows cargo is not on PATH and `tools/build.sh` builds nothing: call cargo
  through `cmd.exe /c` with the user's `.cargo\bin` put on PATH first. In `cmd`, write `set NAME=value&& next` with no space
  before `&&`, or the space becomes part of the value.
- Installing (`tools/install.ps1`) restarts the user's running app. Say so first, and install once per piece of work.
- Windows calls are raw `extern "system"` blocks with handles as `isize`; no `windows` crate. FFI that only exists on one
  system sits behind `#[cfg]`; everything else uses `if cfg!(windows)` so every branch is type-checked everywhere.
- What only Windows reads needs `#[cfg_attr(not(windows), allow(dead_code))]`: a release build has no warnings on any system,
  and the other systems are only compiled by CI.
- A thread that measures or copies displays first calls `screen::aware()`: without it, `maestro --remote` (no window) sees
  scaled sizes.
- A test that needs the real machine (displays, Tailscale) returns early where there is none, so CI stays green.
- A window is moved through the `drag` command, never through the framework's `start_dragging` from the page: on Windows
  tao remembers a move until `WM_EXITSIZEMOVE`, and a move asked for with the button already up never ends, which drops
  every later one. `drag` asks only while the button is down and clears that memory first.
