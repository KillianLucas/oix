# Open Interpreter Architecture

This branch keeps the upstream terminal UI substrate, but changes the runtime shape under it.

## Product shape

The intended product contract is:

- one user-facing command: `interpreter`
- one hidden local daemon shared across tabs
- one upstream-style TUI client attached to that daemon
- broader provider compatibility than upstream Codex
- a provider-first onboarding and `/model` flow

Users should not need to think about the daemon as a separate product. It is an implementation
detail managed by `interpreter`.

## Process model

The process split is:

- `interpreter`
  - public entrypoint
  - resolves the interpreter home
  - hands off into the real TUI startup path
- `interpreter-app-server`
  - hidden local daemon
  - owns durable thread/session state
  - serves the app-server protocol over a local websocket
- `interpreter-root-tui` / `interpreter-tui`
  - hidden worker binaries
  - run the upstream-style TUI against the local app-server

The daemon contract is:

- `0` tabs: no daemon
- `1+` tabs: one daemon
- when the last tab exits, the daemon shuts down shortly after

## Why this exists

The point of this branch is not to invent a second terminal UI. It is to keep the mature upstream
interaction model while changing the architecture underneath it:

- reuse one loaded backend across many tabs
- keep the visible client process thinner
- support more providers and compatible gateways
- preserve resume/fork/subagent flows

## State ownership

Today, state is split like this:

- daemon:
  - durable session/thread state
  - app-server protocol state
  - model/provider execution state
- TUI client:
  - active local UI state
  - composer state
  - popup and picker state
  - some transcript/render state inherited from upstream

The long-term target is stricter:

- daemon owns all durable transcript state
- client owns only ephemeral live UI state
- finalized transcript is flushed into terminal scrollback and dropped locally

That stricter split is the next major RAM-saving step.

## Homes and config

Open Interpreter resolves its own home independently from Codex:

- `INTERPRETER_HOME`
- `OPEN_INTERPRETER_HOME` as a compatibility alias
- otherwise `~/.openinterpreter`

An incoming shell `CODEX_HOME` must not choose the interpreter home. After the interpreter home is
resolved, the process may set `CODEX_HOME` internally so reused upstream crates continue to work.

## Startup model

Startup should be owned by the real TUI, not by a separate launcher UI.

That means:

- `interpreter` should stay thin
- the real TUI should render onboarding, trust prompts, loading states, and chat
- daemon connection and session bootstrap can happen as deferred work inside that real app
- truthful loading states are acceptable when the real app is still starting

What we explicitly do not want is:

- a launcher-owned fake preview
- terminal-clearing handoff tricks
- a temporary shell that gets swapped out for the real chat UI
- cursor/viewport preservation logic whose only purpose is to hide that swap

If we ever revisit ultra-fast startup, it should be built on a single real program that can
lazy-load heavier systems after drawing the actual interface, not on a multi-stage terminal
handoff.
