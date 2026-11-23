# Remote access

Your running sessions from a phone or another computer: what each one is doing, a line to answer it, and the session itself
as a full terminal. It is off unless you turn it on.

## Set it up

1. Install [Tailscale](https://tailscale.com/download) on the computer that runs Maestro and on each device you want to use,
   and sign in to the same account on all of them. Tailscale joins your devices into a private network of your own that works
   from any connection: home, mobile data, a hotel.
2. On Maestro's home screen, under **On your phone**, press **Turn it on** (or type `>` and choose **Remote access**).
3. Press **Pair a phone**. It shows an address and six digits: open the address on the phone and type the digits. Or scan
   the picture with the phone's camera, or copy the link and open it on the other computer. A code pairs one device and
   works for ten minutes; make another for the next device.
4. On a phone, add the page to the home screen. It then opens like an app.

A page that is not paired (the one on the home screen, after its device was removed) asks for the six digits, and can scan
the picture instead: at the secure address it uses the camera itself, at the plain one it asks for a photo.

Maestro has to be running on the computer; hidden in the tray is fine. On Windows the first start may ask whether Maestro may
accept connections: allow it for **private** networks, which is how Windows classes the Tailscale adapter. If a device then
cannot connect, look for another firewall or security product on the computer that filters incoming connections.

## What you can do

- **See every session as a card**: working, waiting for you or idle, with what it is doing or the last lines on its screen.
  Sessions that wait for you come first, and the page says so when one starts waiting.
- **Answer with one tap.** When Claude Code asks before it does something, its answers are buttons, on the card and above the
  keyboard inside the session.
- **Reply without opening the session**: a card's reply line slides up a sheet to type in.
- **Open a session as a full terminal.** On a phone there is a row of keys (`esc`, `tab`, `ctrl`, arrows, page up and down) and
  a message line that uses the phone's own keyboard, dictation included. A message of several lines goes in as one paste. A
  finger moved over the terminal scrolls it. The `···` button sets the text size and ends the session.
- **Read a session in detail.** Beside Chat, Detail shows every step with all it was given (the command, the edit) and what
  came back, as it happens. It is read from the record Claude Code keeps, so the session is not slowed or changed by being
  watched. Reasoning is shown where that record holds it; for most models it holds none.
- **See what a session has changed.** A session has a Git view: the changed files of the repo it works in, and each file's
  diff, kept up to date while it works. The line to write to the session stays under it.
- **A terminal of your own beside it.** On a Claude Code session, Terminal is a shell in the same project, yours to run
  things in; it is started the first time and kept. Claude Code's own screen is under `···`.
- **Start a session**: Claude Code with a first message, or a shell, in any project that keeps its sessions running.
- **Look at the projects.** Projects lists every repo with its branch, how many files have changed and which sessions are at
  work in it. A repo opens to its changed files and last commits, and a file to its diff, which follows the file as a session
  edits it. Nothing is staged, committed or pushed from here.
- **Watch a session's browser.** When a session drives a browser (Claude Code through the Playwright tool, say), the session
  has a third view, Browser: a picture of the page it is on, sent again each time the page changes. Pinch or double-tap to
  enlarge it; when the browser has several pages open, choose which to look at. It only shows: nothing is tapped or typed
  into the page from here. It needs one setting, below.

### The browser view

A session's browser can be watched when its Chrome was started with a debugging port. For the Playwright tool
(`@playwright/mcp`), Maestro keeps a settings file that asks for one, on a port Chrome chooses, and the tool is pointed at it
with `--config`:

```
claude mcp add -s user playwright -- playwright-mcp --config ~/.local/share/maestro/playwright.json
```

(with the options you already give it; a `~` is spelled out by your shell). It applies to sessions started afterwards.
Maestro finds the browser among the programs a session started, so nothing is set per session. Another program's Chrome is
found the same way, when it is started with `--remote-debugging-port`.

The port answers only on the computer itself, not on the network. While such a browser runs, a program on the computer could
connect to it and see or drive its pages, as Maestro does to show them: use it for the browsers your agents work in, not for
one you are signed in to your accounts with.

On a wide screen the list and the open session sit side by side, and Enter sends.

## Limits

- Only sessions that outlive the app are reachable: on macOS and Linux all of them, on Windows the ones inside WSL. PowerShell
  sessions and Claude Code started on Windows itself are not listed.
- A session opened on a small screen is drawn at that size on the computer too, until the computer's window is used again.
  That is how tmux shares one session between two screens.
- The harness, Git and the other screens are not part of the remote page.
- The browser view shows Chrome and its relatives (Edge, Chromium). A browser running on Windows itself is seen only for
  sessions that run there too, and those are not reachable from the remote page.

## What it opens, and to whom

Turned on, Maestro listens on one port (4747, or `remote.port` in [`maestro.config.json`](configuration.md)).

- **Only Tailscale is answered.** A connection is answered if it arrived on this computer's own Tailscale address (the one
  `tailscale ip -4` reports) from an address of the Tailscale range, or if it comes from the computer itself. Anything that
  arrives over another interface, the home network included, is dropped without a reply.
- **A device pairs once.** The pairing code is 128 random bits, works for one device, runs out after ten minutes and is never
  written to disk. The link carries it after the `#`, which a browser does not send to a server.
- **Each device has a token of its own.** Pairing gives the device a random token, kept in that browser's storage. The
  computer keeps only the token's SHA-256, so nothing in `maestro.config.json` or its `.bak` lets anyone in.
- **A device can be removed.** The Remote access screen lists the paired devices and which are connected now. **Remove** stops
  a device's token and cuts its connection at once. A device that has not connected for 30 days has to pair again.
- **You can see it.** While a device is connected, the top of the Maestro window says so. Every pairing, connection, refusal
  and removal is written to `maestro.log` with the time and the address.
- **Guessing is shut out.** After five wrong codes from one address, that address is not answered for five minutes. The six
  digits are a short form of the same code, so they stand for five tries in all: after the fifth wrong one the code is gone
  and a new one has to be made.

A paired device can type into your sessions and start new ones. That is as much as sitting at the computer, so pair only
devices that are yours, keep a screen lock on them, and remove one that is lost. The page itself cannot read or change a
project's files; a session can, as it always could.

The connection is plain HTTP inside Tailscale, which encrypts everything between your devices. Maestro adds no encryption of
its own, which is why it answers nothing else. Your Tailscale account is the front door: protect it with two-step sign-in.

## Without the window

`maestro --remote` serves the remote page and nothing else, until it is stopped. It prints a pairing link for one device,
good for ten minutes; start it again to pair another. Use it on a machine you leave running.
