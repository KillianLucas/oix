---
title: Web Search
description: Control cached, live, or disabled web search behavior.
---

Web search lets the agent consult external information when the active provider
and policy allow it.

## Modes

```toml
web_search = "cached"   # default-style search behavior
web_search = "live"     # request live browsing
web_search = "disabled" # no web search tool
```

Use live search for one run:

```bash
interpreter --search "check the current release notes and summarize changes"
```

## Network Sandbox Is Separate

`web_search` controls the model/tool search surface. It is separate from shell
command network access. A sandboxed `curl` or package manager still needs
network permission through sandbox or permission-profile settings.

## When to Disable

Disable web search for sensitive repositories, offline environments, or tasks
where every answer should come only from local files and configured tools.
