use codex_model_provider_info::WireApi;
use codex_protocol::error::CodexErr;
use codex_tools::Harness;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum MessagesHarnessRoute {
    ClaudeCode,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum ChatHarnessRoute {
    DeepSeekTui,
    KimiCode,
    KimiCli,
    LittleCoder,
    MiniSweAgent,
    Minimal,
    OpenCode,
    Pi,
    QwenCode,
    SweAgent,
    Terminus2,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum StreamTransportRoute {
    ResponsesApi,
    ChatCompletionsCompat,
    ChatHarness(ChatHarnessRoute),
    MessagesHarness(MessagesHarnessRoute),
}

impl StreamTransportRoute {
    pub(crate) fn supports_responses_websocket(self) -> bool {
        matches!(self, Self::ResponsesApi)
    }
}

pub(crate) fn resolve_stream_transport_route(
    wire_api: WireApi,
    harness: &Harness,
) -> Result<StreamTransportRoute, CodexErr> {
    match (wire_api, harness) {
        (WireApi::Responses, Harness::ClaudeCode | Harness::ClaudeCodeBare) => {
            Err(CodexErr::InvalidRequest(
                format!(
                    "harness = \"{}\" requires a provider with wire_api = \"messages\"",
                    harness_config_name(harness)
                ),
            ))
        }
        (WireApi::Responses, _) => Ok(StreamTransportRoute::ResponsesApi),
        (WireApi::Chat, Harness::KimiCli) => {
            Ok(StreamTransportRoute::ChatHarness(ChatHarnessRoute::KimiCli))
        }
        (WireApi::Chat, Harness::KimiCode) => {
            Ok(StreamTransportRoute::ChatHarness(ChatHarnessRoute::KimiCode))
        }
        (WireApi::Chat, Harness::LittleCoder) => Ok(StreamTransportRoute::ChatHarness(
            ChatHarnessRoute::LittleCoder,
        )),
        (WireApi::Chat, Harness::MiniSweAgent) => Ok(StreamTransportRoute::ChatHarness(
            ChatHarnessRoute::MiniSweAgent,
        )),
        (WireApi::Chat, Harness::OpenCode) => Ok(StreamTransportRoute::ChatHarness(
            ChatHarnessRoute::OpenCode,
        )),
        (WireApi::Chat, Harness::Pi) => Ok(StreamTransportRoute::ChatHarness(ChatHarnessRoute::Pi)),
        (WireApi::Chat, Harness::DeepSeekTui) => Ok(StreamTransportRoute::ChatHarness(
            ChatHarnessRoute::DeepSeekTui,
        )),
        (WireApi::Chat, Harness::Minimal) => {
            Ok(StreamTransportRoute::ChatHarness(ChatHarnessRoute::Minimal))
        }
        (WireApi::Chat, Harness::QwenCode) => Ok(StreamTransportRoute::ChatHarness(
            ChatHarnessRoute::QwenCode,
        )),
        (WireApi::Chat, Harness::SweAgent) => Ok(StreamTransportRoute::ChatHarness(
            ChatHarnessRoute::SweAgent,
        )),
        (WireApi::Chat, Harness::Terminus2) => Ok(StreamTransportRoute::ChatHarness(
            ChatHarnessRoute::Terminus2,
        )),
        (WireApi::Chat, _) => Ok(StreamTransportRoute::ChatCompletionsCompat),
        (WireApi::Messages, Harness::ClaudeCode | Harness::ClaudeCodeBare) => {
            Ok(StreamTransportRoute::MessagesHarness(
                MessagesHarnessRoute::ClaudeCode,
            ))
        }
        (WireApi::Messages, Harness::KimiCli) => Err(CodexErr::InvalidRequest(
            "wire_api = \"messages\" is not supported by harness = \"kimi-cli\"".to_string(),
        )),
        (WireApi::Messages, Harness::KimiCode) => Err(CodexErr::InvalidRequest(
            "wire_api = \"messages\" is not supported by harness = \"kimi-code\"".to_string(),
        )),
        (WireApi::Messages, Harness::LittleCoder) => Err(CodexErr::InvalidRequest(
            "wire_api = \"messages\" is not supported by harness = \"little-coder\"".to_string(),
        )),
        (WireApi::Messages, Harness::MiniSweAgent) => Err(CodexErr::InvalidRequest(
            "wire_api = \"messages\" is not supported by harness = \"mini-swe-agent\""
                .to_string(),
        )),
        (WireApi::Messages, Harness::OpenCode) => Err(CodexErr::InvalidRequest(
            "wire_api = \"messages\" is not supported by harness = \"opencode\"".to_string(),
        )),
        (WireApi::Messages, Harness::Pi) => Err(CodexErr::InvalidRequest(
            "wire_api = \"messages\" is not supported by harness = \"pi\"".to_string(),
        )),
        (WireApi::Messages, Harness::DeepSeekTui) => Err(CodexErr::InvalidRequest(
            "wire_api = \"messages\" is not supported by harness = \"deepseek-tui\"".to_string(),
        )),
        (WireApi::Messages, Harness::Minimal) => Err(CodexErr::InvalidRequest(
            "wire_api = \"messages\" is not supported by harness = \"minimal\"".to_string(),
        )),
        (WireApi::Messages, Harness::QwenCode) => Err(CodexErr::InvalidRequest(
            "wire_api = \"messages\" is not supported by harness = \"qwen-code\"".to_string(),
        )),
        (WireApi::Messages, Harness::SweAgent) => Err(CodexErr::InvalidRequest(
            "wire_api = \"messages\" is not supported by harness = \"swe-agent\"".to_string(),
        )),
        (WireApi::Messages, Harness::Terminus2) => Err(CodexErr::InvalidRequest(
            "wire_api = \"messages\" is not supported by harness = \"terminus-2\"".to_string(),
        )),
        (WireApi::Messages, Harness::Native) => Err(CodexErr::InvalidRequest(
            "wire_api = \"messages\" requires a harness-native transport; configure harness = \"claude-code\" or \"claude-code-bare\" for Anthropic-style sessions"
                .to_string(),
        )),
        (WireApi::Messages, Harness::Other(harness_name)) => Err(CodexErr::InvalidRequest(
            format!(
                "wire_api = \"messages\" is not supported by harness = \"{harness_name}\""
            ),
        )),
    }
}

fn harness_config_name(harness: &Harness) -> &str {
    match harness {
        Harness::ClaudeCode => "claude-code",
        Harness::ClaudeCodeBare => "claude-code-bare",
        Harness::Native => "",
        Harness::DeepSeekTui => "deepseek-tui",
        Harness::KimiCode => "kimi-code",
        Harness::KimiCli => "kimi-cli",
        Harness::LittleCoder => "little-coder",
        Harness::MiniSweAgent => "mini-swe-agent",
        Harness::OpenCode => "opencode",
        Harness::Pi => "pi",
        Harness::Minimal => "minimal",
        Harness::QwenCode => "qwen-code",
        Harness::SweAgent => "swe-agent",
        Harness::Terminus2 => "terminus-2",
        Harness::Other(name) => name.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn responses_wire_uses_native_responses_route() {
        assert_eq!(
            resolve_stream_transport_route(WireApi::Responses, &Harness::Native)
                .expect("responses route"),
            StreamTransportRoute::ResponsesApi
        );
    }

    #[test]
    fn chat_wire_uses_chat_compat_route_for_non_claude_harnesses() {
        assert_eq!(
            resolve_stream_transport_route(
                WireApi::Chat,
                &Harness::Other("custom-harness".to_string()),
            )
            .expect("chat route"),
            StreamTransportRoute::ChatCompletionsCompat
        );
    }

    #[test]
    fn claude_code_chat_wire_uses_chat_compat_route() {
        assert_eq!(
            resolve_stream_transport_route(WireApi::Chat, &Harness::ClaudeCode)
                .expect("chat claude-code route"),
            StreamTransportRoute::ChatCompletionsCompat
        );
    }

    #[test]
    fn kimi_cli_chat_wire_uses_harness_native_chat_route() {
        assert_eq!(
            resolve_stream_transport_route(WireApi::Chat, &Harness::KimiCli).expect("kimi route"),
            StreamTransportRoute::ChatHarness(ChatHarnessRoute::KimiCli)
        );
    }

    #[test]
    fn deepseek_tui_chat_wire_uses_harness_native_chat_route() {
        assert_eq!(
            resolve_stream_transport_route(WireApi::Chat, &Harness::DeepSeekTui)
                .expect("deepseek-tui route"),
            StreamTransportRoute::ChatHarness(ChatHarnessRoute::DeepSeekTui)
        );
    }

    #[test]
    fn little_coder_chat_wire_uses_harness_native_chat_route() {
        assert_eq!(
            resolve_stream_transport_route(WireApi::Chat, &Harness::LittleCoder)
                .expect("little-coder route"),
            StreamTransportRoute::ChatHarness(ChatHarnessRoute::LittleCoder)
        );
    }

    #[test]
    fn messages_wire_requires_claude_code_harness() {
        let err = resolve_stream_transport_route(WireApi::Messages, &Harness::Native)
            .expect_err("messages without harness should fail");

        assert_eq!(
            err.to_string(),
            "wire_api = \"messages\" requires a harness-native transport; configure harness = \"claude-code\" or \"claude-code-bare\" for Anthropic-style sessions"
        );
    }

    #[test]
    fn claude_code_harness_rejects_responses_wire() {
        let err = resolve_stream_transport_route(WireApi::Responses, &Harness::ClaudeCode)
            .expect_err("claude-code on responses should fail");

        assert_eq!(
            err.to_string(),
            "harness = \"claude-code\" requires a provider with wire_api = \"messages\""
        );
    }

    #[test]
    fn kimi_cli_harness_rejects_messages_wire() {
        let err = resolve_stream_transport_route(WireApi::Messages, &Harness::KimiCli)
            .expect_err("kimi-cli on messages should fail");

        assert_eq!(
            err.to_string(),
            "wire_api = \"messages\" is not supported by harness = \"kimi-cli\""
        );
    }

    #[test]
    fn mini_swe_agent_chat_wire_uses_harness_native_chat_route() {
        assert_eq!(
            resolve_stream_transport_route(WireApi::Chat, &Harness::MiniSweAgent)
                .expect("mini-swe-agent route"),
            StreamTransportRoute::ChatHarness(ChatHarnessRoute::MiniSweAgent)
        );
    }

    #[test]
    fn opencode_chat_wire_uses_harness_native_chat_route() {
        assert_eq!(
            resolve_stream_transport_route(WireApi::Chat, &Harness::OpenCode)
                .expect("opencode route"),
            StreamTransportRoute::ChatHarness(ChatHarnessRoute::OpenCode)
        );
    }

    #[test]
    fn minimal_chat_wire_uses_harness_native_chat_route() {
        assert_eq!(
            resolve_stream_transport_route(WireApi::Chat, &Harness::Minimal)
                .expect("minimal route"),
            StreamTransportRoute::ChatHarness(ChatHarnessRoute::Minimal)
        );
    }

    #[test]
    fn qwen_code_chat_wire_uses_harness_native_chat_route() {
        assert_eq!(
            resolve_stream_transport_route(WireApi::Chat, &Harness::QwenCode).expect("qwen route"),
            StreamTransportRoute::ChatHarness(ChatHarnessRoute::QwenCode)
        );
    }

    #[test]
    fn swe_agent_chat_wire_uses_harness_native_chat_route() {
        assert_eq!(
            resolve_stream_transport_route(WireApi::Chat, &Harness::SweAgent).expect("swe route"),
            StreamTransportRoute::ChatHarness(ChatHarnessRoute::SweAgent)
        );
    }

    #[test]
    fn terminus_2_chat_wire_uses_harness_native_chat_route() {
        assert_eq!(
            resolve_stream_transport_route(WireApi::Chat, &Harness::Terminus2).expect("route"),
            StreamTransportRoute::ChatHarness(ChatHarnessRoute::Terminus2)
        );
    }
}
