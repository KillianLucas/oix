use std::sync::Arc;

use codex_app_server_protocol::Model;
use codex_app_server_protocol::ModelUpgradeInfo;
use codex_app_server_protocol::ReasoningEffortOption;
use codex_core::config::Config;
use codex_features::Feature;
use codex_login::AuthManager;
use codex_model_provider::create_model_provider;
use codex_model_provider_info::ModelProviderInfo;
use codex_model_provider_info::provider_selection::provider_preset_by_id;
use codex_models_manager::collaboration_mode_presets::CollaborationModesConfig;
use codex_models_manager::manager::RefreshStrategy;
use codex_protocol::openai_models::ModelPreset;
use codex_protocol::openai_models::ReasoningEffortPreset;

pub async fn supported_models(
    config: &Config,
    auth_manager: Arc<AuthManager>,
    include_hidden: bool,
) -> Vec<Model> {
    let collaboration_modes_config = CollaborationModesConfig {
        default_mode_request_user_input: config
            .features
            .enabled(Feature::DefaultModeRequestUserInput),
    };
    let models = create_model_provider(config.model_provider.clone(), Some(auth_manager))
        .models_manager(
            provider_cache_home(config, config.model_provider_id.as_str()),
            config.model_catalog.clone(),
            collaboration_modes_config,
        )
        .list_models(RefreshStrategy::OnlineIfUncached)
        .await;
    presets_into_models(models, include_hidden)
}

pub async fn supported_models_for_provider(
    config: &Config,
    auth_manager: Arc<AuthManager>,
    provider_id: &str,
    include_hidden: bool,
) -> Result<Vec<Model>, String> {
    let collaboration_modes_config = CollaborationModesConfig {
        default_mode_request_user_input: config
            .features
            .enabled(Feature::DefaultModeRequestUserInput),
    };

    // A configured provider queries its full catalog (cache/online), and the
    // active provider additionally layers the user's `model_catalog` override.
    // An unconfigured quick-add preset has no `model_providers` entry yet, so we
    // build a transient provider from the preset and return its bundled catalog
    // offline. That lets the picker browse a provider's models before it is
    // added to config, without a network round-trip or an API key.
    let (provider, model_catalog, refresh_strategy) = match config.model_providers.get(provider_id)
    {
        Some(provider) => {
            let model_catalog = if provider_id == config.model_provider_id {
                config.model_catalog.clone()
            } else {
                None
            };
            (
                provider.clone(),
                model_catalog,
                RefreshStrategy::OnlineIfUncached,
            )
        }
        None => {
            let preset = provider_preset_by_id(provider_id)
                .ok_or_else(|| format!("model provider `{provider_id}` not found"))?;
            let provider = ModelProviderInfo {
                name: preset.title.clone(),
                base_url: Some(preset.base_url.clone()),
                env_key: preset.api_key_env_var.clone(),
                wire_api: preset.wire_api(),
                ..Default::default()
            };
            (provider, None, RefreshStrategy::Offline)
        }
    };

    let models = create_model_provider(provider, Some(auth_manager))
        .models_manager(
            provider_cache_home(config, provider_id),
            model_catalog,
            collaboration_modes_config,
        )
        .list_models(refresh_strategy)
        .await;

    Ok(presets_into_models(models, include_hidden))
}

/// Filter out picker-hidden presets (unless `include_hidden`) and map the
/// remaining presets to the app-server `Model` wire type.
fn presets_into_models(models: Vec<ModelPreset>, include_hidden: bool) -> Vec<Model> {
    models
        .into_iter()
        .filter(|preset| include_hidden || preset.show_in_picker)
        .map(model_from_preset)
        .collect()
}

pub(crate) fn model_from_preset(preset: ModelPreset) -> Model {
    Model {
        id: preset.id.to_string(),
        model: preset.model.to_string(),
        upgrade: preset.upgrade.as_ref().map(|upgrade| upgrade.id.clone()),
        upgrade_info: preset.upgrade.as_ref().map(|upgrade| ModelUpgradeInfo {
            model: upgrade.id.clone(),
            upgrade_copy: upgrade.upgrade_copy.clone(),
            model_link: upgrade.model_link.clone(),
            migration_markdown: upgrade.migration_markdown.clone(),
        }),
        availability_nux: preset.availability_nux.map(Into::into),
        display_name: preset.display_name.to_string(),
        description: preset.description.to_string(),
        hidden: !preset.show_in_picker,
        supported_reasoning_efforts: reasoning_efforts_from_preset(
            preset.supported_reasoning_efforts,
        ),
        default_reasoning_effort: preset.default_reasoning_effort,
        reasoning_control: preset.reasoning_control,
        supports_thinking_toggle: preset.supports_thinking_toggle,
        input_modalities: preset.input_modalities,
        supports_personality: preset.supports_personality,
        additional_speed_tiers: preset.additional_speed_tiers,
        is_default: preset.is_default,
    }
}

fn reasoning_efforts_from_preset(
    efforts: Vec<ReasoningEffortPreset>,
) -> Vec<ReasoningEffortOption> {
    efforts
        .iter()
        .map(|preset| ReasoningEffortOption {
            reasoning_effort: preset.effort,
            description: preset.description.to_string(),
        })
        .collect()
}

fn provider_cache_home(config: &Config, provider_id: &str) -> std::path::PathBuf {
    config
        .codex_home
        .join("models-cache")
        .join(sanitize_provider_id(provider_id))
        .to_path_buf()
}

fn sanitize_provider_id(provider_id: &str) -> String {
    provider_id
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => ch,
            _ => '_',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_login::AuthCredentialsStoreMode;
    use codex_login::CodexAuth;
    use codex_model_provider_info::ModelProviderInfo;
    use codex_model_provider_info::WireApi;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::method;
    use wiremock::matchers::path;

    #[test]
    fn sanitize_provider_id_replaces_non_filesystem_chars() {
        assert_eq!(
            sanitize_provider_id("custom/provider:alpha"),
            "custom_provider_alpha".to_string()
        );
    }

    #[tokio::test]
    async fn supported_models_for_provider_uses_provider_specific_bundled_catalog() {
        let temp_dir = tempdir().expect("tempdir");
        let mut config = codex_core::config::ConfigBuilder::default()
            .codex_home(temp_dir.path().to_path_buf())
            .build()
            .await
            .expect("config");
        config.model_providers.insert(
            "anthropic".to_string(),
            ModelProviderInfo {
                name: "Anthropic".to_string(),
                base_url: Some("https://api.anthropic.com".to_string()),
                env_key: Some("ANTHROPIC_API_KEY".to_string()),
                env_key_instructions: None,
                experimental_bearer_token: None,
                auth: None,
                aws: None,
                wire_api: WireApi::Messages,
                query_params: None,
                http_headers: None,
                env_http_headers: None,
                request_max_retries: None,
                stream_max_retries: None,
                stream_idle_timeout_ms: None,
                websocket_connect_timeout_ms: None,
                requires_openai_auth: false,
                supports_websockets: false,
            },
        );

        let models = supported_models_for_provider(
            &config,
            AuthManager::from_auth_for_testing(CodexAuth::from_api_key("Test API Key")),
            "anthropic",
            /*include_hidden*/ false,
        )
        .await
        .expect("provider models");

        assert!(
            models.iter().any(|model| model.model == "claude-opus-4-7"),
            "expected Anthropic bundled catalog to expose Claude models"
        );
        assert!(
            !models.iter().any(|model| model.model.starts_with("gpt-")),
            "expected Anthropic model picker to exclude OpenAI bundled models"
        );
    }

    #[tokio::test]
    async fn supported_models_for_provider_falls_back_to_bundled_preset_when_unconfigured() {
        let temp_dir = tempdir().expect("tempdir");
        let config = codex_core::config::ConfigBuilder::default()
            .codex_home(temp_dir.path().to_path_buf())
            .build()
            .await
            .expect("config");
        // Quick-add presets are not written to `model_providers`, so listing their
        // models must work before the provider is configured.
        assert!(
            !config.model_providers.contains_key("openrouter"),
            "precondition: openrouter must be unconfigured for this test"
        );

        let auth = || AuthManager::from_auth_for_testing(CodexAuth::from_api_key("Test API Key"));

        // OpenRouter preset → bundled slug catalog (this is the hosted catalog the
        // app's hosted picker browses).
        let openrouter = supported_models_for_provider(&config, auth(), "openrouter", false)
            .await
            .expect("openrouter preset models");
        assert!(
            openrouter
                .iter()
                .any(|model| model.model == "anthropic/claude-sonnet-4.6"),
            "expected the unconfigured OpenRouter preset to expose its bundled slug catalog"
        );

        // Anthropic preset → bundled Claude catalog, no OpenAI bleed-through.
        let anthropic = supported_models_for_provider(&config, auth(), "anthropic", false)
            .await
            .expect("anthropic preset models");
        assert!(
            anthropic
                .iter()
                .any(|model| model.model == "claude-sonnet-4-6"),
            "expected the unconfigured Anthropic preset to expose its bundled catalog"
        );

        // An id that is neither configured nor a known preset still errors.
        let err = supported_models_for_provider(&config, auth(), "not-a-real-provider", false)
            .await
            .expect_err("unknown provider id must error");
        assert!(err.contains("not found"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn supported_models_for_provider_allows_public_catalog_without_provider_env_key() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_json(serde_json::json!({
                        "data": [
                            {
                                "id": "anthropic/claude-sonnet-4.6",
                                "name": "Anthropic: Claude Sonnet 4.6",
                                "description": "High-end Anthropic reasoning model",
                                "supported_parameters": [
                                    "tools",
                                    "tool_choice",
                                    "reasoning",
                                    "temperature"
                                ],
                                "architecture": {
                                    "input_modalities": ["text", "image"]
                                }
                            }
                        ]
                    })),
            )
            .mount(&server)
            .await;

        let temp_dir = tempdir().expect("tempdir");
        let mut config = codex_core::config::ConfigBuilder::default()
            .codex_home(temp_dir.path().to_path_buf())
            .build()
            .await
            .expect("config");
        let mut provider = config.model_provider.clone();
        provider.name = "OpenRouter".to_string();
        provider.base_url = Some(server.uri());
        provider.env_key = Some("OPENROUTER_API_KEY".to_string());
        provider.env_key_instructions = Some("Set OPENROUTER_API_KEY".to_string());
        provider.experimental_bearer_token = None;
        provider.auth = None;
        provider.query_params = None;
        provider.http_headers = None;
        provider.env_http_headers = None;
        provider.request_max_retries = Some(0);
        provider.stream_max_retries = Some(0);
        provider.stream_idle_timeout_ms = Some(5_000);
        provider.websocket_connect_timeout_ms = None;
        provider.requires_openai_auth = false;
        provider.supports_websockets = false;
        config
            .model_providers
            .insert("openrouter".to_string(), provider);

        let auth_manager = Arc::new(AuthManager::new(
            temp_dir.path().to_path_buf(),
            /*enable_codex_api_key_env*/ false,
            AuthCredentialsStoreMode::File,
            None,
        ));
        let openrouter_provider = config
            .model_providers
            .get("openrouter")
            .cloned()
            .expect("openrouter provider");
        let direct_models =
            create_model_provider(openrouter_provider, Some(Arc::clone(&auth_manager)))
                .models_manager(
                    temp_dir.path().to_path_buf(),
                    None,
                    CollaborationModesConfig::default(),
                )
                .list_models(codex_models_manager::manager::RefreshStrategy::OnlineIfUncached)
                .await;
        assert!(
            direct_models
                .iter()
                .any(|model| model.model == "anthropic/claude-sonnet-4.6"),
            "expected direct manager fallback to expose the public OpenRouter catalog"
        );
        let models = supported_models_for_provider(
            &config,
            auth_manager,
            "openrouter",
            /*include_hidden*/ false,
        )
        .await
        .expect("provider models");
        let requests = server.received_requests().await.unwrap_or_default();

        assert!(
            models
                .iter()
                .any(|model| model.model == "anthropic/claude-sonnet-4.6"),
            "expected public provider catalog model to be present without OPENROUTER_API_KEY"
        );
        assert!(
            requests.is_empty(),
            "expected bundled public provider catalog fallback without a live /models request"
        );
    }
}
