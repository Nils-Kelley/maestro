# Validation Gates
## Phase Complete
- [ ] The page scripts parse: `node tools/check-page.mjs`
- [ ] Tests pass with no warnings: `cargo test` in `src-tauri` (through the Windows toolchain from WSL)
- [ ] `cargo build --release` has no warnings
- [ ] No private names in what would be committed: `node tools/leak-check.mjs --all`
- [ ] No unfinished TODOs introduced this session
- [ ] The change was tried: in the preview for the interface, in the real app (or a throwaway `--remote` server and a
      browser) when it touches sessions, windows, the disk or remote access
- [ ] `CHANGELOG.md` has the change under Unreleased, and `docs/` says what a user now can do

## Milestone Complete
- All phase gates above, plus:
- [ ] All requirements mapped to this milestone are satisfied, and REQUIREMENTS.md says so
- [ ] CI is green on Windows, Linux and both kinds of Mac
- [ ] No regressions in existing functionality
