pub(crate) mod cache;
pub mod collaboration_mode_presets;
pub(crate) mod config;
pub mod manager;
pub mod model_info;
pub mod model_presets;
pub mod provider_catalog_models;
pub mod test_support;

pub use codex_app_server_protocol::AuthMode;
use codex_login::default_client::CODEX_BACKEND_CLIENT_VERSION;
pub use config::ModelsManagerConfig;

/// Load the bundled model catalog shipped with `codex-models-manager`.
pub fn bundled_models_response()
-> std::result::Result<codex_protocol::openai_models::ModelsResponse, serde_json::Error> {
    serde_json::from_str(include_str!("../models.json"))
}

/// Version sent to `/models` for backend compatibility checks.
pub fn client_version_to_whole() -> String {
    // OpenAI's model catalog is keyed to Codex's backend compatibility version.
    CODEX_BACKEND_CLIENT_VERSION.to_string()
}
