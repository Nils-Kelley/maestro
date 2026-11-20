#!/bin/sh
# Put Maestro where a program belongs on macOS or Linux: ~/.local/bin/maestro, and on Linux an entry in the applications menu.
# Run it from an unpacked release (the program is beside this file) or from the repository after `cargo build --release`.
set -e
here="$(cd "$(dirname "$0")" && pwd)"
src="$here/maestro"; [ -f "$src" ] || src="$here/../src-tauri/target/release/maestro"
[ -f "$src" ] || { echo "no build found: run cargo build --release in src-tauri first" >&2; exit 1; }
bin="$HOME/.local/bin"; mkdir -p "$bin"
pkill -x maestro 2>/dev/null || true
cp "$src" "$bin/maestro.new" && chmod +x "$bin/maestro.new" && mv -f "$bin/maestro.new" "$bin/maestro"
case "$(uname -s)" in
  Darwin)
    # a program downloaded by a browser is held back until this mark is taken off
    xattr -d com.apple.quarantine "$bin/maestro" 2>/dev/null || true ;;
  *)
    apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"; mkdir -p "$apps"
    printf '[Desktop Entry]\nType=Application\nName=Maestro\nComment=Your agents and their harness, in one place\nExec=%s/maestro\nTerminal=false\nCategories=Development;\n' "$bin" > "$apps/maestro.desktop" ;;
esac
echo "installed to $bin/maestro"
case ":$PATH:" in *":$bin:"*) ;; *) echo "note: $bin is not on your PATH; start it as $bin/maestro" ;; esac
