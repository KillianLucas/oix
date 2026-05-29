---
title: Sessions
description: Resume, fork, rename, compact, and manage local conversation history.
---

Open Interpreter stores conversations locally under `~/.openinterpreter/` so
you can continue work later.

## Resume

Resume the newest session for the current directory:

```bash
interpreter resume --last
```

Open a picker:

```bash
interpreter resume
```

Include sessions from other directories:

```bash
interpreter resume --all
```

Resume a known id:

```bash
interpreter resume <SESSION_ID>
```

## Fork

Forking creates a new thread from an older session. The original stays intact.

```bash
interpreter fork --last
interpreter fork <SESSION_ID>
interpreter fork --all
```

Inside the TUI, use `/fork` or `/side`.

## Exec Sessions

Non-interactive sessions can also be resumed:

```bash
interpreter exec resume --last "continue with the implementation"
```

Use `--include-non-interactive` with interactive resume when you want exec
sessions to appear in the picker.

## Compact

Long conversations can be compacted into a shorter summary:

```text
/compact
```

Automatic compaction can also run when the active model is near its context
limit. Set `model_auto_compact_token_limit` in config if you need an explicit
threshold.

## History Controls

Disable saved transcript history:

```toml
[history]
persistence = "none"
```

Cap history size:

```toml
[history]
max_bytes = 104857600
```

## Daemon

The public `interpreter` launcher starts a local app-server-backed runtime for
interactive work, and reuses it across runs so subsequent launches are fast.
Stop it with:

```bash
interpreter kill
```

Use `--force` only when a normal stop does not return. See the
[Daemon](/docs/daemon) page for startup behavior, file locations, and
troubleshooting.
