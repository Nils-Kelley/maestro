# What Maestro reads

Maestro has no format of its own. This page lists every file it looks at, so you can tell why a screen is full or empty, and
make your own harness show up without adopting anyone else's.

## Where the harness is

For each project Maestro picks one harness folder:

- `.claude/` by default;
- `.agents/` when the project has that folder and `.claude/` holds no `rules/`, `skills/` or `log/` (a `.claude/` with only
  settings in it does not count).

The instructions file is `CLAUDE.md` if it exists, otherwise `AGENTS.md`. In an `.agents/` project `AGENTS.md` wins when both exist.

Below, `<harness>` is that folder and `<claude>` is Claude Code's own folder: `~/.claude` inside WSL, or `%USERPROFILE%\.claude`
on Windows.

## Files

| What | Where | Shape |
| --- | --- | --- |
| Instructions | `CLAUDE.md` or `AGENTS.md` in the project; `<claude>/CLAUDE.md` for yours | Any Markdown. Only the size is used. |
| Rules | `<harness>/rules/*.md` | Optional `---` header with a `paths:` list of globs. Under each `## ` heading, every line starting with `- ` is one rule; indented lines under it belong to it. |
| Skills | `<harness>/skills/<name>/SKILL.md`, `<claude>/skills/<name>/SKILL.md` | A `---` header with `name:` and `description:`, then the steps. |
| Memory | `<claude>/projects/<project>/memory/*.md` | One fact per file with a `---` header (`name:`, `description:`, `type:`); `MEMORY.md` is the index, one line per memory. |
| Hooks | `<claude>/settings.json`, `.claude/settings.json` | Claude Code's `hooks` object. Shown, not edited. |
| Work log | `<harness>/log/YYYY-MM-DD-<slug>.md` | The title is the first `# ` line. If `<harness>/log/INDEX.json` exists (`{ "entries": [{ "date", "title", "slug", "pending", "tags" }] }`) it is used instead of reading the files. |
| Audits | files with `audit` in the name in `docs/`, `.planning/`, `<harness>/reference/`; every `.md`/`.txt` in the folders listed as `audits` for the project in `maestro.config.json` | Any text. A date in the file name (`YYYY-MM-DD`) is used for sorting. |
| Earlier conversations | `<claude>/projects/<project>/*.jsonl` | Claude Code's own transcripts. Only the first thing asked is read, to offer "resume". |
| Rule load | `<harness>/maestro.json` | `{ "hot": ["path/one.ts", …], "limit": 120000 }` |

`<project>` is the project's path with every character that is not a letter or digit turned into a dash
(`/mnt/d/projects/my-site` becomes `-mnt-d-projects-my-site`, `D:\projects\my-site` becomes `D--projects-my-site`). That is
Claude Code's own naming.

## Rules, in detail

A rules file is read as: header, text before the first `## `, then sections of rules. Maestro changes one rule at a time and
writes the rest of the file back byte for byte, line endings included (there is a test for that). Before any change, the first
copy of the file that day is kept in `<harness>/archive/rules-ui-<date>/`.

```markdown
---
paths:
  - "src/payments/**"
---

# Payments

## Conventions
- **Amounts are integer cents**: floats lost a cent on refunds (log: refund-rounding)
- **Never retry a charge without its idempotency key**
```

The home screen's "given to the agent before it edits" adds up, for each file named in `hot`: the project's instructions, your
own `CLAUDE.md`, and every rules file whose `paths` match that file (a rules file without `paths` always counts).

## Tabs that appear only when there is something to show

Work (a log), Audits (at least one), and three that need a kit: Ship, Sessions and Checks. See [kit.md](kit.md).
