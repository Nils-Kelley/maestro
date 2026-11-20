# Contributing

Thanks for looking. Maestro is small on purpose: one Rust file, one page. Changes that keep it that way are the easiest to take.

## Getting it running

- **The interface only:** `node tools/preview/serve.mjs`, then open `http://localhost:4173/?scene=full`. The page runs in your
  browser against an invented project (`tools/preview/stub.js`), so you need neither Rust nor a real project. Other scenes:
  `?scene=bare` (a project with nothing set up) and `?scene=welcome` (first run).
- **The app:** `cd src-tauri && cargo build --release`, on Windows. From WSL, `tools/build.sh` does the same through the Windows
  toolchain and checks the page's script first.
- **Tests:** `cd src-tauri && cargo test` (or `tools/build.sh test`).

## Where things are

| Path | What |
| --- | --- |
| `src-tauri/src/main.rs` | Everything that touches the disk, git, WSL and the terminals. Each `#[tauri::command]` is one call the page can make. |
| `ui/index.html` | The whole interface: styles, markup and script in one file, embedded at build time. |
| `ui/vendor/` | xterm.js, vendored so the app needs no network. |
| `wsl/` | The two files that keep sessions alive inside WSL; built into the program. |
| `starter/` | The starter harness templates (`{{project}}`, `{{date}}`, `{{dir}}`, `{{instructions}}` are filled in). |
| `tools/preview/` | The browser preview and its invented data. |
| `docs/` | What Maestro reads, and the kit contract. |

## Conventions

- **The page never touches the disk.** New behaviour that reads or writes files is a command in `main.rs`; the page calls it.
- **Check every path that comes from the page.** A file name is one segment (`safe`), a path stays inside its repo (`inside`),
  a commit id is hex (`hexish`). Follow the commands that are already there.
- **Never replace a user's file silently.** Edits keep a first copy of the day; the starter writes missing files only.
- **A new command gets a test** when it parses or writes something, and a canned answer in `tools/preview/stub.js` when the page
  calls it.
- **If the page calls a new command, add it to `generate_handler!`** at the bottom of `main.rs`, or the call fails at run time.
- **A window-creating command is `async fn`.** Built from a blocking command, a second window opens blank on Windows.
- **Words in the interface are plain.** Say what a button does; no exclamation marks; no "simply".
- Commit messages follow Conventional Commits (`feat:`, `fix:`, `docs:`, `chore:`).

## Before you open a pull request

1. `cargo test` passes and `cargo build --release` has no warnings.
2. The page's script parses: `tools/build.sh` checks it, or open the preview and watch the browser console.
3. You tried the change in the preview, and in the real app if it touches sessions, windows or the disk.
4. `node tools/leak-check.mjs` is clean (below).

## Keeping private names out of commits

Maestro is developed against real projects, and it is easy for a client's name or a home folder to end up in a test or a
screenshot. `node tools/leak-check.mjs` scans what is about to be committed for:

- home folders and user names in paths (`/home/<someone>/`, `C:\Users\<someone>\`),
- email addresses,
- every project name and folder in your own `maestro.config.json` (it is git-ignored; the names are read from it, never stored),
- any extra pattern you list in `.private-words` (also git-ignored), one per line.

Run `node tools/leak-check.mjs --install` once to make it a pre-commit and pre-push hook. Use invented names in tests, examples and
screenshots: the preview's project is called `tidepool`. Screenshots for the README come from the preview, never from a real project.
