---
title: Migrate
description: Bring Codex or other compatible agent setup into Open Interpreter.
---

Open Interpreter can reuse much of the Codex-style local setup because the CLI
surface and configuration model are closely related.

## What Usually Migrates

| Source item | Open Interpreter destination |
| ----------- | ---------------------------- |
| Instructions | `AGENTS.md` |
| Config | `~/.openinterpreter/config.toml` or `.openinterpreter/config.toml` |
| Skills | `.agents/skills/` or `~/.agents/skills/` |
| MCP config | `[mcp_servers]` |
| Hooks | `hooks.json` or inline `[hooks]` |
| Slash-command workflows | Skills or project instructions |
| Subagents | `[agents]` config |
| Recent sessions | Local session history where supported |

## Review After Import

Review migrated setup before relying on it:

- MCP servers with custom auth, headers, or transports
- Hooks that run local commands
- Skill scripts and references
- Agent permissions and tool restrictions
- Prompt templates that depend on shell interpolation or path placeholders

## Codex Home

Open Interpreter uses `~/.openinterpreter/` for its user state. If you previously
used Codex, inspect both homes during migration:

```text
~/.codex/
~/.openinterpreter/
```

Do not copy secrets blindly. Prefer re-authenticating or using environment
variables.
