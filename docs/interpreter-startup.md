# Open Interpreter Startup Direction

Startup is still important, but this repo is no longer treating "fake instant startup from a tiny
launcher" as the product requirement.

## Current rule

The public `interpreter` binary should stay thin and hand off quickly into the real TUI.

From there:

- the real TUI owns onboarding
- the real TUI owns trust prompts
- the real TUI owns loading and session bootstrap states
- the daemon/app-server connection can happen as deferred work inside that app

## What is acceptable

The startup flow may show a truthful loading state inside the real TUI while heavier work happens.

That can include:

- starting or reusing the daemon
- connecting to the local app-server
- loading full config
- bootstrapping a session

## What is not acceptable

We do not want startup to rely on a separate launcher UI that pretends to be the real app.

That means no:

- fake preview screen drawn by the launcher
- launcher-owned startup shell that gets replaced by the real chat UI
- terminal-clearing handoff tricks whose only purpose is to hide that replacement
- cursor/viewport repair logic dedicated to papering over the swap

## Why

That approach turned out to be the wrong architecture for the product constraints:

- same visible TUI experience as upstream
- shared daemon-backed runtime
- less client memory
- no embarrassing viewport jank

The prepaint/handoff design fought those constraints instead of supporting them.

## Testing guidance

Startup tests should now focus on the behaviors that still matter:

- the installed `interpreter` path reaches the expected real UI
- first-run launches reach the provider picker
- trusted configured launches reach the normal chat session
- interpreter-home resolution beats stale `CODEX_HOME`
- daemon reuse still works across tabs and workdirs

Tests should not enforce a fake `<50ms` launcher contract or require the first visible state to
appear before daemon work begins.

## If fast startup is revisited later

Revisit it only if the real app can do it honestly, ideally by being the small program itself and
lazy-loading heavier systems after drawing the actual interface.

Do not reintroduce a separate launcher-owned pseudo-interface.
