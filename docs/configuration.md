# Configuration

Maestro keeps one file, `maestro.config.json`. The app writes it; you rarely edit it by hand.
[`maestro.config.example.json`](../maestro.config.example.json) shows the shape.

| System | Where |
| --- | --- |
| Windows | beside the program, or `%APPDATA%\Maestro` when that folder cannot be written |
| macOS | `~/Library/Application Support/Maestro` |
| Linux | `~/.config/maestro` |

| Field | Meaning |
| --- | --- |
| `hotkey` | The key that shows and hides the window, for example `Ctrl+Shift+Space` or `Alt+Space`. Read at start. |
| `checkUpdates` | `true` to ask GitHub for the latest release at each start. Off unless you turn it on. |
| `projects[].name`, `root` | The name shown and the folder, as Windows opens it |
| `projects[].runs` | Where this project's agent runs: `wsl` or `windows` on Windows, `native` on macOS and Linux |
| `projects[].wslRoot`, `distro` | The folder as WSL sees it, and the distro (WSL projects) |
| `projects[].claudeHome`, `memoryDir` | Claude's own folder and the project's memory folder |
| `projects[].favorite` | Listed first, with one-click start buttons |
| `projects[].audits` | Extra folders whose `.md` and `.txt` files appear under Audits |
| `projects[].kit` | Optional: a script that answers the Ship and Checks screens ([kit.md](kit.md)) |

Per project, `<harness>/maestro.json` may hold `{ "hot": ["src/a.ts", …], "limit": 120000 }`: the files whose rule load the
home screen measures, and the number of characters it is held against.

## What Maestro writes

On disk, only this:

- `maestro.config.json` and `maestro.log` (interface errors), in the folder above.
- A memory, skill or rule when you save an edit. The first copy of a changed file each day is kept in `<harness>/archive/`
  (skills: `~/.claude/backups/`). A retired rule is kept in `<harness>/archive/retired-laws.md`.
- The starter harness, when you ask for it: missing files only, nothing is replaced.
- `term.sh` and `tmux.conf` in `~/.local/share/maestro/` (on Windows inside the WSL distro), the first time a session is
  opened. See [wsl/README.md](../wsl/README.md).
- A new copy of the program, when you install an update. The one it replaces is kept beside it with `.old` added until the
  next start.
- Start at sign-in, when you tick it: one value under the current user's `Run` registry key on Windows, a launch agent in
  `~/Library/LaunchAgents` on macOS, an entry in `~/.config/autostart` on Linux.

The first thing asked in a session is shown in lists. Anything that looks like a pasted login is cut from that line before it
is displayed.

## Network

There is no server, no port and no telemetry. The interface talks to the Rust side over the window's own IPC.

The only request Maestro makes is the update check: one question to GitHub's public API, made when you open the Updates screen,
or at start if you turned `checkUpdates` on. An update is downloaded over HTTPS from this repository's releases and is not
signed.
