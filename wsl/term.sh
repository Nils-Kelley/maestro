#!/bin/bash
# term.sh — the sessions Maestro shows. Maestro copies this file, tmux.conf and playwright.json into ~/.local/share/maestro/ and calls it with bash:
# on macOS and Linux directly, on Windows inside WSL (`wsl.exe --cd <project> --exec bash ~/.local/share/maestro/term.sh …`).
#   --new <name> <kind> <label> [claude args…]   start a session (kind: claude | ubuntu) inside tmux and attach to it
#   --attach <name>                              attach to one that is still running
#   --list                                       name, created, kind, label, folder, attached — one session per line, tab-separated
#   --kill <name>                                end one
#   --label <name> <text>                        what the session is called (a Claude session is named after the first thing asked)
#   --peek                                       the last lines on each session's screen: a line \036<name>, then the lines
#   --say <name> <text> [stay]                   paste a message into a session and press Enter (with `stay`, type it as keys, no Enter)
#   --chat <name>                                a line "created<TAB>folder<TAB>conversation id", then the session's screen and recent scroll-back
#   --browsers                                   name<TAB>port, one line per session whose programs drive a Chrome that can be watched
#   --check                                      which of tmux and claude this shell can find
# Sessions live in a tmux server of their own (-L maestro), so closing Maestro only detaches: the program keeps running with its
# scroll-back. Sessions named reins-… were started by a build from before the app was renamed and live in the server of that name.
# A shell started by wsl.exe, or by an app opened from the dock, has loaded neither the profile nor nvm: set PATH up so that
# `claude`, `tmux` and the hooks' node are found (Homebrew's folders included).
[ -f "$HOME/.profile" ] && . "$HOME/.profile" >/dev/null 2>&1
N=$(ls -d "$HOME"/.nvm/versions/node/*/bin 2>/dev/null | sort -V 2>/dev/null | tail -1)
for d in /opt/homebrew/bin /usr/local/bin; do case ":$PATH:" in *":$d:"*) ;; *) [ -d "$d" ] && PATH="$PATH:$d" ;; esac; done
export PATH="${N:+$N:}$HOME/.local/bin:$PATH"
SH="${SHELL:-bash}"; command -v "$SH" >/dev/null 2>&1 || SH=bash
export TERM=xterm-256color COLORTERM=truecolor
CONF="$(cd "$(dirname "$0")" && pwd)/tmux.conf"
server() { case "$1" in reins-*) echo reins ;; *) echo maestro ;; esac; }
case "$1" in
  --check) command -v tmux >/dev/null 2>&1 && echo tmux; command -v claude >/dev/null 2>&1 && echo claude; exit 0 ;;
  --list) for s in maestro reins; do tmux -L "$s" list-sessions -F '#{session_name}	#{session_created}	#{@kind}	#{@label}	#{session_path}	#{session_attached}' 2>/dev/null; done; exit 0 ;;
  --kill) tmux -L "$(server "$2")" kill-session -t "=$2" 2>/dev/null; exit 0 ;;
  --label) tmux -L "$(server "$2")" set-option -t "=$2" @label "$3" 2>/dev/null; exit 0 ;;
  --peek) for s in maestro reins; do for n in $(tmux -L "$s" list-sessions -F '#{session_name}' 2>/dev/null); do printf '\036%s\n' "$n"; tmux -L "$s" capture-pane -p -t "=$n:" 2>/dev/null | grep -v '^[[:space:]]*$' | tail -n 40; done; done; exit 0 ;;
  --say) # with `stay` the text is typed as keys; otherwise it goes in as one paste (so several lines stay one message), then Enter
    s="$(server "$2")"
    if [ "$4" = stay ]; then tmux -L "$s" send-keys -t "=$2:" -l -- "$3" 2>/dev/null
    else tmux -L "$s" set-buffer -b maestro-say -- "$3" 2>/dev/null && tmux -L "$s" paste-buffer -d -p -b maestro-say -t "=$2:" 2>/dev/null; sleep 0.15; tmux -L "$s" send-keys -t "=$2:" Enter 2>/dev/null; fi
    exit 0 ;;
  --chat) # what is needed to find a session's conversation: when it started, where, its conversation id if known; then its screen with some of what scrolled by
    # asked of list-sessions, which only walks sessions that exist: `display-message -t` goes on without its target when the session
    # has just ended, and tmux 3.2a then crashes on #{session_created}, taking the server and every session in it down
    s="$(server "$2")"; head=$(tmux -L "$s" list-sessions -F '#{session_name}	#{session_created}	#{session_path}	#{@sid}' 2>/dev/null | awk -F'\t' -v n="$2" '$1 == n { print $2 "\t" $3 "\t" $4; exit }')
    [ -n "$head" ] || exit 0; printf '%s\n' "$head"
    tmux -L "$s" capture-pane -p -S -150 -t "=$2:" 2>/dev/null; exit 0 ;;
  --browsers) # the Chrome a session's programs started (Playwright's, say), where it was started with --remote-debugging-port: it is
    # found among the processes under the session's pane, and the port it chose is the first line of a file in its profile folder
    all=$(ps -eo pid=,ppid=,args= 2>/dev/null) || exit 0
    for s in maestro reins; do tmux -L "$s" list-panes -a -F '#{session_name}	#{pane_pid}' 2>/dev/null; done | while IFS='	' read -r n root; do
      printf '%s\n' "$all" | awk -v root="$root" '{ par[$1] = $2; if (index($0, "--remote-debugging-port=") && !index($0, " --type=") && match($0, /--user-data-dir=[^ ]+/)) dir[$1] = substr($0, RSTART + 16, RLENGTH - 16) }
        END { for (p in dir) { a = p; for (k = 0; k < 12 && a > 1; k++) { if (a == root) { print dir[p]; break } a = par[a] } } }' | while read -r d; do
        port=$(head -n 1 "$d/DevToolsActivePort" 2>/dev/null); case "$port" in ''|*[!0-9]*) ;; *) printf '%s\t%s\n' "$n" "$port"; break ;; esac
      done
    done; exit 0 ;;
  --attach) exec tmux -L "$(server "$2")" -f "$CONF" attach-session -t "=$2" ;;
  --new)
    name="$2"; kind="$3"; label="$4"; shift 4
    if [ "$kind" = claude ] && ! command -v claude >/dev/null 2>&1; then
      echo "Claude Code was not found here (looked on PATH, in ~/.local/bin and under ~/.nvm)."
      echo "Install it (https://code.claude.com/docs), then start the session again."; exec "$SH" -l
    fi
    if ! command -v tmux >/dev/null 2>&1; then   # no tmux here: run it directly (it then ends with Maestro)
      [ "$kind" = claude ] && exec claude "$@"
      exec "$SH" -l
    fi
    # a new Claude conversation is given an id of our choosing, kept with the session, so that its conversation file is known
    sid=""
    if [ "$kind" = claude ]; then
      case " $* " in *" --resume "*|*" --continue "*|*" --session-id "*) ;; *)
        if claude --help 2>/dev/null | grep -q -- '--session-id'; then sid=$( (cat /proc/sys/kernel/random/uuid 2>/dev/null || uuidgen 2>/dev/null) | tr 'A-Z' 'a-z'); fi ;;
      esac
      [ -n "$sid" ] && set -- --session-id "$sid" "$@"
    fi
    if [ "$kind" = claude ]; then cmd=$(printf '%q ' claude "$@"); else cmd=$(printf '%q -l' "$SH"); fi
    exec tmux -L maestro -f "$CONF" new-session -s "$name" "$cmd" ';' set-option @kind "$kind" ';' set-option @label "$label" ';' set-option @sid "$sid" ;;
  *) echo "term.sh: --new | --attach | --list | --peek | --say | --chat | --browsers | --kill | --label | --check" >&2; exit 2 ;;
esac
