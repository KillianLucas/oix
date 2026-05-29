//! Handlers for the `interpreter*` app-server methods.
//!
//! These power the non-interactive "pick a provider, pick a model, pick a
//! harness" flow over JSON-RPC. The dispatch arms in
//! [`crate::codex_message_processor`] delegate to these free functions.

use std::sync::Arc;

use codex_app_server_protocol::InterpreterHarness;
use codex_app_server_protocol::InterpreterHarnessListParams;
use codex_app_server_protocol::InterpreterHarnessListResponse;
use codex_app_server_protocol::InterpreterHarnessSetParams;
use codex_app_server_protocol::InterpreterHarnessSetResponse;
use codex_app_server_protocol::InterpreterModelListParams;
use codex_app_server_protocol::InterpreterModelListResponse;
use codex_app_server_protocol::InterpreterModelSetParams;
use codex_app_server_protocol::InterpreterModelSetResponse;
use codex_app_server_protocol::InterpreterProvider;
use codex_app_server_protocol::InterpreterProviderListResponse;
use codex_app_server_protocol::InterpreterProviderSetParams;
use codex_app_server_protocol::InterpreterProviderSetResponse;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::WireApiDto;
use codex_core::config::Config;
use codex_core::config::edit::ConfigEditsBuilder;
use codex_login::AuthManager;
use codex_model_provider_info::WireApi;
use codex_model_provider_info::bundled_provider_catalog;
use codex_model_provider_info::harness_selection::harness_choices_for_provider_model;

use crate::error_code::INTERNAL_ERROR_CODE;
use crate::models::supported_models;
use crate::models::supported_models_for_provider;

fn map_wire(wire_api: WireApi) -> WireApiDto {
    match wire_api {
        WireApi::Responses => WireApiDto::Responses,
        WireApi::Chat => WireApiDto::Chat,
        WireApi::Messages => WireApiDto::Messages,
    }
}

/// List known providers: configured providers union the bundled catalog.
///
/// `configured` is true for entries present in `config.model_providers`;
/// `is_default` is true when the id equals `config.model_provider_id`. When
/// `include_unconfigured` is true, bundled catalog entries not already
/// configured are appended with `configured = false`.
pub fn list_providers(
    config: &Config,
    include_unconfigured: bool,
) -> InterpreterProviderListResponse {
    let mut data: Vec<InterpreterProvider> = config
        .model_providers
        .iter()
        .map(|(id, provider)| InterpreterProvider {
            id: id.clone(),
            name: provider.name.clone(),
            base_url: provider.base_url.clone(),
            wire_api: map_wire(provider.wire_api),
            env_key: provider.env_key.clone(),
            configured: true,
            is_default: *id == config.model_provider_id,
        })
        .collect();

    if include_unconfigured {
        data.extend(
            bundled_provider_catalog()
                .iter()
                .filter(|entry| !config.model_providers.contains_key(&entry.id))
                .map(|entry| InterpreterProvider {
                    id: entry.id.clone(),
                    name: entry.name.clone(),
                    base_url: Some(entry.base_url.clone()),
                    wire_api: map_wire(entry.wire_api),
                    env_key: entry.env_key.clone(),
                    configured: false,
                    is_default: entry.id == config.model_provider_id,
                }),
        );
    }

    InterpreterProviderListResponse { data }
}

/// List the models available for a provider. Performs network I/O.
///
/// When `model_provider` is set, lists that provider's models; otherwise lists
/// the active provider's models. `include_hidden` defaults to false.
pub async fn list_models(
    config: &Config,
    auth_manager: Arc<AuthManager>,
    params: InterpreterModelListParams,
) -> Result<InterpreterModelListResponse, JSONRPCErrorError> {
    let InterpreterModelListParams {
        model_provider,
        include_hidden,
    } = params;
    let include_hidden = include_hidden.unwrap_or(false);
    let data = match model_provider {
        Some(provider_id) => supported_models_for_provider(
            config,
            auth_manager,
            provider_id.as_str(),
            include_hidden,
        )
        .await
        .map_err(|message| JSONRPCErrorError {
            code: crate::error_code::INVALID_PARAMS_ERROR_CODE,
            message,
            data: None,
        })?,
        None => supported_models(config, auth_manager, include_hidden).await,
    };
    Ok(InterpreterModelListResponse { data })
}

/// List the harness choices compatible with a provider/model.
///
/// Provider details (name, base URL, wire API) come from
/// `config.model_providers` when configured; otherwise the bundled catalog is
/// consulted internally by `harness_choices_for_provider_model`.
pub fn list_harnesses(
    config: &Config,
    params: InterpreterHarnessListParams,
) -> InterpreterHarnessListResponse {
    let InterpreterHarnessListParams { provider_id, model } = params;
    let provider = config.model_providers.get(&provider_id);
    let choices = harness_choices_for_provider_model(
        &provider_id,
        provider.map(|p| p.name.as_str()),
        provider.and_then(|p| p.base_url.as_deref()),
        provider.map(|p| p.wire_api),
        model.as_deref(),
    );
    let data = choices
        .into_iter()
        .map(|choice| InterpreterHarness {
            id: choice.stored,
            label: choice.label,
            description: choice.description,
            is_recommended: choice.is_recommended,
        })
        .collect();
    InterpreterHarnessListResponse { data }
}

/// Persist the selected provider to config (affects future turns).
pub async fn set_provider(
    config: &Config,
    params: InterpreterProviderSetParams,
) -> Result<InterpreterProviderSetResponse, JSONRPCErrorError> {
    let InterpreterProviderSetParams {
        provider_id,
        profile,
    } = params;
    ConfigEditsBuilder::new(&config.codex_home)
        .with_profile(profile.as_deref())
        .set_model_provider(&provider_id)
        .apply()
        .await
        .map_err(|err| internal_error(format!("failed to set model provider: {err}")))?;
    Ok(InterpreterProviderSetResponse {})
}

/// Persist the selected model (and optional reasoning effort) to config.
pub async fn set_model(
    config: &Config,
    params: InterpreterModelSetParams,
) -> Result<InterpreterModelSetResponse, JSONRPCErrorError> {
    let InterpreterModelSetParams {
        model,
        reasoning_effort,
        profile,
    } = params;
    ConfigEditsBuilder::new(&config.codex_home)
        .with_profile(profile.as_deref())
        .set_model(Some(&model), reasoning_effort)
        .apply()
        .await
        .map_err(|err| internal_error(format!("failed to set model: {err}")))?;
    Ok(InterpreterModelSetResponse {})
}

/// Persist the selected harness to config. `harness == None` selects native.
pub async fn set_harness(
    config: &Config,
    params: InterpreterHarnessSetParams,
) -> Result<InterpreterHarnessSetResponse, JSONRPCErrorError> {
    let InterpreterHarnessSetParams { harness, profile } = params;
    ConfigEditsBuilder::new(&config.codex_home)
        .with_profile(profile.as_deref())
        .set_harness(harness.as_deref())
        .apply()
        .await
        .map_err(|err| internal_error(format!("failed to set harness: {err}")))?;
    Ok(InterpreterHarnessSetResponse {})
}

fn internal_error(message: String) -> JSONRPCErrorError {
    JSONRPCErrorError {
        code: INTERNAL_ERROR_CODE,
        message,
        data: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_core::config::Config;
    use codex_core::config::ConfigBuilder;
    use codex_model_provider_info::ModelProviderInfo;
    use std::collections::BTreeSet;
    use tempfile::tempdir;

    async fn empty_config() -> Config {
        let temp_dir = tempdir().expect("tempdir");
        ConfigBuilder::default()
            .codex_home(temp_dir.path().to_path_buf())
            .build()
            .await
            .expect("config")
    }

    fn provider(name: &str, wire_api: WireApi) -> ModelProviderInfo {
        ModelProviderInfo {
            name: name.to_string(),
            wire_api,
            ..Default::default()
        }
    }

    fn provider_ids(response: &InterpreterProviderListResponse) -> Vec<&str> {
        response.data.iter().map(|p| p.id.as_str()).collect()
    }

    fn harness_ids(response: &InterpreterHarnessListResponse) -> BTreeSet<Option<String>> {
        response.data.iter().map(|h| h.id.clone()).collect()
    }

    async fn list_harnesses_for_wire(wire_api: WireApi) -> InterpreterHarnessListResponse {
        // Use a provider id that is absent from the bundled catalog (and no
        // base URL) so `harness_choices_for_provider_model` falls back to the
        // configured `wire_api` rather than a catalog-derived one. A neutral
        // name keeps the recommendation deterministic (no model-family match).
        let provider_id = "custom-provider";
        let mut config = empty_config().await;
        config
            .model_providers
            .insert(provider_id.to_string(), provider("Custom", wire_api));
        list_harnesses(
            &config,
            InterpreterHarnessListParams {
                provider_id: provider_id.to_string(),
                model: None,
            },
        )
    }

    #[tokio::test]
    async fn list_providers_marks_configured_and_single_default() {
        let mut config = empty_config().await;
        config
            .model_providers
            .insert("custom-a".to_string(), provider("Custom A", WireApi::Chat));
        config
            .model_providers
            .insert("custom-b".to_string(), provider("Custom B", WireApi::Chat));
        config.model_provider_id = "custom-a".to_string();

        let response = list_providers(&config, /*include_unconfigured*/ false);

        assert!(
            response.data.iter().all(|p| p.configured),
            "every entry from config.model_providers should be marked configured"
        );
        let defaults: Vec<&str> = response
            .data
            .iter()
            .filter(|p| p.is_default)
            .map(|p| p.id.as_str())
            .collect();
        assert_eq!(
            defaults,
            vec!["custom-a"],
            "exactly the provider matching config.model_provider_id is the default"
        );
    }

    #[tokio::test]
    async fn list_providers_excludes_catalog_when_not_requested() {
        let config = empty_config().await;

        let response = list_providers(&config, /*include_unconfigured*/ false);

        // "openrouter" is in the bundled catalog but is not a built-in provider,
        // so it must not appear unless unconfigured providers were requested.
        assert!(
            !provider_ids(&response).contains(&"openrouter"),
            "catalog-only providers must be excluded when include_unconfigured = false"
        );
    }

    #[tokio::test]
    async fn list_providers_appends_catalog_without_duplicating_configured() {
        let mut config = empty_config().await;
        // Configure a provider that also exists in the bundled catalog.
        config.model_providers.insert(
            "anthropic".to_string(),
            provider("Anthropic", WireApi::Messages),
        );

        let response = list_providers(&config, /*include_unconfigured*/ true);

        // A catalog-only provider appears as unconfigured.
        let openrouter = response
            .data
            .iter()
            .find(|p| p.id == "openrouter")
            .expect("bundled catalog provider should be appended");
        assert!(
            !openrouter.configured,
            "appended catalog providers are not configured"
        );

        // The configured catalog provider is not duplicated by the catalog pass.
        let anthropic: Vec<&InterpreterProvider> = response
            .data
            .iter()
            .filter(|p| p.id == "anthropic")
            .collect();
        assert_eq!(
            anthropic.len(),
            1,
            "a configured provider must not be duplicated by the catalog"
        );
        assert!(
            anthropic[0].configured,
            "the surviving anthropic entry is the configured one"
        );
    }

    #[tokio::test]
    async fn list_harnesses_responses_offers_only_native() {
        let response = list_harnesses_for_wire(WireApi::Responses).await;
        assert_eq!(harness_ids(&response), BTreeSet::from([None]));
        assert_eq!(response.data.iter().filter(|h| h.is_recommended).count(), 1);
    }

    #[tokio::test]
    async fn list_harnesses_messages_offers_claude_code_variants() {
        let response = list_harnesses_for_wire(WireApi::Messages).await;
        assert_eq!(
            harness_ids(&response),
            BTreeSet::from([
                Some("claude-code".to_string()),
                Some("claude-code-bare".to_string()),
            ])
        );
        let recommended: Vec<Option<String>> = response
            .data
            .iter()
            .filter(|h| h.is_recommended)
            .map(|h| h.id.clone())
            .collect();
        assert_eq!(recommended, vec![Some("claude-code".to_string())]);
    }

    #[tokio::test]
    async fn list_harnesses_chat_offers_full_set_including_native() {
        let response = list_harnesses_for_wire(WireApi::Chat).await;
        assert_eq!(
            harness_ids(&response),
            BTreeSet::from([
                None,
                Some("claude-code".to_string()),
                Some("claude-code-bare".to_string()),
                Some("kimi-cli".to_string()),
                Some("qwen-code".to_string()),
                Some("deepseek-tui".to_string()),
                Some("mini-swe-agent".to_string()),
                Some("opencode".to_string()),
                Some("swe-agent".to_string()),
                Some("terminus-2".to_string()),
                Some("minimal".to_string()),
            ])
        );
        // With a neutral provider and no model, the native harness is recommended.
        let recommended: Vec<Option<String>> = response
            .data
            .iter()
            .filter(|h| h.is_recommended)
            .map(|h| h.id.clone())
            .collect();
        assert_eq!(recommended, vec![None]);
    }
}
