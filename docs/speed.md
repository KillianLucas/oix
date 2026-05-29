---
title: Speed
description: Make local sessions faster without hiding important checks.
---

Speed comes from model choice, context size, and tool setup.

## Fast Mode

Use `/fast` where supported by the active model/provider. For durable defaults:

```toml
[profiles.fast]
model_reasoning_effort = "low"
```

## Keep Context Tight

- Use `@` to mention only relevant files.
- Compact long sessions with `/compact`.
- Put durable repo rules in `AGENTS.md`, not every prompt.
- Put repeatable procedures in skills so they load only when needed.

## Tooling

Make build and test commands easy to run from the repository root. Document
them in `AGENTS.md`.
