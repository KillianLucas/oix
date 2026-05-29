use crate::ModelProviderInfo;
use crate::WireApi;
use crate::bundled_provider_catalog_entry;
use crate::bundled_provider_catalog_entry_for_base_url;
use crate::default_harness_for_provider_model;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarnessChoice {
    pub stored: Option<String>,
    pub label: String,
    pub description: String,
    pub is_recommended: bool,
}

pub fn harness_choices_for_provider_model(
    provider_id: &str,
    provider_name: Option<&str>,
    base_url: Option<&str>,
    wire_api: Option<WireApi>,
    model: Option<&str>,
) -> Vec<HarnessChoice> {
    // Determine the provider's wire API from a single authoritative source so
    // every screen (onboarding, `/harness`, model switcher) offers the same set
    // of harnesses for the same provider. The bundled catalog — looked up by id,
    // then by base URL — is authoritative for built-in providers; only fall back
    // to the caller-supplied `wire_api` for custom providers that aren't in the
    // catalog. Without this, call sites that don't know a provider's wire (e.g.
    // the new-chat onboarding flow, where built-in providers are reserved and
    // absent from `model_providers`) defaulted to `Responses` and only ever
    // offered the Codex harness.
    let wire_api = bundled_provider_catalog_entry(provider_id)
        .or_else(|| base_url.and_then(bundled_provider_catalog_entry_for_base_url))
        .map(|entry| entry.wire_api)
        .or(wire_api)
        .unwrap_or_default();
    let provider = ModelProviderInfo {
        name: provider_name.unwrap_or_default().to_string(),
        base_url: base_url.map(ToOwned::to_owned),
        wire_api,
        ..Default::default()
    };
    let recommended = default_harness_for_provider_model(provider_id, &provider, model);
    let recommended = recommended.unwrap_or("");
    let mut choices = match provider.wire_api {
        WireApi::Messages => vec!["claude-code", "claude-code-bare"],
        WireApi::Chat => vec![
            "",
            "claude-code",
            "claude-code-bare",
            "kimi-cli",
            "qwen-code",
            "deepseek-tui",
            "mini-swe-agent",
            "opencode",
            "swe-agent",
            "terminus-2",
            "minimal",
        ],
        WireApi::Responses => vec![""],
    };
    choices.sort_by_key(|harness| usize::from(*harness != recommended));
    choices
        .into_iter()
        .map(|harness| harness_choice(harness, harness == recommended))
        .collect()
}

fn harness_choice(harness: &str, is_recommended: bool) -> HarnessChoice {
    let base_label = match harness {
        "" => "Codex",
        "claude-code" => "Claude Code",
        "claude-code-bare" => "Claude Code Bare",
        "kimi-cli" => "Kimi CLI",
        "qwen-code" => "Qwen Code",
        "deepseek-tui" => "DeepSeek TUI",
        "mini-swe-agent" => "mini-swe-agent",
        "opencode" => "opencode",
        "swe-agent" => "SWE-agent",
        "terminus-2" => "Terminus 2",
        "minimal" => "Minimal",
        other => other,
    };
    let label = if is_recommended {
        format!("{base_label} (recommended)")
    } else {
        base_label.to_string()
    };
    let description = match harness {
        "" => "Use the native Codex tool harness.",
        "claude-code" => "Use the Claude Code-style tool harness.",
        "claude-code-bare" => "Use the lean Claude Code-style harness.",
        "kimi-cli" => "Use the Kimi CLI-style tool harness.",
        "qwen-code" => "Use the Qwen Code-style tool harness.",
        "deepseek-tui" => "Use the DeepSeek TUI-style tool harness.",
        "mini-swe-agent" => "Use the mini-swe-agent-style tool harness.",
        "opencode" => "Use the opencode-style tool harness.",
        "swe-agent" => "Use the SWE-agent-style tool harness.",
        "terminus-2" => "Use the Terminus 2-style terminal harness.",
        "minimal" => "Use a minimal shell-oriented tool harness.",
        _ => "Use this configured tool harness.",
    }
    .to_string();
    HarnessChoice {
        stored: (!harness.is_empty()).then(|| harness.to_string()),
        label,
        description,
        is_recommended,
    }
}
