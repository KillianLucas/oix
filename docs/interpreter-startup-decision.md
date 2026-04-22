# Startup Decision

This branch is preserving the daemon-backed architecture and the TUI/UI work, but it is backing
away from the fake ultra-fast startup experiment.

## Decision

Open Interpreter should start through the real TUI, even if that means showing a truthful loading
state while the daemon and session come up.

We are intentionally not pursuing the previous approach where a tiny launcher tried to:

- decide the exact first screen ahead of time
- draw that screen before the real app was ready
- preserve cursor and viewport state across a later handoff

## Why this changed

That design created the wrong kind of complexity:

- the launcher became a second UI owner
- terminal clearing and redraw bugs were hard to eliminate completely
- cursor and viewport correctness depended on fragile handoff logic
- small startup mismatches looked embarrassing in the actual product

The branch goal is still valid:

- one public `interpreter` command
- one hidden local daemon shared across tabs
- the upstream-style TUI preserved as the main interaction model
- a thinner client with better memory characteristics

But the launcher-first prepaint strategy was not helping those goals.

## What we are keeping

- the daemon/app-server architecture
- the provider-first onboarding and model-selection work
- the TUI UI changes and upstream-style interaction model
- daemon reuse across tabs and workdirs

## What we are dropping

- the strict fake-instant launcher startup contract
- launcher-owned preview/startup shells
- startup-specific cursor and viewport handoff tricks
- tests that only make sense for that discarded contract

## New startup policy

- `interpreter` stays thin and hands off quickly to the real TUI
- the real TUI owns onboarding, trust prompts, loading states, and session startup
- truthful loading UI is acceptable
- a separate launcher pseudo-interface is not

## When to revisit fast startup

Only revisit it if the real app can do it honestly, ideally with one real program that lazy-loads
heavier systems after drawing the actual interface.

Do not reintroduce a second startup UI that needs to be swapped out under the user.
