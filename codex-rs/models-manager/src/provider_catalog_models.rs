use codex_model_provider_info::BundledProviderModelEntry;
use codex_model_provider_info::ModelProviderInfo;
use codex_model_provider_info::bundled_provider_catalog;
use codex_model_provider_info::bundled_provider_catalog_entry;
use codex_model_provider_info::bundled_provider_catalog_entry_for_base_url;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ModelVisibility;
use codex_protocol::openai_models::ReasoningEffort;

use crate::ModelsManagerConfig;
use crate::manager::construct_model_info_from_candidates;

pub fn bundled_provider_model_infos(provider: &ModelProviderInfo) -> Vec<ModelInfo> {
    let entry = if provider.is_anthropic_provider() {
        bundled_provider_catalog_entry("anthropic")
    } else {
        provider
            .base_url
            .as_deref()
            .and_then(bundled_provider_catalog_entry_for_base_url)
            .or_else(|| {
                bundled_provider_catalog().iter().find(|entry| {
                    entry.name.eq_ignore_ascii_case(provider.name.as_str())
                        || provider
                            .env_key
                            .as_deref()
                            .is_some_and(|env_key| entry.env_key.as_deref() == Some(env_key))
                })
            })
    };
    let Some(entry) = entry else {
        return Vec::new();
    };

    entry
        .models
        .iter()
        .map(model_info_from_bundled_provider_model)
        .collect()
}

pub fn provider_model_info(
    provider: &ModelProviderInfo,
    model: &str,
    config: &ModelsManagerConfig,
    remote_candidates: &[ModelInfo],
) -> ModelInfo {
    if !remote_candidates.is_empty() {
        let model_info = construct_model_info_from_candidates(model, remote_candidates, config);
        if !model_info.used_fallback_model_metadata {
            return model_info;
        }
    }

    let catalog_candidates = if let Some(model_catalog) = config.model_catalog.as_ref() {
        model_catalog.models.clone()
    } else {
        bundled_provider_model_infos(provider)
    };
    construct_model_info_from_candidates(model, &catalog_candidates, config)
}

fn model_info_from_bundled_provider_model(model: &BundledProviderModelEntry) -> ModelInfo {
    let mut fallback = crate::model_info::model_info_from_slug(model.id.as_str());
    fallback.slug = model.id.clone();
    fallback.display_name = model.display_name.clone();
    fallback.description = model.description.clone();
    fallback.default_reasoning_level =
        (model.reasoning || model.thinking_toggle).then_some(ReasoningEffort::Medium);
    fallback.supported_reasoning_levels = Vec::new();
    fallback.visibility = ModelVisibility::List;
    fallback.supported_in_api = true;
    fallback.priority = model.priority;
    fallback.context_window = model.context_window.or(fallback.context_window);
    if !model.input_modalities.is_empty() {
        fallback.input_modalities = model.input_modalities.clone();
    }
    fallback.used_fallback_model_metadata = false;
    fallback
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_model_provider_info::WireApi;
    use pretty_assertions::assert_eq;

    fn provider(name: &str, base_url: &str, env_key: &str, wire_api: WireApi) -> ModelProviderInfo {
        ModelProviderInfo {
            name: name.to_string(),
            base_url: Some(base_url.to_string()),
            env_key: Some(env_key.to_string()),
            wire_api,
            ..Default::default()
        }
    }

    #[test]
    fn bundled_provider_models_seed_deepseek() {
        let models = bundled_provider_model_infos(&provider(
            "DeepSeek",
            "https://api.deepseek.com",
            "DEEPSEEK_API_KEY",
            WireApi::Chat,
        ));
        assert!(models.iter().any(|model| model.slug == "deepseek-v4-pro"));
        let v4_pro = models
            .iter()
            .find(|model| model.slug == "deepseek-v4-pro")
            .expect("deepseek-v4-pro model");
        assert_eq!(v4_pro.visibility, ModelVisibility::List);
        assert_eq!(
            v4_pro.default_reasoning_level,
            Some(ReasoningEffort::Medium)
        );
        assert!(v4_pro.supported_reasoning_levels.is_empty());
    }

    #[test]
    fn bundled_provider_models_seed_anthropic_with_reasoning_and_vision() {
        let models = bundled_provider_model_infos(&provider(
            "Anthropic",
            "https://api.anthropic.com",
            "ANTHROPIC_API_KEY",
            WireApi::Messages,
        ));
        let sonnet = models
            .iter()
            .find(|model| model.slug == "claude-sonnet-4-6")
            .expect("claude-sonnet-4-6 model");
        assert_eq!(sonnet.visibility, ModelVisibility::List);
        assert_eq!(
            sonnet.default_reasoning_level,
            Some(ReasoningEffort::Medium)
        );
        assert!(
            sonnet.supported_reasoning_levels.is_empty(),
            "generated provider catalog should not invent effort levels from a boolean reasoning flag"
        );
        assert!(
            sonnet.input_modalities.iter().any(|modality| {
                matches!(
                    modality,
                    codex_protocol::openai_models::InputModality::Image
                )
            }),
            "expected anthropic model to advertise image input from the generated catalog"
        );
    }

    #[test]
    fn bundled_provider_models_seed_anthropic_for_proxy_base_url() {
        let models = bundled_provider_model_infos(&provider(
            "Anthropic",
            "http://127.0.0.1:9000",
            "ANTHROPIC_API_KEY",
            WireApi::Messages,
        ));
        assert!(
            models.iter().any(|model| model.slug == "claude-sonnet-4-6"),
            "expected Anthropic proxy provider to reuse bundled Anthropic catalog"
        );
    }

    #[test]
    fn bundled_provider_models_seed_openrouter_for_proxy_base_url() {
        let models = bundled_provider_model_infos(&provider(
            "OpenRouter",
            "http://127.0.0.1:4010",
            "OPENROUTER_API_KEY",
            WireApi::Chat,
        ));
        assert!(
            models
                .iter()
                .any(|model| model.slug == "anthropic/claude-sonnet-4.6"),
            "expected OpenRouter proxy provider to reuse bundled OpenRouter catalog"
        );
    }

    #[test]
    fn provider_model_info_uses_active_provider_catalog_when_remote_metadata_misses() {
        let model_info = provider_model_info(
            &provider(
                "Anthropic",
                "https://api.anthropic.com",
                "ANTHROPIC_API_KEY",
                WireApi::Messages,
            ),
            "claude-sonnet-4-6",
            &ModelsManagerConfig::default(),
            &[],
        );

        assert_eq!(model_info.slug, "claude-sonnet-4-6");
        assert!(!model_info.used_fallback_model_metadata);
        assert_eq!(
            model_info.default_reasoning_level,
            Some(ReasoningEffort::Medium)
        );
    }
}
