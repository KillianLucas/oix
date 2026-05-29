---
title: Auto-Review
description: Let a reviewer agent evaluate eligible approval requests.
---

Auto-review can delegate eligible approval prompts to a reviewer agent instead
of always asking the user directly.

```toml
approvals_reviewer = "auto_review"
```

Auto-review does not remove the sandbox. It changes who evaluates eligible
approval prompts under the active policy. Use it only when the reviewer policy
and sandbox are narrow enough for the repository.

For code review, use `/review` or `interpreter exec review`.
