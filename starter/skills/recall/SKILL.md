---
name: recall
description: Answer "what happened with X" or "why is X like this" from the project's log in {{dir}}/log, newest first, with links to the entries. Use before re-deriving history from memory or from the code.
---

# /recall

1. Search the log for the terms in the question: `grep -ril '<term>' {{dir}}/log | sort -r` (file names start with the date, so
   this lists the newest first). Try a second wording if nothing matches.
2. Read the newest two or three matching entries.
3. Answer from them, and name each entry you used with its path. Say which parts come from the log and which you inferred.
4. If the log has nothing, say so plainly ("not in the log") before falling back to git history or the code.
