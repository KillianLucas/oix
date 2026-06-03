use std::sync::Arc;

use codex_agent_identity::AgentIdentityKey;
use codex_agent_identity::AgentTaskAuthorizationTarget;
use codex_agent_identity::authorization_header_for_agent_task;
use codex_api::AuthProvider;
use codex_api::SharedAuthProvider;
use codex_login::AuthManager;
use codex_login::CodexAuth;
use codex_model_provider_info::ModelProviderInfo;
use codex_protocol::error::CodexErr;
use http::HeaderMap;
use http::HeaderValue;

use crate::bearer_auth_provider::BearerAuthProvider;

#[derive(Clone, Debug)]
struct AgentIdentityAuthProvider {
    auth: codex_login::auth::AgentIdentityAuth,
}

impl AuthProvider for AgentIdentityAuthProvider {
    fn add_auth_headers(&self, headers: &mut HeaderMap) {
        let record = self.auth.record();
        let header_value = self
            .auth
            .process_task_id()
            .ok_or_else(|| std::io::Error::other("agent identity process task is not initialized"))
            .and_then(|task_id| {
                authorization_header_for_agent_task(
                    AgentIdentityKey {
                        agent_runtime_id: &record.agent_runtime_id,
                        private_key_pkcs8_base64: &record.agent_private_key,
                    },
                    AgentTaskAuthorizationTarget {
                        agent_runtime_id: &record.agent_runtime_id,
                        task_id,
                    },
                )
                .map_err(std::io::Error::other)
            });

        if let Ok(header_value) = header_value
            && let Ok(header) = HeaderValue::from_str(&header_value)
        {
            let _ = headers.insert(http::header::AUTHORIZATION, header);
        }

        if let Ok(header) = HeaderValue::from_str(self.auth.account_id()) {
            let _ = headers.insert("ChatGPT-Account-ID", header);
        }

        if self.auth.is_fedramp_account() {
            let _ = headers.insert("X-OpenAI-Fedramp", HeaderValue::from_static("true"));
        }
    }
}

// Some providers are meant to send no auth headers. Examples include local OSS
// providers and custom test providers with `requires_openai_auth = false`.
#[derive(Clone, Debug)]
struct UnauthenticatedAuthProvider;

impl AuthProvider for UnauthenticatedAuthProvider {
    fn add_auth_headers(&self, _headers: &mut HeaderMap) {}
}

pub fn unauthenticated_auth_provider() -> SharedAuthProvider {
    Arc::new(UnauthenticatedAuthProvider)
}

/// Returns the provider-scoped auth manager when this provider uses command-backed auth.
///
/// Providers without custom auth continue using the caller-supplied base manager, when present.
pub(crate) fn auth_manager_for_provider(
    auth_manager: Option<Arc<AuthManager>>,
    provider: &ModelProviderInfo,
) -> Option<Arc<AuthManager>> {
    match provider.auth.clone() {
        Some(config) => Some(AuthManager::external_bearer_only(config)),
        None => auth_manager,
    }
}

fn bearer_auth_provider_from_auth(
    auth: Option<&CodexAuth>,
    provider: &ModelProviderInfo,
) -> codex_protocol::error::Result<BearerAuthProvider> {
    if let Some(api_key) = provider.api_key()? {
        return Ok(BearerAuthProvider {
            token: Some(api_key),
            account_id: None,
            is_fedramp_account: false,
            token_header_name: Some(provider.auth_header_name()),
            use_bearer_prefix: provider.auth_header_prefix().is_some(),
        });
    }

    if let Some(token) = provider.experimental_bearer_token.clone() {
        return Ok(BearerAuthProvider {
            token: Some(token),
            account_id: None,
            is_fedramp_account: false,
            token_header_name: Some(provider.auth_header_name()),
            use_bearer_prefix: provider.auth_header_prefix().is_some(),
        });
    }

    if let Some(auth) = auth {
        let token = auth.get_token()?;
        Ok(BearerAuthProvider {
            token: Some(token),
            account_id: auth.get_account_id(),
            is_fedramp_account: auth.is_fedramp_account(),
            token_header_name: Some(provider.auth_header_name()),
            use_bearer_prefix: provider.auth_header_prefix().is_some(),
        })
    } else if provider.requires_openai_auth {
        let provider_name = provider_display_name(provider);
        Err(CodexErr::InvalidRequest(format!(
            "Authentication required for {provider_name}. Sign in with ChatGPT or configure API key auth before starting a chat."
        )))
    } else if provider.has_command_auth() {
        let provider_name = provider_display_name(provider);
        Err(CodexErr::InvalidRequest(format!(
            "Authentication required for {provider_name}. The configured provider auth command did not return a token."
        )))
    } else {
        Ok(BearerAuthProvider {
            token: None,
            account_id: None,
            is_fedramp_account: false,
            token_header_name: Some(provider.auth_header_name()),
            use_bearer_prefix: provider.auth_header_prefix().is_some(),
        })
    }
}

fn provider_display_name(provider: &ModelProviderInfo) -> &str {
    if provider.name.trim().is_empty() {
        "the selected provider"
    } else {
        provider.name.as_str()
    }
}

pub(crate) fn resolve_provider_auth(
    auth: Option<&CodexAuth>,
    provider: &ModelProviderInfo,
) -> codex_protocol::error::Result<SharedAuthProvider> {
    Ok(Arc::new(bearer_auth_provider_from_auth(auth, provider)?))
}

/// Builds request-header auth for a first-party Codex auth snapshot.
pub fn auth_provider_from_auth(auth: &CodexAuth) -> SharedAuthProvider {
    match auth {
        CodexAuth::AgentIdentity(auth) => {
            Arc::new(AgentIdentityAuthProvider { auth: auth.clone() })
        }
        CodexAuth::ApiKey(_) | CodexAuth::Chatgpt(_) | CodexAuth::ChatgptAuthTokens(_) => {
            Arc::new(BearerAuthProvider {
                token: auth.get_token().ok(),
                account_id: auth.get_account_id(),
                is_fedramp_account: auth.is_fedramp_account(),
                token_header_name: None,
                use_bearer_prefix: true,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use codex_model_provider_info::WireApi;
    use codex_model_provider_info::create_oss_provider_with_base_url;
    use codex_protocol::config_types::ModelProviderAuthInfo;

    use super::*;

    #[test]
    fn unauthenticated_auth_provider_adds_no_headers() {
        let provider =
            create_oss_provider_with_base_url("http://localhost:11434/v1", WireApi::Responses);
        let auth = resolve_provider_auth(/*auth*/ None, &provider).expect("auth should resolve");

        assert!(auth.to_auth_headers().is_empty());
    }

    #[test]
    fn openai_auth_provider_requires_sign_in_when_auth_is_missing() {
        let provider = ModelProviderInfo::create_openai_provider(/*base_url*/ None);

        let error = match resolve_provider_auth(/*auth*/ None, &provider) {
            Ok(_) => panic!("missing OpenAI auth should fail before sending a request"),
            Err(error) => error,
        };

        assert!(matches!(error, CodexErr::InvalidRequest(_)));
        assert!(
            error.to_string().contains("Sign in with ChatGPT"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn command_auth_provider_requires_token_when_auth_is_missing() {
        let provider = ModelProviderInfo {
            auth: Some(ModelProviderAuthInfo {
                command: "missing-token-command".to_string(),
                args: Vec::new(),
                timeout_ms: NonZeroU64::new(5_000).expect("timeout should be non-zero"),
                refresh_interval_ms: 300_000,
                cwd: std::env::current_dir()
                    .expect("current dir should be available")
                    .try_into()
                    .expect("current dir should be absolute"),
            }),
            requires_openai_auth: false,
            ..ModelProviderInfo::create_openai_provider(/*base_url*/ None)
        };

        let error = match resolve_provider_auth(/*auth*/ None, &provider) {
            Ok(_) => panic!("missing command auth should fail before sending a request"),
            Err(error) => error,
        };

        assert!(matches!(error, CodexErr::InvalidRequest(_)));
        assert!(
            error.to_string().contains("provider auth command"),
            "unexpected error: {error}"
        );
    }
}
