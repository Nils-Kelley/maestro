# Changelog

Notable changes, newest first. The format follows [Keep a Changelog](https://keepachangelog.com); versions follow
[Semantic Versioning](https://semver.org).

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
