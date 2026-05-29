---
title: Workflows
description: Repeatable ways to use Open Interpreter for real development tasks.
---

## Fix a Bug

1. Start at the repository root.
2. Provide reproduction steps and constraints.
3. Ask Open Interpreter to reproduce before editing.
4. Review the patch.
5. Ask it to rerun the reproduction and project checks.

## Review a Diff

```bash
interpreter exec review --uncommitted
```

Or in the TUI:

```text
/review
```

Review output should prioritize bugs, regressions, missing tests, and risky
behavior.

## Refactor Safely

Ask for a plan first:

```text
/plan
Split the oversized parser module without changing public behavior.
```

Then execute in small stages and run tests between stages.

## Keep Docs Current

Point Open Interpreter at the changed files and ask it to update user-facing
docs. Keep private workspace details out of product docs.
