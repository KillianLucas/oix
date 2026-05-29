---
title: MCP Server
description: Expose Open Interpreter as a tool to another MCP client.
---

Open Interpreter can run as an MCP server over stdio. This lets another MCP
client start and continue Open Interpreter sessions as tools.

```bash
interpreter mcp-server
```

The server exposes tools for starting a session and continuing an existing
thread. Tool parameters mirror the normal Open Interpreter configuration
surface: prompt, working directory, model, profile, sandbox, approval policy,
and per-run config overrides.

## When To Use It

Use MCP server mode when another agent framework or internal tool should call
Open Interpreter as a coding agent.

Use `interpreter exec` when you only need a single command-line job, and use
the [SDK](/docs/sdk) when you are embedding Open Interpreter directly in a
Python or TypeScript application.

## Client Configuration

An MCP client usually needs a stdio command entry similar to:

```json
{
  "mcpServers": {
    "open-interpreter": {
      "command": "interpreter",
      "args": ["mcp-server"]
    }
  }
}
```

Keep the MCP client and Open Interpreter in the same trust boundary you would
use for the CLI. The called session can still run tools, request approvals, and
modify the workspace according to the sandbox and permissions you configure.
