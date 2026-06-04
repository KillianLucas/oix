use std::sync::Arc;

use codex_app_server_protocol::Model;
use codex_app_server_protocol::ModelServiceTier;
use codex_app_server_protocol::ModelUpgradeInfo;
use codex_app_server_protocol::ReasoningEffortOption;
use codex_core::ThreadManager;
use codex_core::build_models_manager;
use codex_core::config::Config;
use codex_login::AuthManager;
use codex_models_manager::manager::RefreshStrategy;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ModelPreset;
use codex_protocol::openai_models::ReasoningEffortPreset;

pub async fn supported_models(
    thread_manager: Arc<ThreadManager>,
    include_hidden: bool,
) -> Vec<Model> {
    models_from_presets(
        thread_manager
            .list_models(RefreshStrategy::OnlineIfUncached)
            .await,
        include_hidden,
    )
}

pub async fn supported_models_for_provider(
    config: &Config,
    auth_manager: Arc<AuthManager>,
    provider_id: &str,
    include_hidden: bool,
) -> Vec<Model> {
    let Some(provider) = config.model_providers.get(provider_id).cloned() else {
        return Vec::new();
    };
    if let Some(cached_models) = provider_specific_cached_models(config, provider_id) {
        return models_from_presets(
            cached_models.into_iter().map(ModelPreset::from).collect(),
            include_hidden,
        );
    }

    let mut provider_config = config.clone();
    provider_config.model_provider_id = provider_id.to_string();
    provider_config.model_provider = provider;
    let models_manager = build_models_manager(&provider_config, auth_manager);
    models_from_presets(
        models_manager
            .list_models(RefreshStrategy::OnlineIfUncached)
            .await,
        include_hidden,
    )
}

fn provider_specific_cached_models(config: &Config, provider_id: &str) -> Option<Vec<ModelInfo>> {
    let cache_path = config
        .codex_home
        .join("models-cache")
        .join(provider_id)
        .join("models_cache.json");
    let bytes = std::fs::read(cache_path).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let models = value.get("models")?.clone();
    serde_json::from_value(models).ok()
}

fn models_from_presets(presets: Vec<ModelPreset>, include_hidden: bool) -> Vec<Model> {
    presets
        .into_iter()
        .filter(|preset| include_hidden || preset.show_in_picker)
        .map(model_from_preset)
        .collect()
}

fn model_from_preset(preset: ModelPreset) -> Model {
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
        input_modalities: preset.input_modalities,
        supports_personality: preset.supports_personality,
        additional_speed_tiers: preset.additional_speed_tiers,
        service_tiers: preset
            .service_tiers
            .into_iter()
            .map(|service_tier| ModelServiceTier {
                id: service_tier.id,
                name: service_tier.name,
                description: service_tier.description,
            })
            .collect(),
        default_service_tier: preset.default_service_tier,
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
