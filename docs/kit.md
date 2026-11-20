# Kits: Ship, Sessions and Checks

Three screens are about things most harnesses do not have: a ledger of what has reached production, a draft per agent session,
and a record of which checks have passed since the files they guard last changed. Maestro shows them when a project keeps those
files, and stays out of the way when it does not.

Maestro does not ship a kit. It reads a few files and, for the actions that change them, calls one script that you name as
`kit` for the project in `maestro.config.json`. Anything that writes those files and answers those calls will do. This page is
the contract.

The kit runs inside WSL, in the project's folder:

```
wsl.exe -d <distro> --cd <wslRoot> --exec bash <kit> <arguments…>
```

## Files Maestro reads

| File | Used for | Shape |
| --- | --- | --- |
| `<harness>/state/deploy.md` | counts by status on the home screen | a Markdown table; rows start with `| L` and the ninth cell starts with the status word (`uncommitted`, `committed`, `pushed`, `deployed`, `shipped`, `verified`, `superseded`, `n/a`) |
| `<harness>/state/prod-heads.json` | which repositories the project spans (also the Git tab) | `{ "repos": { "<name>": { "path": "/abs/path", "head": "<commit or empty>" } } }` |
| `<harness>/state/sessions/<id>.md` | the Sessions tab | a `status: live` (or `ended…`, `abandoned…`) line in the first three lines, a `last update: <ISO time>` line, and the first request on the line after `## What the user asked` |
| `scripts/probes.json` | the Checks tab | `{ "probes": [{ "name", "area", "last_pass", "paths": [] }] }` |
| `<harness>/state/probes-audit.json` | the Checks summary | `{ "registered", "current", "stale", "never_run", "at" }` |
| `<harness>/log/INDEX.json` | the Work tab, instead of reading each file | see [harness.md](harness.md) |

## Calls Maestro makes

| Arguments | When | Expected |
| --- | --- | --- |
| `ui --ledger-json` | opening Ship | JSON on stdout: `{ "open", "total", "why": { "<reason>": n }, "repos": [{ "name", "head", "at", "local", "stuck" }], "rows": [{ "id", "date", "what", "repo", "commits", "status", "why" }] }` |
| `deploy --mark <repo> <commit> --by Maestro` | "Mark as live", after a typed commit id and a second click | record that production runs that commit; print one or two lines saying what closed |
| `deploy --close` | "Close these" | close the rows that are ready; print a line |
| `probes audit` | "Run the audit" | plain text; a block starting with `pass stale` whose indented lines read `name: reason` |

Text before the first `{` of the ledger answer is ignored, so a kit may print a banner. A non-zero exit with nothing on stdout
is shown as an error with the kit's stderr.
