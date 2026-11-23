# Changelog

Notable changes, newest first. The format follows [Keep a Changelog](https://keepachangelog.com); versions follow
[Semantic Versioning](https://semver.org).

## [Unreleased]

### Added
- Remote access: your running sessions on a phone or another computer, over your own Tailscale network. A card per session
  with its last lines, one-tap answers and a reply line, and each session as a full terminal with a row of keys for a phone.
  Off unless you turn it on (`> Remote access`). A device pairs by scanning a code that works once; each device then has a
  token of its own and can be removed, and the window shows when one is connected. On Windows the Remote access screen
  checks the firewall and can add the one rule that lets Tailscale devices through. `maestro --remote` serves it without the
  window. See [docs/remote.md](docs/remote.md).
- Remote access shows the browser a session drives: a Browser view beside the conversation and the terminal, with a picture
  of the page each time it changes, the address, and a choice between the pages it has open. It needs the browser started
  with a debugging port; for the Playwright tool there is a settings file to point it at
  ([docs/remote.md](docs/remote.md#the-browser-view)).
- Remote access shows a Claude Code session as its conversation: what was said and done, read from Claude Code's own record,
  with a message line. A session started from the page says that it is starting, then opens as its conversation; the project
  it will start in is named on the sheet, and a session nobody named is called after its project.
- A session started from a paired device gets a tab in the window too, without restarting Maestro.
- Remote access has a Projects list: every repo with its branch, what has changed in it and the sessions at work there, its
  commits, and diffs to read on a phone. Read only.
- A page that is not paired scans the pairing code itself, and `> Pair a phone` shows one.
- The home screen has an **On your phone** panel: turn remote access on and pair a phone from there.
- Console tabs can be put in any order: drag a tab to another slot, or press `Ctrl+Shift+PageUp` / `PageDown` to move the
  session you are in. The tiles and the `Alt+1`…`9` numbers follow, and sessions that outlive the app come back in that order.

## [0.3.1] - 2026-11-20

### Added
- macOS and Linux. A project there is simply its folder; sessions live in a tmux server on the machine and outlive the app;
  the shell button opens your own shell. Clipboard, start at sign-in, the update system and the app's own files each use what
  the system provides. Releases carry a build for Windows, Linux, and Apple-silicon and Intel Macs.

### Fixed
- A desktop without a tray no longer stops the app from starting.

## [0.3.0] - 2026-11-19

### Added
- The home screen starts sessions: type a task, pick a project, and Claude Code starts there while you stay on the screen.
- Session cards, on the home screen and in the Console's "All": the answers a session is waiting for are buttons, any session
  can be replied to from its card, and a small trace shows how busy it has been over the last five minutes.
- One project picker everywhere: a list you can type into, favourites first and starred in place, with what is running in each.

### Changed
- The Console's start bar is one control: where, then Claude, Ubuntu or PowerShell, with drawn icons instead of letters.

## [0.2.0] - 2026-11-19

### Added
- Updates: `> Check for updates` asks GitHub for the latest release, shows what changed and installs it in one step. Checking
  at start is opt-in ("Check when Maestro starts"); a newer version then shows on the home screen.
- `Ctrl+V`, `Shift+Insert` and a right click paste into a session, as one paste; the clipboard is read by the app when the
  page is refused. Before, plain `Ctrl+V` sent a control key instead of the text.
- Projects can be starred and unstarred from the Console's one-click list, and any project added to it from a picker there.

## [0.1.0] - 2026-11-19

The first public version.

### Added
- The Console: Claude Code, Ubuntu and PowerShell sessions in any project, as tabs, tiles (1, 2, 4, 6), an overview card per
  session, and windows of their own. Sessions inside WSL outlive the app and are reopened with their scroll-back.
- The harness screens: Memory, Skills, Rules, Hooks, Work and Audits, with one search across all of them; edit a memory, a
  skill or a rule; move or retire a rule; new memory, new skill.
- The Git tab, read only: changes with diffs, history, tree.
- A welcome screen for a machine with no projects: point it at a folder and it works out the rest.
- Projects whose agent runs on Windows itself, without WSL.
- Projects that keep their harness in `AGENTS.md` and `.agents/`.
- The starter harness: instructions, a rules file, `/log`, `/rule` and `/recall`, and a first log entry, in either layout,
  written only where files are missing.
- The work log is read from its dated files when there is no index; earlier conversations are read from Claude Code's own
  transcripts.
- `hotkey` in `maestro.config.json` is honoured; if another program holds the key the app still starts.
- A browser preview with invented data for working on the interface (`tools/preview`).
