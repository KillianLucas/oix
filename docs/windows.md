---
title: Windows
description: Install and run Open Interpreter on Windows.
---

Install from PowerShell:

```powershell
irm https://openinterpreter.com/install.ps1 | iex
```

Restart the terminal, then verify:

```powershell
interpreter --version
```

## WSL

WSL is a good choice when your project already uses Linux tooling. Install with
the macOS/Linux command inside WSL:

```bash
curl -fsSL https://openinterpreter.com/install | sh
```

## Paths and Shells

Use the same shell and path style your project expects. Native Windows projects
should use Windows paths and PowerShell conventions. WSL projects should use
Linux paths and tools.

## Sandbox Notes

Native Windows sandboxing has different enforcement details than macOS and
Linux. If you need Linux-style sandbox behavior, use WSL. For trusted local
repos, start with the default permissions and loosen only when the task needs
it.
