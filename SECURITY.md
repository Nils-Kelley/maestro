# Security

## Reporting

Please report a vulnerability privately, through GitHub's "Report a vulnerability" button on the
[Security tab](https://github.com/Nils-Kelley/maestro/security), not in a public issue. Say what you found, how to reproduce it,
and which version or commit. You will get an answer within a week.

## What is in scope

Maestro reads and writes files in the projects you add, runs `git` and `wsl.exe`, and starts terminals. Things worth reporting:

- a way for the content of a project (a file name, a rule, a log entry, terminal output, a commit message) to make the app run
  a command, write outside the project's harness folder, or read a file it should not;
- a way for the page to reach a path that the Rust side should have refused;
- a way to make the updater fetch or run anything other than this repository's release build;
- with remote access on: a way to be answered from outside your Tailscale network, to do anything without being a paired
  device, to keep working after a device is removed, or to reach anything other than sessions from the remote page;
- a secret shown on screen that the app should have cut (the first request of a session is shown in lists; pasted logins are
  removed from it).

## What Maestro does not do

It has no telemetry. Nothing listens on the network unless you turn remote access on ([docs/remote.md](docs/remote.md)): a
paired device can type into your sessions, so pair only your own. Its only network request is the update check (GitHub's public releases API), made when you ask, or at
start if you turned that on; installing an update downloads the build from this repository's releases over HTTPS. Builds are not
signed. It does not store credentials. Sessions run with your own
permissions, exactly as if you had opened a terminal; what Claude Code may do inside a session is governed by Claude Code's own
settings, not by Maestro.
