---
title: Cyber Safety
description: Use Open Interpreter responsibly on security-sensitive work.
---

Open Interpreter can inspect and modify real systems. Keep security work scoped
to assets you own or are authorized to test.

## Practical Rules

- Use read-only mode for unfamiliar or sensitive repositories.
- Keep secrets out of prompts and repo-local hooks.
- Prefer explicit allowlists for network access.
- Review commands that touch auth, signing, infrastructure, or deployment.
- Do not ask the agent to exploit third-party systems.

For local command boundaries, see [Sandbox & approvals](/docs/sandbox) and
[Permissions](/docs/permissions).
