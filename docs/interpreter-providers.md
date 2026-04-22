# Open Interpreter Provider System

This branch is intentionally provider-first instead of OpenAI-first.

## Goals

The provider system should:

- work with OpenAI and non-OpenAI providers
- support both Responses-native and chat/completions-compatible backends
- feel usable offline by shipping bundled provider/model metadata
- become useful immediately on machines that already have provider state configured elsewhere

## Picker model

The shared provider/model flow is:

1. choose provider
2. choose model
3. choose reasoning effort when supported

Onboarding and `/model` should use the same underlying picker logic.

## Bundled catalogs

The branch ships bundled provider and compatibility metadata so the picker works quickly and
offline.

Main generated artifacts:

- [codex-rs/model-provider-info/provider_catalog.json](/Users/killianlucas/Documents/GitHub/open-interpreter-next/openinterpreter/codex-rs/model-provider-info/provider_catalog.json)
- [codex-rs/codex-api/model_compatibility_catalog.json](/Users/killianlucas/Documents/GitHub/open-interpreter-next/openinterpreter/codex-rs/codex-api/model_compatibility_catalog.json)

Generation scripts:

- [codex-rs/scripts/write_provider_catalog.py](/Users/killianlucas/Documents/GitHub/open-interpreter-next/openinterpreter/codex-rs/scripts/write_provider_catalog.py)
- [codex-rs/scripts/write_model_compatibility_catalog.py](/Users/killianlucas/Documents/GitHub/open-interpreter-next/openinterpreter/codex-rs/scripts/write_model_compatibility_catalog.py)

Current source strategy:

- provider catalog generated from `models.dev` metadata
- model compatibility catalog generated from LiteLLM-compatible metadata plus a small override file

Hard rule:

- provider/model membership must come from generated catalogs, not hand-written Rust lists
- if a provider is missing, fix the generator sources or override files rather than adding a
  provider-specific model list in product code
- `write_provider_catalog.py` and `write_model_compatibility_catalog.py` are critical system
  infrastructure, not optional convenience scripts
- generated provider metadata must also carry the correct `wire_api` contract for each provider,
  such as `responses`, `chat`, or Anthropic `messages`
- reasoning support, tool-calling eligibility, input modalities such as vision, and related picker
  capability metadata should come from those generated artifacts or live provider metadata, not
  hand-maintained Rust lists

The intent is to avoid manually curating giant provider/model lists inside the app.

## Wire API vs harness

These are separate concerns:

- `wire_api` means the provider's actual HTTP protocol family, such as `responses`, `chat`, or
  `messages`
- `harness` means the emulated request semantics on top of that protocol, including tool surface,
  tool prompts, flags, and session behavior

That separation matters for future harnesses. A new harness may still reuse `responses` or `chat`,
and Anthropic-style `messages` providers should not be mislabeled as chat-compatible just to fit an
OpenAI transport path.

## Compatibility transport

Open Interpreter stays Responses-native internally, but supports chat/completions-compatible
providers through the shared compatibility layer in:

- [codex-rs/chat-wire-compat](/Users/killianlucas/Documents/GitHub/open-interpreter-next/openinterpreter/codex-rs/chat-wire-compat)

That lets the rest of the app stay on one main execution path while still talking to providers that
offer OpenAI-compatible chat/completions endpoints.

Anthropic-style providers are not OpenAI chat-compatible. Their generated provider metadata should
set `wire_api = "messages"`, and the runtime should pair that with a harness-native transport such
as `claude-code` rather than routing them through the chat-compat proxy.

## Compatibility-first import

On startup, Interpreter tries to discover usable local provider state so the first picker is
already populated with options that work on this machine.

Main sources:

- `~/.codex/config.toml`
- `~/.codex/auth.json`
- env-backed provider keys already in the shell
- OpenCode auth/config when available
- local-provider installs such as Ollama

This logic lives in:

- [codex-rs/server-cli/src/system_import.rs](/Users/killianlucas/Documents/GitHub/open-interpreter-next/openinterpreter/codex-rs/server-cli/src/system_import.rs)

On a fresh `~/.openinterpreter`, that import must not skip provider onboarding.
Imported state is supposed to make onboarding immediately useful, not to create a
hidden current provider before the user has chosen one in Open Interpreter.

## Readiness labels

Providers that already work locally should be promoted near the top of the picker and subtly
labeled:

- `Logged in`
- `Ready`
- `Installed`

On a fresh imported home, those readiness labels are the important signal. The
picker should not mark a provider as `(current)` until the user actually selects
it inside Open Interpreter.

That readiness logic is surfaced through:

- [codex-rs/tui/src/provider_readiness.rs](/Users/killianlucas/Documents/GitHub/open-interpreter-next/openinterpreter/codex-rs/tui/src/provider_readiness.rs)

## Auth modes

OpenAI is intentionally split into distinct user-facing choices:

- OpenAI (ChatGPT sign-in)
- OpenAI (API key)

They share the same model catalog but represent different setup/auth paths for users.

## Local providers

Local providers such as Ollama should behave off live runtime state:

- if the local service is running, show the live model list
- if it is not running, show a compact unavailable/start/manual path

## What still matters

Bundled metadata is only a fast starting point. Real live provider refresh still matters:

- live `/models` should replace stale seeded models
- dead or removed upstream models should disappear after refresh
- manual custom model entry should remain as an escape hatch

The intended UX is:

- fast offline/provider-first picker
- real live provider metadata when available
- clear fallback to custom endpoint or custom model only when needed
