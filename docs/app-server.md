---
title: App Server
description: Embed Open Interpreter with the local app-server protocol.
---

Open Interpreter uses an app-server-backed runtime for the interactive TUI. The
same app-server protocol is available for advanced integrations that need
threads, streamed events, approvals, session state, model selection, and MCP
status inside another application.

Most automation should start with [Non-interactive mode](/docs/exec) or the
[SDK](/docs/sdk). Use the app server when you are building a richer client.
Use [Agent Client Protocol](/docs/acp) instead when you want an ACP-compatible
editor or UI to launch Open Interpreter as its coding agent.

## Start a Local Server

Use stdio for an SDK-style child process:

```bash
interpreter-app-server --listen stdio://
```

Use WebSocket transport when a separate client needs to connect:

```bash
interpreter-app-server --listen ws://127.0.0.1:9000
```

Then connect a TUI client:

```bash
interpreter --remote ws://127.0.0.1:9000
```

## Secure Remote Access

If the server is not strictly local, terminate TLS in front of it and require a
bearer token:

```bash
interpreter --remote wss://agent.example.com \
  --remote-auth-token-env INTERPRETER_REMOTE_TOKEN
```

Store the token in the named environment variable on the client. Do not expose
an unauthenticated app-server listener on a public network.

## Protocol

The protocol is inherited from Codex app-server. Open Interpreter keeps that
surface compatible so existing Codex app-server clients can use
`interpreter-app-server` as the launched process.

Use OpenAI's [Codex app-server docs](https://developers.openai.com/codex/app-server)
for the full protocol shape, then apply the Open Interpreter-specific launch
commands above.
