# The session scripts

These two files keep sessions alive when Maestro closes. On macOS and Linux they run on the machine itself; on Windows, for a
project whose agent runs inside WSL, they run inside the distro (hence this folder's name). They are built into the program:
Maestro writes them to `~/.local/share/maestro/` the first time a session is opened (and again whenever a new build carries a
different copy). You do not install them by hand.

- `term.sh` starts, lists, attaches to, names and ends sessions. Sessions run in a tmux server of their own (`tmux -L maestro`),
  apart from any tmux you use yourself, so they keep running when Maestro closes. Without `tmux` in the distro a session still
  opens; it then ends with the app.
- `tmux.conf` is that server's settings: no status bar, mouse on, highlighting copies, and copying while scrolled back keeps
  your place.

`term.sh` finds `claude` and `tmux` on the PATH that `~/.profile` sets, in `~/.local/bin`, in Homebrew's folders
(`/opt/homebrew/bin`, `/usr/local/bin`) and under the newest Node that nvm installed. If yours live somewhere else, put that
folder on the PATH in `~/.profile`.
