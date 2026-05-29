# Plan: `interpreter*` provider / model / harness app-server API

- Issue: https://github.com/KillianLucas/oix/issues/30 ("oix providers subcommand / app-server methods")
- Status: PROPOSED (awaiting approval before edits)
- Date: 2026-05-29
- Scope decided with issue author: app-server methods only (no CLI subcommand), camelCase method names, fork-owned module, lists + selection (no change-events yet).

## Goal

Expose the "pick a provider, pick a model, pick a harness" flow over the app-server (JSON-RPC) so a GUI/SDK can drive it non-interactively. Today this flow only exists interactively in the TUI (`/model`, `/harness`).

## Background: how the app-server works

### Inbound (listen + dispatch)

```
                          client (GUI / SDK / CLI)
                                  | JSON-RPC
                                  v
        transport: stdio | websocket | in-process     lib.rs::run_main_with_transport
          read loop: bytes -> JSONRPCRequest/Notification
                                  v
        MessageProcessor::process_request              message_processor.rs:362
          serde_json: raw JSON -> typed `ClientRequest` enum
          (the "method" string selects the variant, e.g. "model/list")
                                  v
        dispatch match on ClientRequest               message_processor.rs:807
          |-- fs/* , config/* , device/key/*  -> handled INLINE
          \-- everything else  -> CodexMessageProcessor::process_request
                                       codex_message_processor.rs
                                     match ClientRequest {
                                       ModelList => list_models()   :5653 / arm :1130
                                       TurnStart => turn_start()     :7127
                                     }
                                  v
        OutgoingMessageSender.send_response / send_error    outgoing_message.rs
                                  |
                       transport -+-> client (request resolved)
```

Method strings + variants are macro-generated in `protocol/common.rs` (`client_request_definitions!` at `:80`; e.g. `ModelList => "model/list"` at `:495`). Wire types live in `protocol/v2.rs`.

### Execution + event streaming (turn/start)

```
  turn/start request -> turn_start()                   codex_message_processor.rs:7127
        validate input, load_thread, build turn
        start the turn on codex core (agent loop begins)
        send_response(TurnStartResponse{ turn })  ->  client  (~:7307 early ack)
        :
        : turn runs asynchronously on codex core
        v
   codex core: model calls, tool/command exec, file edits, reasoning, ...
        emits a stream of `EventMsg`
        v
   per-thread listener task (subscribed via ThreadWatchManager on
   thread/start | thread/resume | subscribe)
        v
   apply_bespoke_event_handling(EventMsg)              bespoke_event_handling.rs:173
        match EventMsg -> ServerNotification:
          TurnStarted        -> "turn/started"
          ItemStarted        -> "item/started"
          CommandExecOutput  -> "item/commandExecution/outputDelta"
          TurnComplete       -> "turn/completed"
        v
   OutgoingMessageSender.send_server_notification(..) -> transport -> client (streamed)

  Blocking sub-flow (approvals / input):
   core needs exec/patch approval
        -> ServerRequest "item/commandExecution/requestApproval" -> client
                            client replies (allow/deny) -> resolves -> core resumes
```

Four wire shapes: client->server **requests** (resolved by a response) and **notifications**; server->client **notifications** (streamed events) and **server requests** (approvals/elicitations that pause execution).

## Out of scope (tracked separately): model/list custom-provider gate

`model/list` does NOT fetch a custom provider's live `/models` for the common case. The live fetch is gated (`models-manager/src/manager.rs:332`):

```rust
async fn should_refresh_models(&self) -> bool {
    self.endpoint_client.uses_codex_backend().await || self.endpoint_client.has_command_auth()
}
```

- `uses_codex_backend()` = active auth is the codex/ChatGPT backend.
- `has_command_auth()` = `provider.auth.is_some()`, i.e. a command-backed bearer-token block (`model-provider-info/src/lib.rs:535`). A plain `env_key` API key does NOT set this.

So a plain API-key custom provider (`env_key` set, `auth = None`, not codex backend) has the gate FALSE and never calls the provider `/models`. It returns the seed list instead:
- provider matches `provider_catalog.json` -> its static, baked-in models (stale, never refreshed);
- no match -> falls back to bundled `models.json` = codex's `gpt-5.x` / `codex-*` models (wrong models).

Evidence: `app-server/src/models.rs:208` (`supported_models_for_provider_allows_public_catalog_without_provider_env_key`) sets `auth = None` and asserts `requests.is_empty()` ("without a live /models request"). The Anthropic model it returns comes from the bundled catalog, not the live endpoint.

Decision: leave as-is for this issue. `interpreterModel/list` reuses `supported_models_for_provider`, so it inherits this behavior. File the gate fix as its own issue.

## Decisions locked

- Deliverable: app-server methods only. No CLI subcommand.
- Naming: camelCase segments split by `/` (matches `mcpServerStatus/list`, `collaborationMode/list`).
- First cut: 3 list methods + 3 set (selection) methods. No `*/changed` events yet.
- `interpreterProvider/list` returns ALL known providers (configured + bundled catalog), each flagged.
- Process: written plan (this file) approved before edits.

## API: 6 methods

| Method | Params | Response | Backed by |
|---|---|---|---|
| `interpreterProvider/list` | `{ includeUnconfigured?: bool }` | `{ data: InterpreterProvider[] }` | `config.model_providers` union `bundled_provider_catalog()` |
| `interpreterModel/list` | `{ modelProvider?: string, includeHidden?: bool }` | `{ data: Model[] }` | reuse `supported_models_for_provider` (app-server/src/models.rs) |
| `interpreterHarness/list` | `{ providerId: string, model?: string }` | `{ data: InterpreterHarness[] }` | extracted `harness_choices_for_provider_model` |
| `interpreterProvider/set` | `{ providerId: string, profile?: string }` | `{}` | `ConfigEditsBuilder::set_model_provider` |
| `interpreterModel/set` | `{ model: string, reasoningEffort?: ReasoningEffort, profile?: string }` | `{}` | `ConfigEditsBuilder::set_model` |
| `interpreterHarness/set` | `{ harness?: string, profile?: string }` | `{}` | `ConfigEditsBuilder::set_harness` |

### Types (in the fork-owned module)

```rust
// InterpreterProvider: union of configured providers + bundled catalog.
//   configured = present in config.model_providers
//   is_default = id == config.model_provider_id
pub struct InterpreterProvider {
    pub id: String,
    pub name: String,
    pub base_url: Option<String>,
    pub wire_api: WireApiDto,       // mirror of codex_model_provider_info::WireApi
    pub env_key: Option<String>,
    pub configured: bool,
    pub is_default: bool,
}

// InterpreterHarness: mirrors HarnessChoice. id = None means native/Codex harness.
pub struct InterpreterHarness {
    pub id: Option<String>,         // e.g. "claude-code", "kimi-cli"; None = native
    pub label: String,              // e.g. "Claude Code (recommended)"
    pub description: String,
    pub is_recommended: bool,
}
```

All types derive `Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS` with `#[serde(rename_all = "camelCase")]` and `#[ts(export_to = "v2/")]`.

`interpreterModel/list` reuses the existing `Model` type (`protocol/v2.rs:2339`).

Harness source enum: `tools/src/harness.rs` (`Native`, `ClaudeCode`, `ClaudeCodeBare`, `DeepSeekTui`, `KimiCode`, `KimiCli`, `LittleCoder`, `MiniSweAgent`, `OpenCode`, `Pi`, `QwenCode`, `SweAgent`, `Terminus2`, `Minimal`, `Other`). Compatible set per provider derived from `WireApi` (Messages -> claude-code variants; Chat -> full list; Responses -> native only) by `harness_choices_for_provider_model`.

## File changes

### New (fork-owned)

1. `codex-rs/app-server-protocol/src/protocol/interpreter.rs`
   - The single "file we own and export from": all 6 params + 6 response structs, plus `InterpreterProvider`, `InterpreterHarness`, `WireApiDto`.
   - Re-exported via `protocol/mod.rs` and `lib.rs` so consumers import from one path.

2. `codex-rs/app-server/src/interpreter_api.rs`
   - Handler fns, mirrors `fs_api.rs` / `config_api.rs` / `device_key_api.rs`:
     `list_providers`, `list_models`, `list_harnesses`, `set_provider`, `set_model`, `set_harness`.
   - Set handlers follow the established pattern at `codex_message_processor.rs:7098`:
     ```rust
     ConfigEditsBuilder::new(&config.codex_home)
         .with_profile(profile.as_deref())
         .set_model_provider(&provider_id)   // or .set_model(..) / .set_harness(..)
         .apply().await
     ```

### Modified (minimal upstream touch)

3. `codex-rs/app-server-protocol/src/protocol/common.rs`
   - Add 6 entries to the `client_request_definitions!` invocation (`:80`), referencing `interpreter::*`. Each ~4 lines:
     ```rust
     InterpreterProviderList => "interpreterProvider/list" {
         params: crate::protocol::interpreter::InterpreterProviderListParams,
         response: crate::protocol::interpreter::InterpreterProviderListResponse,
     },
     // ... InterpreterModelList, InterpreterHarnessList,
     //     InterpreterProviderSet, InterpreterModelSet, InterpreterHarnessSet
     ```

4. `codex-rs/app-server-protocol/src/protocol/mod.rs` + `src/lib.rs`
   - `pub mod interpreter;` and `pub use protocol::interpreter::*;` (single export point).

5. `codex-rs/app-server/src/codex_message_processor.rs`
   - 6 dispatch arms beside `ClientRequest::ModelList` (`:1130`), delegating to `interpreter_api`.

6. `codex-rs/app-server/src/lib.rs`
   - `mod interpreter_api;`

### Shared extraction (one source of truth)

7. `codex-rs/model-provider-info/src/harness_selection.rs` (new module in this crate)
   - Move `harness_choices_for_provider_model`, `harness_choice`, and the `HarnessChoice` struct out of `tui/src/onboarding/model_selection.rs`.
   - Pure lift: every dependency (`WireApi`, `ModelProviderInfo`, `bundled_provider_catalog`, `bundled_provider_catalog_entry`, `bundled_provider_catalog_entry_for_base_url`, `default_harness_for_provider_model`) already lives in `codex_model_provider_info`.

8. `codex-rs/tui/src/onboarding/model_selection.rs`
   - Delete the local copies; import from `codex_model_provider_info`. Existing TUI harness tests keep passing.

### Generated artifacts

9. Regenerate TS/JSON schema via the `app-server generate-ts` / `generate-json-schema` subcommands (added in #35). Snapshot tests under `app-server-protocol/schema/` will otherwise fail.

## Tests

- `app-server` handler tests mirroring `app-server/src/models.rs` tests and `message_processor/model_list_tests.rs` (MockServer-based):
  - `interpreterProvider/list` returns configured + catalog entries with correct `configured` / `is_default` flags.
  - `interpreterHarness/list` returns the compatible set per wire API (Messages vs Chat vs Responses).
  - `interpreterProvider/set` / `interpreterModel/set` / `interpreterHarness/set` persist the expected keys to `config.toml` (assert via re-read).
- Keep existing TUI harness tests green after the import move.

## Sequencing (reviewable chunks)

1. Extract harness logic to `model-provider-info` (+ fix TUI imports). Self-contained, no API change.
2. Add `interpreter.rs` types + `common.rs` registrations + `lib.rs` exports. Compiles, no behavior yet.
3. Add `interpreter_api.rs` handlers + dispatch arms.
4. Schema regen + tests.

## Assumed defaults (override if wrong)

- Module names `interpreter.rs` / `interpreter_api.rs` (alt: `custom_app_server`).
- Methods are NOT experimental-gated (plain v2).
- `*/set` take an optional `profile` and persist to config (affects future turns); they do NOT mutate active threads. Per-turn override stays via `turn/start`.
- `InterpreterProvider` omits an "env var actually set" credentials check for now (just `configured`).

## Key references

- Dispatch: `app-server/src/message_processor.rs:362,807`; `app-server/src/codex_message_processor.rs:1130` (ModelList arm), `:5653` (`list_models`).
- Events: `app-server/src/bespoke_event_handling.rs:173` (`apply_bespoke_event_handling`).
- Method macro: `app-server-protocol/src/protocol/common.rs:80`.
- Model listing: `app-server/src/models.rs` (`supported_models_for_provider`).
- Model gate (out of scope): `models-manager/src/manager.rs:332`.
- Harness logic: `tui/src/onboarding/model_selection.rs:91` (`harness_choices_for_provider_model`); enum `tools/src/harness.rs`.
- Persistence: `core/src/config/edit.rs:937` (`ConfigEditsBuilder`: `set_model` `:957`, `set_model_provider` `:965`, `set_harness` `:982`, `apply` `:1235`); existing app-server use at `codex_message_processor.rs:7098`.
