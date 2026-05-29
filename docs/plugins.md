---
title: Plugins
description: Bundle skills, MCP servers, hooks, and related extension files.
---

Plugins package reusable Open Interpreter extensions. A plugin can include
skills, MCP server definitions, hooks, and other configuration that should move
together.

Plugins are experimental and may be disabled by default:

```toml
[features]
plugins = true
```

## Plugin Shape

```text
my-plugin/
├── .codex-plugin/
│   └── plugin.json
├── skills/
├── mcp/
└── hooks/
```

The manifest describes the plugin and the bundled extension points.

## Marketplaces

Codex-compatible plugin marketplace commands may be present in lower-level
tooling. In the public Open Interpreter launcher, prefer installed or local
plugins that are documented for the current release.

## Trust

Review plugin contents before enabling them. A plugin can bring code that runs
as hooks, MCP servers, or skill scripts, and those run through the normal trust,
sandbox, and approval controls.
