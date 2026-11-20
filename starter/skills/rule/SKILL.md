---
name: rule
description: Add ONE rule to the right file in {{dir}}/rules. Use when work established a convention, a trap or an architectural decision that the code alone does not show.
---

# /rule

1. Say the rule in one line: what to do (or never do) and why. If it cannot be said in one line it is documentation, not a rule.
2. Pick the file in `{{dir}}/rules/` whose `paths:` cover the code the rule is about. If none fits and this is the second rule
   for that area, create a file for the area with a `paths:` header:

   ```markdown
   ---
   paths:
     - "src/payments/**"
   ---

   # Payments

   ## Conventions
   - **<the rule>**: <the reason> (log: <slug of the entry it came from>)
   ```

3. Append the line under the `## ` section it belongs to. A rule starts with `- ` and stays on one line.
4. Do not add: what the code already shows, file locations, one-off fixes, or a dated story (that is a log entry: `/log`).
