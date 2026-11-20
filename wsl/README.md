# The WSL side

Maestro runs on Windows. For a project whose agent runs inside WSL, Claude Code and the Ubuntu terminals run there, and these two
files keep them alive. They are built into the program: Maestro writes them to `~/.local/share/maestro/` in the distro the first
time a session is opened (and again whenever a new build carries a different copy). You do not install them by hand.

- `term.sh` starts, lists, attaches to, names and ends sessions. Sessions run in a tmux server of their own (`tmux -L maestro`),
  apart from any tmux you use yourself, so they keep running when Maestro closes. Without `tmux` in the distro a session still
  opens; it then ends with the app.
- `tmux.conf` is that server's settings: no status bar, mouse on, highlighting copies, and copying while scrolled back keeps
  your place.

`term.sh` finds `claude` on the PATH that `~/.profile` sets, in `~/.local/bin`, and under the newest Node that nvm installed.
If your `claude` lives somewhere else, put that folder on the PATH in `~/.profile`.
