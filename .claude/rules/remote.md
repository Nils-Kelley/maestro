---
paths:
  - "ui/remote.html"
  - "docs/remote.md"
  - "src-tauri/src/main.rs"
---
# Remote access

- `docs/remote.md` says what remote access opens and to whom. A change to what a paired device can do changes that text in
  the same commit.
- The page's one `<script>` block is allowed by its SHA-256 (`remote_scripts`). No inline handlers (`onclick=`), no
  `javascript:` addresses, no script added as text: the browser refuses them. Styles may be inline.
- What the app sends is still not written into HTML raw: text goes through `esc`, numbers are made numbers first.
- A new thing a device may do is a permission of its own on the device (as `screens` is), off until it is given at the
  computer, visible in the window while it is used, and written to `maestro.log`.
- Pictures go to a device through a queue of two and wait for `seen`: nothing a slow or silent device does may grow memory.
- To try the server without touching the installed app: put a `maestro.config.json` with its own port beside the debug
  program and run `maestro.exe --remote`. Stop that process by its path under `target\debug`, never by name alone, and
  delete the config and log afterwards.
- Whatever a device does that changes something (types, starts, ends, clicks) is in the list at the top of `remote_op` that
  a watch-only device is refused. A new such thing is added to that list.
- The page reads its token from storage at each connection. The computer replaces it daily, and a replaced token shown
  again removes the device.
- A test may move the real pointer a few pixels and press F24. It never clicks and never types: somebody is at the machine.
