# How this project keeps its context

These rules apply to every file. A rules file can instead start with a `paths:` list (in a `---` header) so that it is only
loaded for the files it is about; keep rules that concern one area in a file scoped to that area.

## The log
- **One entry per piece of work**: `{{dir}}/log/YYYY-MM-DD-short-slug.md`, starting with a `# Title` line that says what changed, then what was done, why, and how it was checked.
- **{{instructions}} carries headlines only**: one line per entry linking to its file, newest first. Never write a dated paragraph into {{instructions}}.
- **Search the log before re-deriving history**: `grep -ril '<term>' {{dir}}/log`, newest file first.

## Rules
- **A rule is one line starting with `- `** under a `## ` heading, with the reason in the same line. If it came out of an incident, end it with `(log: <slug>)`.
- **Write a rule only for what the code cannot tell you**: a decision and its reason, a convention that is not obvious, a trap someone fell into. No file locations, no one-off fixes.
- **A rule that no longer holds is removed**, not left with a note; the log keeps the history.

## Evidence
- **Say how each claim was established**: `ran:` (the command and what it printed), `read:` (what the source showed), `not run:` (why). An unrun check is never reported as passing.
- **Missing coverage is unknown, not absence**: an empty list or a quiet log proves nothing on its own.
