---
title: Daemon
description: How the local Open Interpreter daemon starts, is reused, and is stopped.
---

When you run `interpreter`, the launcher talks to a local **app-server daemon**
rather than doing all the work in the foreground process. The daemon is what
holds the running runtime; the `interpreter` command you type is a thin client
that connects to it.

This is why the **first** launch after a reboot (or after the daemon has been
stopped) is a little slower — it has to start the daemon — while every launch
after that connects almost instantly.

## Startup behavior

- **Cold start** — no daemon running yet. The launcher starts one and briefly
  shows a `Starting up...` status line while it comes up. As soon as the daemon
  is ready the line is cleared and the interface takes over.
- **Warm start** — a healthy daemon is already running. The client connects
  immediately and **no startup line is shown**.

The daemon is shared across every `interpreter` invocation on the machine,
including `interpreter exec`. It stays running between sessions so repeated use
stays fast.

## Stopping the daemon

```bash
interpreter kill
```

This asks the daemon to shut down gracefully. If it does not exit on its own,
force it:

```bash
interpreter kill --force   # or: interpreter kill -f
```

Stopping the daemon is safe — the next `interpreter` run simply cold-starts a
new one.

## Where the daemon lives

The daemon's runtime files live under your Open Interpreter home directory,
which defaults to `~/.openinterpreter` and can be overridden with the
`INTERPRETER_HOME` environment variable:

| File | Path | Purpose |
| ---- | ---- | ------- |
| Lockfile | `<home>/tmp/interpreter/app-server.json` | Records the running daemon's PID and WebSocket URL. |
| Log | `<home>/log/interpreter-app-server.log` | Daemon output — start here when debugging. |

The launcher validates the lockfile against the live process and probes the
daemon's health endpoint before reusing it, so a stale lockfile left by a
crashed daemon is detected and replaced automatically on the next launch.

## Remote mode bypasses the local daemon

When you connect to a remote endpoint the local daemon is not involved:

```bash
interpreter --remote <ws-url>
```

See [Server deployments](/docs/remote) for details.

## Troubleshooting

- **Launches feel stuck on `Starting up...`** — check
  `<home>/log/interpreter-app-server.log` for startup errors.
- **Something is wedged after an update or crash** — run `interpreter kill`
  (add `--force` if needed) and relaunch to cold-start a fresh daemon.
