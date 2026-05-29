---
title: Prompting
description: Write prompts that give the agent enough context to act safely.
---

Good Open Interpreter prompts are concrete. Include the observed problem, the
expected behavior, constraints, and verification commands.

## Bug Fix Template

```text
Bug: Clicking Save shows success but does not persist the setting.
Repro:
1. npm run dev
2. Open /settings
3. Toggle Enable alerts
4. Click Save
5. Refresh; the toggle resets

Constraints:
- Do not change the API shape.
- Keep the patch minimal.
- Add a regression test if practical.

Start by reproducing, then patch, then rerun the repro and tests.
```

## Better Than Vague Requests

Prefer "run `pnpm test -- auth` and fix the failing refresh-token test" over
"fix auth." Prefer "do not touch migrations" over assuming the agent knows that
constraint.

## Use Files

Mention files with `@` in the TUI or attach relevant files/images from the
command line. Keep context focused; too much irrelevant context makes the task
harder.
