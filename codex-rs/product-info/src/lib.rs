pub const OPEN_INTERPRETER_BRAND_ENV_VAR: &str = "OPEN_INTERPRETER_BRAND";

const CODEX_RELEASE_NOTES_URL: &str = "https://github.com/openai/codex/releases/latest";
const OPEN_INTERPRETER_RELEASE_NOTES_URL: &str =
    "https://github.com/KillianLucas/oix/releases/latest";
const CODEX_LATEST_RELEASE_URL: &str = "https://api.github.com/repos/openai/codex/releases/latest";
const OPEN_INTERPRETER_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/KillianLucas/oix/releases/latest";
const CODEX_INSTALLER_URL: &str = "https://chatgpt.com/codex/install.sh";
const OPEN_INTERPRETER_INSTALLER_URL: &str =
    "https://github.com/KillianLucas/oix/releases/latest/download/install.sh";
const CODEX_INSTALL_COMMAND: &str = "curl -fsSL https://chatgpt.com/codex/install.sh | sh";
const OPEN_INTERPRETER_INSTALL_COMMAND: &str = "\
curl -fsSL https://github.com/KillianLucas/oix/releases/latest/download/install.sh | sh";
const CODEX_STANDALONE_UNIX_UPDATE_COMMAND: &str =
    "curl -fsSL https://chatgpt.com/codex/install.sh | CODEX_NON_INTERACTIVE=1 sh";
const OPEN_INTERPRETER_STANDALONE_UNIX_UPDATE_COMMAND: &str = "\
curl -fsSL https://github.com/KillianLucas/oix/releases/latest/download/install.sh | \
CODEX_NON_INTERACTIVE=1 sh";
const CODEX_STANDALONE_WINDOWS_UPDATE_COMMAND: &str =
    "$env:CODEX_NON_INTERACTIVE=1; irm https://chatgpt.com/codex/install.ps1 | iex";
const OPEN_INTERPRETER_STANDALONE_WINDOWS_UPDATE_COMMAND: &str = "\
$env:CODEX_NON_INTERACTIVE=1; \
irm https://github.com/KillianLucas/oix/releases/latest/download/install.ps1 | iex";
const CODEX_STANDALONE_UNIX_UPDATE_ARGS: &[&str] = &["-c", CODEX_STANDALONE_UNIX_UPDATE_COMMAND];
const OPEN_INTERPRETER_STANDALONE_UNIX_UPDATE_ARGS: &[&str] =
    &["-c", OPEN_INTERPRETER_STANDALONE_UNIX_UPDATE_COMMAND];
const CODEX_STANDALONE_WINDOWS_UPDATE_ARGS: &[&str] = &[
    "-ExecutionPolicy",
    "Bypass",
    "-c",
    CODEX_STANDALONE_WINDOWS_UPDATE_COMMAND,
];
const OPEN_INTERPRETER_STANDALONE_WINDOWS_UPDATE_ARGS: &[&str] = &[
    "-ExecutionPolicy",
    "Bypass",
    "-c",
    OPEN_INTERPRETER_STANDALONE_WINDOWS_UPDATE_COMMAND,
];

/// Product channel information used by branded package variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Product {
    Codex,
    OpenInterpreter,
}

impl Product {
    pub fn current() -> Self {
        if std::env::var_os(OPEN_INTERPRETER_BRAND_ENV_VAR).is_some() {
            Self::OpenInterpreter
        } else {
            Self::Codex
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Product::Codex => "OpenAI Codex",
            Product::OpenInterpreter => "Open Interpreter",
        }
    }

    pub fn release_notes_url(self) -> &'static str {
        match self {
            Product::Codex => CODEX_RELEASE_NOTES_URL,
            Product::OpenInterpreter => OPEN_INTERPRETER_RELEASE_NOTES_URL,
        }
    }

    pub fn latest_release_url(self) -> &'static str {
        match self {
            Product::Codex => CODEX_LATEST_RELEASE_URL,
            Product::OpenInterpreter => OPEN_INTERPRETER_LATEST_RELEASE_URL,
        }
    }

    pub fn installer_url(self) -> &'static str {
        match self {
            Product::Codex => CODEX_INSTALLER_URL,
            Product::OpenInterpreter => OPEN_INTERPRETER_INSTALLER_URL,
        }
    }

    pub fn install_command(self) -> &'static str {
        match self {
            Product::Codex => CODEX_INSTALL_COMMAND,
            Product::OpenInterpreter => OPEN_INTERPRETER_INSTALL_COMMAND,
        }
    }

    pub fn installer_env(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Product::Codex => &[],
            Product::OpenInterpreter => &[("CODEX_NON_INTERACTIVE", "1")],
        }
    }

    pub fn standalone_unix_update_args(self) -> &'static [&'static str] {
        match self {
            Product::Codex => CODEX_STANDALONE_UNIX_UPDATE_ARGS,
            Product::OpenInterpreter => OPEN_INTERPRETER_STANDALONE_UNIX_UPDATE_ARGS,
        }
    }

    pub fn standalone_windows_update_args(self) -> &'static [&'static str] {
        match self {
            Product::Codex => CODEX_STANDALONE_WINDOWS_UPDATE_ARGS,
            Product::OpenInterpreter => OPEN_INTERPRETER_STANDALONE_WINDOWS_UPDATE_ARGS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_product_uses_upstream_release_channel() {
        assert_eq!(
            Product::Codex.release_notes_url(),
            "https://github.com/openai/codex/releases/latest"
        );
        assert_eq!(
            Product::Codex.latest_release_url(),
            "https://api.github.com/repos/openai/codex/releases/latest"
        );
        assert_eq!(
            Product::Codex.installer_url(),
            "https://chatgpt.com/codex/install.sh"
        );
        assert_eq!(
            Product::Codex.install_command(),
            "curl -fsSL https://chatgpt.com/codex/install.sh | sh"
        );
        assert_eq!(Product::Codex.installer_env(), &[]);
    }

    #[test]
    fn open_interpreter_product_uses_oix_release_channel() {
        assert_eq!(
            Product::OpenInterpreter.release_notes_url(),
            "https://github.com/KillianLucas/oix/releases/latest"
        );
        assert_eq!(
            Product::OpenInterpreter.latest_release_url(),
            "https://api.github.com/repos/KillianLucas/oix/releases/latest"
        );
        assert_eq!(
            Product::OpenInterpreter.installer_url(),
            "https://github.com/KillianLucas/oix/releases/latest/download/install.sh"
        );
        assert!(
            Product::OpenInterpreter
                .install_command()
                .contains("github.com/KillianLucas/oix/releases/latest/download/install.sh")
        );
        assert_eq!(
            Product::OpenInterpreter.installer_env(),
            &[("CODEX_NON_INTERACTIVE", "1")]
        );
    }
}
