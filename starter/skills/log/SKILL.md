---
name: log
description: Record a finished piece of work as one log entry plus a headline in {{instructions}}. Use when a task is done, when the user wraps up, or when they say "log this".
---

# /log

Record this session's work as ONE entry.

1. Pick a short title that says what changed, in plain words (not "updates" or "fixes").
2. Write `{{dir}}/log/<today as YYYY-MM-DD>-<slug>.md`, where the slug is the title in lower case with dashes:

   ```markdown
   # <Title>

   **Date:** <YYYY-MM-DD>

   ## What changed
   <the files and behaviour that changed, briefly>

   ## Why
   <what was asked, and any decision made along the way with its reason>

   ## Evidence
   - ran: <command> → <what it printed>
   - read: <what the source showed>
   - not run: <what was not checked, and why>

   ## Open
   <what is left, or "nothing">
   ```

3. In `{{instructions}}`, add a headline at the top of the list under "Current state" and update "Last updated":
   `- <date> · **<Title>** · [log]({{dir}}/log/<file>.md)`. Keep about eight headlines; drop the oldest (its entry stays in the log).
4. Tick any roadmap box the work completed.
5. If the work settled a convention, offer to add it with `/rule`.

Never write the story of the session into `{{instructions}}` itself.
