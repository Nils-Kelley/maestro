#!/bin/bash
# term.sh — the sessions Maestro shows. Maestro copies this file and tmux.conf into ~/.local/share/maestro/ inside WSL and calls
# it from Windows: `wsl.exe --cd <project> --exec bash ~/.local/share/maestro/term.sh …`
#   --new <name> <kind> <label> [claude args…]   start a session (kind: claude | ubuntu) inside tmux and attach to it
#   --attach <name>                              attach to one that is still running
#   --list                                       name, created, kind, label, folder, attached — one session per line, tab-separated
#   --kill <name>                                end one
#   --label <name> <text>                        what the session is called (a Claude session is named after the first thing asked)
#   --check                                      which of tmux and claude this shell can find
# Sessions live in a tmux server of their own (-L maestro), so closing Maestro only detaches: the program keeps running with its
# scroll-back. Sessions named reins-… were started by a build from before the app was renamed and live in the server of that name.
# A shell started by wsl.exe has loaded neither the profile nor nvm: set PATH up so `claude` and the hooks' node are found.
[ -f "$HOME/.profile" ] && . "$HOME/.profile" >/dev/null 2>&1
N=$(ls -d "$HOME"/.nvm/versions/node/*/bin 2>/dev/null | sort -V | tail -1)
export PATH="${N:+$N:}$HOME/.local/bin:$PATH"
export TERM=xterm-256color COLORTERM=truecolor
CONF="$(cd "$(dirname "$0")" && pwd)/tmux.conf"
server() { case "$1" in reins-*) echo reins ;; *) echo maestro ;; esac; }
case "$1" in
  --check) command -v tmux >/dev/null 2>&1 && echo tmux; command -v claude >/dev/null 2>&1 && echo claude; exit 0 ;;
  --list) for s in maestro reins; do tmux -L "$s" list-sessions -F '#{session_name}	#{session_created}	#{@kind}	#{@label}	#{session_path}	#{session_attached}' 2>/dev/null; done; exit 0 ;;
  --kill) tmux -L "$(server "$2")" kill-session -t "=$2" 2>/dev/null; exit 0 ;;
  --label) tmux -L "$(server "$2")" set-option -t "=$2" @label "$3" 2>/dev/null; exit 0 ;;
  --attach) exec tmux -L "$(server "$2")" -f "$CONF" attach-session -t "=$2" ;;
  --new)
    name="$2"; kind="$3"; label="$4"; shift 4
    if [ "$kind" = claude ] && ! command -v claude >/dev/null 2>&1; then
      echo "Claude Code was not found in this WSL distro (looked on PATH, in ~/.local/bin and under ~/.nvm)."
      echo "Install it (https://code.claude.com/docs), then start the session again."; exec bash -l
    fi
    if ! command -v tmux >/dev/null 2>&1; then   # no tmux here: run it directly (it then ends with Maestro)
      [ "$kind" = claude ] && exec claude "$@"
      exec bash -l
    fi
    if [ "$kind" = claude ]; then cmd=$(printf '%q ' claude "$@"); else cmd="bash -l"; fi
    exec tmux -L maestro -f "$CONF" new-session -s "$name" "$cmd" ';' set-option @kind "$kind" ';' set-option @label "$label" ;;
  *) echo "term.sh: --new | --attach | --list | --kill | --label | --check" >&2; exit 2 ;;
esac
