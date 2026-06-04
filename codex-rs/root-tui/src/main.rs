use anyhow::Context;
use clap::ArgAction;
use clap::Args;
use clap::Parser;
use codex_app_server_daemon::LifecycleCommand;
use codex_arg0::Arg0DispatchPaths;
use codex_arg0::arg0_dispatch_or_else;
use codex_product_info::OPEN_INTERPRETER_BRAND_ENV_VAR;
use codex_tui::AppExitInfo;
use codex_tui::ExitReason;
use codex_tui::RemoteAppServerEndpoint;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_cli::CliConfigOverrides;
use std::ffi::OsStr;
use std::path::PathBuf;

const CODEX_HOME_ENV_VAR: &str = "CODEX_HOME";
const INTERPRETER_HOME_ENV_VAR: &str = "INTERPRETER_HOME";
const OPEN_INTERPRETER_HOME_ENV_VAR: &str = "OPEN_INTERPRETER_HOME";
const DEFAULT_OPEN_INTERPRETER_HOME_DIR: &str = ".openinterpreter";

#[derive(Parser, Debug)]
#[command(version)]
struct RootTuiCli {
    #[command(flatten)]
    config_overrides: CliConfigOverrides,

    #[command(flatten)]
    feature_toggles: FeatureToggles,

    #[command(flatten)]
    launch: LaunchOptions,

    #[command(flatten)]
    alt_screen: AltScreenCli,

    #[command(flatten)]
    interactive: codex_tui::Cli,
}

#[derive(Parser, Debug, Clone, Default)]
struct FeatureToggles {
    /// Enable a feature. Equivalent to `-c features.<name>=true`.
    #[arg(long = "enable", value_name = "FEATURE", action = ArgAction::Append, global = true)]
    enable: Vec<String>,

    /// Disable a feature. Equivalent to `-c features.<name>=false`.
    #[arg(long = "disable", value_name = "FEATURE", action = ArgAction::Append, global = true)]
    disable: Vec<String>,
}

impl FeatureToggles {
    fn into_overrides(self) -> Vec<String> {
        let mut overrides = Vec::with_capacity(self.enable.len() + self.disable.len());
        overrides.extend(
            self.enable
                .into_iter()
                .map(|feature| format!("features.{feature}=true")),
        );
        overrides.extend(
            self.disable
                .into_iter()
                .map(|feature| format!("features.{feature}=false")),
        );
        overrides
    }
}

#[derive(Debug, Args, Clone, Copy, Default, Eq, PartialEq)]
struct AltScreenCli {
    /// Use fullscreen alternate-screen mode instead of Open Interpreter's inline default.
    #[arg(long = "alt-screen", default_value_t = false, global = true)]
    alt_screen: bool,
}

#[derive(Parser, Debug, Clone, Default)]
struct LaunchOptions {
    /// Connect to a remote app server websocket or unix-socket endpoint.
    #[arg(long = "remote", alias = "url", value_name = "ADDR")]
    remote: Option<String>,

    /// Name of the environment variable containing the bearer token to send to
    /// a remote app server websocket.
    #[arg(long = "remote-auth-token-env", value_name = "ENV_VAR")]
    remote_auth_token_env: Option<String>,
}

fn main() -> anyhow::Result<()> {
    ensure_interpreter_home_env()?;
    arg0_dispatch_or_else(|arg0_paths: Arg0DispatchPaths| async move {
        let RootTuiCli {
            config_overrides,
            feature_toggles,
            launch,
            alt_screen,
            mut interactive,
        } = RootTuiCli::parse();
        interactive.config_overrides = config_overrides;
        interactive
            .config_overrides
            .raw_overrides
            .extend(feature_toggles.into_overrides());
        apply_interpreter_feature_defaults(&mut interactive.config_overrides);
        apply_interpreter_alt_screen_default(&mut interactive.no_alt_screen, alt_screen)?;
        run_root_tui(launch, interactive, arg0_paths).await
    })
}

async fn run_root_tui(
    launch: LaunchOptions,
    mut interactive: codex_tui::Cli,
    arg0_paths: Arg0DispatchPaths,
) -> anyhow::Result<()> {
    if let Some(prompt) = interactive.prompt.take() {
        interactive.prompt = Some(prompt.replace("\r\n", "\n").replace('\r', "\n"));
    }

    let endpoint = match launch.remote {
        Some(remote) => {
            let mut endpoint = codex_tui::resolve_remote_addr(&remote)
                .map_err(|err| anyhow::anyhow!(err.to_string()))?;
            apply_remote_auth_token(&mut endpoint, launch.remote_auth_token_env.as_deref())?;
            endpoint
        }
        None => {
            if launch.remote_auth_token_env.is_some() {
                anyhow::bail!("`--remote-auth-token-env` requires `--remote`.");
            }
            local_daemon_endpoint().await?
        }
    };

    let exit_info = codex_tui::run_main(
        interactive,
        arg0_paths,
        codex_config::LoaderOverrides::default(),
        Some(endpoint),
    )
    .await?;
    handle_app_exit(exit_info)
}

async fn local_daemon_endpoint() -> anyhow::Result<RemoteAppServerEndpoint> {
    let output = codex_app_server_daemon::run(LifecycleCommand::Start).await?;
    Ok(RemoteAppServerEndpoint::UnixSocket {
        socket_path: AbsolutePathBuf::try_from(output.socket_path)?,
    })
}

fn apply_remote_auth_token(
    endpoint: &mut RemoteAppServerEndpoint,
    remote_auth_token_env: Option<&str>,
) -> anyhow::Result<()> {
    let Some(remote_auth_token_env) = remote_auth_token_env else {
        return Ok(());
    };
    if !codex_tui::remote_addr_supports_auth_token(endpoint) {
        anyhow::bail!("`--remote-auth-token-env` requires a `wss://` or loopback `ws://` remote.");
    }
    let auth_token = read_remote_auth_token_from_env_var(remote_auth_token_env)?;
    let RemoteAppServerEndpoint::WebSocket {
        auth_token: slot, ..
    } = endpoint
    else {
        anyhow::bail!("`--remote-auth-token-env` requires a `wss://` or loopback `ws://` remote.");
    };
    *slot = Some(auth_token);
    Ok(())
}

fn handle_app_exit(exit_info: AppExitInfo) -> anyhow::Result<()> {
    match exit_info.exit_reason {
        ExitReason::UserRequested => Ok(()),
        ExitReason::Fatal(message) => anyhow::bail!("{message}"),
    }
}

fn apply_interpreter_alt_screen_default(
    no_alt_screen: &mut bool,
    alt_screen: AltScreenCli,
) -> anyhow::Result<()> {
    if alt_screen.alt_screen && *no_alt_screen {
        anyhow::bail!("`--alt-screen` conflicts with `--no-alt-screen`");
    }

    *no_alt_screen = !alt_screen.alt_screen;

    Ok(())
}

const DEFAULT_MODE_REQUEST_USER_INPUT_OVERRIDE: &str =
    "features.default_mode_request_user_input=true";

fn apply_interpreter_feature_defaults(config_overrides: &mut CliConfigOverrides) {
    if config_overrides.raw_overrides.iter().all(|override_entry| {
        override_entry_key(override_entry) != Some("features.default_mode_request_user_input")
    }) {
        config_overrides
            .raw_overrides
            .push(DEFAULT_MODE_REQUEST_USER_INPUT_OVERRIDE.to_string());
    }
}

fn override_entry_key(override_entry: &str) -> Option<&str> {
    Some(
        override_entry
            .split_once('=')
            .map_or(override_entry, |(path, _)| path)
            .trim(),
    )
}

fn ensure_interpreter_home_env() -> anyhow::Result<PathBuf> {
    let resolved = current_interpreter_home()?;
    std::fs::create_dir_all(&resolved)?;
    let canonical = resolved.canonicalize()?;
    // SAFETY: main() calls this before the tokio runtime starts any background
    // threads, so mutating the process environment here is safe.
    unsafe {
        std::env::set_var(CODEX_HOME_ENV_VAR, &canonical);
        std::env::set_var(INTERPRETER_HOME_ENV_VAR, &canonical);
        std::env::set_var(OPEN_INTERPRETER_HOME_ENV_VAR, &canonical);
        std::env::set_var(OPEN_INTERPRETER_BRAND_ENV_VAR, "1");
    }
    Ok(canonical)
}

fn current_interpreter_home() -> anyhow::Result<PathBuf> {
    resolve_interpreter_home_from_env(
        std::env::var_os(INTERPRETER_HOME_ENV_VAR).as_deref(),
        std::env::var_os(OPEN_INTERPRETER_HOME_ENV_VAR).as_deref(),
        fallback_home_directory(),
    )
}

fn resolve_interpreter_home_from_env(
    interpreter_home: Option<&OsStr>,
    open_interpreter_home: Option<&OsStr>,
    fallback_home_dir: Option<PathBuf>,
) -> anyhow::Result<PathBuf> {
    if let Some(path) = non_empty_path(interpreter_home) {
        return Ok(path);
    }

    if let Some(path) = non_empty_path(open_interpreter_home) {
        return Ok(path);
    }

    let Some(home_dir) = fallback_home_dir else {
        anyhow::bail!("could not find a home directory for Open Interpreter");
    };

    Ok(home_dir.join(DEFAULT_OPEN_INTERPRETER_HOME_DIR))
}

fn non_empty_path(value: Option<&OsStr>) -> Option<PathBuf> {
    value.filter(|value| !value.is_empty()).map(PathBuf::from)
}

fn fallback_home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
}

fn read_remote_auth_token_from_env_var(env_var_name: &str) -> anyhow::Result<String> {
    let token = std::env::var(env_var_name).with_context(|| {
        format!("failed to read remote auth token from environment variable `{env_var_name}`")
    })?;
    if token.trim().is_empty() {
        anyhow::bail!("environment variable `{env_var_name}` contained an empty auth token");
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn alt_screen_defaults_interpreter_to_inline_mode() {
        let mut no_alt_screen = false;

        apply_interpreter_alt_screen_default(&mut no_alt_screen, AltScreenCli::default())
            .expect("apply default alt-screen mode");

        assert_eq!(no_alt_screen, true);
    }

    #[test]
    fn alt_screen_flag_restores_fullscreen_mode() {
        let mut no_alt_screen = false;

        apply_interpreter_alt_screen_default(&mut no_alt_screen, AltScreenCli { alt_screen: true })
            .expect("apply alt-screen override");

        assert_eq!(no_alt_screen, false);
    }

    #[test]
    fn conflicting_alt_screen_flags_error() {
        let mut no_alt_screen = true;

        let err = apply_interpreter_alt_screen_default(
            &mut no_alt_screen,
            AltScreenCli { alt_screen: true },
        )
        .expect_err("conflicting flags should fail");

        assert_eq!(
            err.to_string(),
            "`--alt-screen` conflicts with `--no-alt-screen`"
        );
    }

    #[test]
    fn interpreter_enables_request_user_input_by_default() {
        let mut config_overrides = CliConfigOverrides::default();

        apply_interpreter_feature_defaults(&mut config_overrides);

        assert_eq!(
            config_overrides.raw_overrides,
            vec![DEFAULT_MODE_REQUEST_USER_INPUT_OVERRIDE.to_string()]
        );
    }

    #[test]
    fn explicit_request_user_input_override_is_preserved() {
        let mut config_overrides = CliConfigOverrides {
            raw_overrides: vec!["features.default_mode_request_user_input=false".to_string()],
        };

        apply_interpreter_feature_defaults(&mut config_overrides);

        assert_eq!(
            config_overrides.raw_overrides,
            vec!["features.default_mode_request_user_input=false".to_string()]
        );
    }

    #[test]
    fn resolve_prefers_interpreter_home() {
        let resolved = resolve_interpreter_home_from_env(
            Some(OsStr::new("/tmp/interpreter-home")),
            Some(OsStr::new("/tmp/open-interpreter-home")),
            Some(PathBuf::from("/Users/test")),
        )
        .expect("resolve INTERPRETER_HOME");

        assert_eq!(resolved, PathBuf::from("/tmp/interpreter-home"));
    }

    #[test]
    fn resolve_falls_back_to_open_interpreter_home() {
        let resolved = resolve_interpreter_home_from_env(
            /*interpreter_home*/ None,
            Some(OsStr::new("/tmp/open-interpreter-home")),
            Some(PathBuf::from("/Users/test")),
        )
        .expect("resolve OPEN_INTERPRETER_HOME");

        assert_eq!(resolved, PathBuf::from("/tmp/open-interpreter-home"));
    }

    #[test]
    fn resolve_defaults_to_dot_openinterpreter() {
        let resolved = resolve_interpreter_home_from_env(
            /*interpreter_home*/ None,
            /*open_interpreter_home*/ None,
            Some(PathBuf::from("/Users/test")),
        )
        .expect("resolve default home");

        assert_eq!(resolved, PathBuf::from("/Users/test/.openinterpreter"));
    }
}
