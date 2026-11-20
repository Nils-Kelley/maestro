#!/bin/bash
# Build the release from WSL with the Windows toolchain (cargo on the Windows side). `tools/build.sh test` runs the tests instead.
# The page's script is syntax-checked first: a script that does not parse leaves the app sitting on its intro graphic with nothing
# running, and cargo cannot see that.
set -e
here="$(cd "$(dirname "$0")/.." && pwd)"
node "$here/tools/check-page.mjs"
win="$(wslpath -w "$here/src-tauri")"
what="build --release"; [ "$1" = test ] && what="test"
cd /mnt/c && cmd.exe /c "cd /d $win && cargo $what 2>&1" | tr -d '\r' | grep -E "^error|^warning|^test |test result|Finished|-->|^\s+\|" || true
