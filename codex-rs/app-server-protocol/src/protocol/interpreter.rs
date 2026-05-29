//! Fork-owned protocol types for the `interpreter*` app-server methods.
//!
//! These power the non-interactive "pick a provider, pick a model, pick a
//! harness" flow over JSON-RPC. All types live in this single module so
//! consumers import them from one path. The method registrations live in
//! [`crate::protocol::common`].

use crate::protocol::v2::Model;
use codex_protocol::openai_models::ReasoningEffort;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

/// Wire protocol a provider speaks, as it appears on the app-server JSON-RPC/TS
/// contract. Deliberately a standalone copy of `codex_model_provider_info::WireApi`,
/// which lives in a heavy domain crate this protocol crate must not depend on; a
/// dedicated wire type also keeps the generated bindings insulated from internal
/// refactors of that enum. Converters map between the two with an exhaustive match,
/// so a new variant on either side fails to compile until it is mapped.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "v2/")]
pub enum WireApiDto {
    /// The Responses API exposed by OpenAI at `/v1/responses`.
    Responses,
    /// OpenAI-compatible Chat Completions exposed at `/v1/chat/completions`.
    Chat,
    /// Anthropic Messages exposed at `/v1/messages`.
    Messages,
}

/// A known provider: the union of configured providers and the bundled
/// catalog. `configured` is true when present in `config.model_providers`;
/// `is_default` is true when its id equals `config.model_provider_id`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterProvider {
    pub id: String,
    pub name: String,
    #[ts(optional)]
    pub base_url: Option<String>,
    pub wire_api: WireApiDto,
    #[ts(optional)]
    pub env_key: Option<String>,
    pub configured: bool,
    pub is_default: bool,
}

/// A harness choice for a provider/model. `id == None` is the native (Codex)
/// harness.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterHarness {
    /// e.g. `"claude-code"`, `"kimi-cli"`; `None` means the native harness.
    #[ts(optional)]
    pub id: Option<String>,
    /// Human-facing label, e.g. `"Claude Code (recommended)"`.
    pub label: String,
    pub description: String,
    pub is_recommended: bool,
}

/// List known providers (configured + bundled catalog).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterProviderListParams {
    /// When true, include providers that are not yet configured.
    #[ts(optional = nullable)]
    pub include_unconfigured: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterProviderListResponse {
    pub data: Vec<InterpreterProvider>,
}

/// List models for a provider.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterModelListParams {
    /// Optional provider override; defaults to the active provider.
    #[ts(optional = nullable)]
    pub model_provider: Option<String>,
    /// When true, include models hidden from the default picker list.
    #[ts(optional = nullable)]
    pub include_hidden: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterModelListResponse {
    pub data: Vec<Model>,
}

/// List the harness choices compatible with a provider/model.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterHarnessListParams {
    pub provider_id: String,
    /// Optional model; refines the compatible harness set.
    #[ts(optional = nullable)]
    pub model: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterHarnessListResponse {
    pub data: Vec<InterpreterHarness>,
}

/// Persist the selected provider to config (affects future turns).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterProviderSetParams {
    pub provider_id: String,
    /// Optional config profile to write to instead of the top level.
    #[ts(optional = nullable)]
    pub profile: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterProviderSetResponse {}

/// Persist the selected model (and optional reasoning effort) to config.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterModelSetParams {
    pub model: String,
    #[ts(optional = nullable)]
    pub reasoning_effort: Option<ReasoningEffort>,
    /// Optional config profile to write to instead of the top level.
    #[ts(optional = nullable)]
    pub profile: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterModelSetResponse {}

/// Persist the selected harness to config. `harness == None` selects native.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterHarnessSetParams {
    /// Harness id, e.g. `"claude-code"`; `None` selects the native harness.
    #[ts(optional = nullable)]
    pub harness: Option<String>,
    /// Optional config profile to write to instead of the top level.
    #[ts(optional = nullable)]
    pub profile: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct InterpreterHarnessSetResponse {}
