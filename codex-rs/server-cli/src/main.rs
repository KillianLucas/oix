mod cli_common;
mod daemon;
mod home;
mod startup_trace;
mod system_import;

use codex_arg0::arg0_dispatch_or_else_current_thread;
use codex_login::KIMI_CODE_PROVIDER_ID;
use startup_trace::record_startup_trace_event;
use std::ffi::OsString;
use std::io::IsTerminal;
use std::io::Write;
use std::path::PathBuf;

const INTERPRETER_CLI_BINARY: &str = if cfg!(windows) {
    "interpreter-tui.exe"
} else {
    "interpreter-tui"
};

const INTERPRETER_ROOT_TUI_BINARY: &str = if cfg!(windows) {
    "interpreter-root-tui.exe"
} else {
    "interpreter-root-tui"
};
const INTERPRETER_ACP_BINARY: &str = if cfg!(windows) {
    "interpreter-acp.exe"
} else {
    "interpreter-acp"
};

fn main() -> anyhow::Result<()> {
    record_startup_trace_event("interpreter.main.enter");
    home::ensure_interpreter_home_env()?;
    record_startup_trace_event("interpreter.main.home.ready");

    let raw_args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if let Some(command) = route_top_level_command(&raw_args) {
        return match command {
            TopLevelCommand::Passthrough => exec_interpreter_cli(raw_args),
            TopLevelCommand::Acp => exec_interpreter_acp(raw_args),
            TopLevelCommand::Kill {
                force,
                remote_present,
                remote_auth_token_present,
            } => {
                let launch = crate::cli_common::LaunchOptions {
                    remote: remote_present.then_some(String::new()),
                    remote_auth_token_env: remote_auth_token_present.then_some(String::new()),
                    app_server_bin: None,
                };
                ensure_daemon_command_uses_local_daemon(&launch)?;
                arg0_dispatch_or_else_current_thread(|_| async move { kill_daemon(force).await })
            }
            TopLevelCommand::ProviderAuth { provider_id } => {
                arg0_dispatch_or_else_current_thread(|_| async move {
                    print_provider_auth_token(provider_id).await
                })
            }
        };
    }

    exec_interpreter_root_tui(raw_args)
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum TopLevelCommand {
    Passthrough,
    Acp,
    Kill {
        force: bool,
        remote_present: bool,
        remote_auth_token_present: bool,
    },
    ProviderAuth {
        provider_id: String,
    },
}

fn route_top_level_command(raw_args: &[OsString]) -> Option<TopLevelCommand> {
    scan_top_level_command(raw_args)
        .or_else(|| should_delegate_directly(raw_args).then_some(TopLevelCommand::Passthrough))
}

fn scan_top_level_command(raw_args: &[OsString]) -> Option<TopLevelCommand> {
    let mut remote_present = false;
    let mut remote_auth_token_present = false;
    let mut index = 0usize;
    while index < raw_args.len() {
        let arg = raw_args[index].to_string_lossy();
        if arg == "--" {
            return None;
        }

        if matches!(
            arg.as_ref(),
            "-c" | "--config"
                | "--enable"
                | "--disable"
                | "--remote"
                | "--url"
                | "--remote-auth-token-env"
                | "--app-server-bin"
                | "--image"
                | "-i"
                | "--model"
                | "-m"
                | "--local-provider"
                | "--profile"
                | "-p"
                | "--sandbox"
                | "-s"
                | "--ask-for-approval"
                | "-a"
                | "--cd"
                | "-C"
                | "--add-dir"
        ) {
            if matches!(arg.as_ref(), "--remote" | "--url") {
                remote_present = true;
            }
            if arg == "--remote-auth-token-env" {
                remote_auth_token_present = true;
            }
            index += 2;
            continue;
        }

        if arg.starts_with("--remote=") || arg.starts_with("--url=") {
            remote_present = true;
            index += 1;
            continue;
        }
        if arg.starts_with("--remote-auth-token-env=") {
            remote_auth_token_present = true;
            index += 1;
            continue;
        }
        if arg.starts_with("--")
            || matches!(
                arg.as_ref(),
                "--oss"
                    | "--alt-screen"
                    | "--search"
                    | "--no-alt-screen"
                    | "--full-auto"
                    | "--dangerously-bypass-approvals-and-sandbox"
                    | "--yolo"
            )
        {
            index += 1;
            continue;
        }

        return match arg.as_ref() {
            "help" | "resume" | "fork" | "exec" | "mcp" | "update" => {
                Some(TopLevelCommand::Passthrough)
            }
            "acp" => Some(TopLevelCommand::Acp),
            "kill" => Some(TopLevelCommand::Kill {
                force: raw_args[index + 1..]
                    .iter()
                    .map(|arg| arg.to_string_lossy())
                    .any(|arg| matches!(arg.as_ref(), "-f" | "--force")),
                remote_present,
                remote_auth_token_present,
            }),
            "provider-auth" => {
                raw_args
                    .get(index + 1)
                    .map(|provider_id| TopLevelCommand::ProviderAuth {
                        provider_id: provider_id.to_string_lossy().to_string(),
                    })
            }
            _ => None,
        };
    }

    None
}

fn should_delegate_directly(raw_args: &[OsString]) -> bool {
    raw_args
        .iter()
        .map(|arg| arg.to_string_lossy())
        .any(|arg| matches!(arg.as_ref(), "-h" | "--help" | "-V" | "--version"))
}

fn resolve_interpreter_cli_binary() -> anyhow::Result<PathBuf> {
    resolve_binary(INTERPRETER_CLI_BINARY)
}

fn resolve_interpreter_root_tui_binary() -> anyhow::Result<PathBuf> {
    resolve_binary(INTERPRETER_ROOT_TUI_BINARY)
}

fn resolve_interpreter_acp_binary() -> anyhow::Result<PathBuf> {
    resolve_binary(INTERPRETER_ACP_BINARY)
}

fn resolve_binary(binary_name: &str) -> anyhow::Result<PathBuf> {
    let current_exe = std::env::current_exe()?;
    let sibling = current_exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("interpreter path missing parent directory"))?
        .join(binary_name);
    if sibling.exists() {
        return Ok(sibling);
    }

    which::which(binary_name).map_err(anyhow::Error::from)
}

fn exec_interpreter_cli(raw_args: Vec<OsString>) -> anyhow::Result<()> {
    exec_binary(resolve_interpreter_cli_binary()?, raw_args)
}

fn exec_interpreter_root_tui(raw_args: Vec<OsString>) -> anyhow::Result<()> {
    // The startup status line is owned entirely by `interpreter-root-tui`, which
    // can show it conditionally (only when the daemon is actually cold-starting)
    // and animate it. Printing here would always fire, regardless of state.
    exec_binary(resolve_interpreter_root_tui_binary()?, raw_args)
}

fn exec_interpreter_acp(_raw_args: Vec<OsString>) -> anyhow::Result<()> {
    exec_binary(resolve_interpreter_acp_binary()?, Vec::new())
}

fn exec_binary(program: PathBuf, raw_args: Vec<OsString>) -> anyhow::Result<()> {
    exec_binary_with_env(program, raw_args, [])
}

fn exec_binary_with_env<const N: usize>(
    program: PathBuf,
    raw_args: Vec<OsString>,
    envs: [(&str, &str); N],
) -> anyhow::Result<()> {
    let mut command = std::process::Command::new(&program);
    command.args(raw_args);
    for (key, value) in envs {
        command.env(key, value);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        let err = command.exec();
        Err(anyhow::Error::from(err))
    }

    #[cfg(not(unix))]
    {
        let status = command.status()?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn ensure_daemon_command_uses_local_daemon(
    launch: &crate::cli_common::LaunchOptions,
) -> anyhow::Result<()> {
    if launch.remote.is_some() || launch.remote_auth_token_env.is_some() {
        anyhow::bail!("daemon commands only manage the local Open Interpreter daemon");
    }
    Ok(())
}

async fn print_provider_auth_token(provider_id: String) -> anyhow::Result<()> {
    let interpreter_home = crate::home::current_interpreter_home()?;
    match provider_id.as_str() {
        KIMI_CODE_PROVIDER_ID => {
            let access_token = codex_login::kimi_code::ensure_access_token(
                &interpreter_home,
                /*open_browser*/ true,
            )
            .await
            .map_err(|err| anyhow::anyhow!(err.to_string()))?;
            print!("{access_token}");
            Ok(())
        }
        _ => anyhow::bail!("unsupported provider auth command for `{provider_id}`"),
    }
}

async fn kill_daemon(force: bool) -> anyhow::Result<()> {
    let status = daemon::local_app_server_status().await?;
    let Some(_status) = status else {
        println!("Open Interpreter daemon is not running.");
        return Ok(());
    };

    if !force && !confirm_daemon_stop()? {
        println!("Aborted.");
        return Ok(());
    }

    match daemon::stop_local_app_server().await? {
        daemon::StopLocalAppServerOutcome::NotRunning => {
            println!("Open Interpreter daemon is not running.");
        }
        daemon::StopLocalAppServerOutcome::Stopped(status) => {
            println!("Stopped Open Interpreter daemon (pid {}).", status.pid);
        }
    }
    Ok(())
}

fn confirm_daemon_stop() -> anyhow::Result<bool> {
    let mut stderr = std::io::stderr();
    if !std::io::stdin().is_terminal() {
        anyhow::bail!(
            "daemon is running; rerun with `interpreter kill --force` to stop it non-interactively"
        );
    }

    write!(
        stderr,
        "This will stop the Open Interpreter daemon and disconnect any running sessions. Continue? [y/N] "
    )?;
    stderr.flush()?;

    let mut response = String::new();
    std::io::stdin().read_line(&mut response)?;
    Ok(is_confirmation_response(&response))
}

fn is_confirmation_response(response: &str) -> bool {
    matches!(response.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_subcommand_delegates_to_app_server_cli() {
        assert_eq!(
            scan_top_level_command(&[OsString::from("mcp"), OsString::from("--help")]),
            Some(TopLevelCommand::Passthrough)
        );
    }

    #[test]
    fn acp_subcommand_routes_to_acp_server() {
        assert_eq!(
            scan_top_level_command(&[OsString::from("acp")]),
            Some(TopLevelCommand::Acp)
        );
    }

    #[test]
    fn acp_subcommand_after_root_options_routes_to_acp_server() {
        assert_eq!(
            scan_top_level_command(&[
                OsString::from("-c"),
                OsString::from("model=\"gpt-5.4\""),
                OsString::from("acp"),
            ]),
            Some(TopLevelCommand::Acp)
        );
    }

    #[test]
    fn mcp_help_routes_to_app_server_cli_before_help_passthrough() {
        assert_eq!(
            route_top_level_command(&[OsString::from("mcp"), OsString::from("--help")]),
            Some(TopLevelCommand::Passthrough)
        );
    }

    #[test]
    fn root_help_routes_to_app_server_cli() {
        assert_eq!(
            route_top_level_command(&[OsString::from("--help")]),
            Some(TopLevelCommand::Passthrough)
        );
    }

    #[test]
    fn mcp_subcommand_after_root_options_delegates_to_app_server_cli() {
        assert_eq!(
            scan_top_level_command(&[
                OsString::from("-c"),
                OsString::from("model=\"gpt-5.4\""),
                OsString::from("mcp"),
                OsString::from("list"),
            ]),
            Some(TopLevelCommand::Passthrough)
        );
    }

    #[test]
    fn exec_subcommand_delegates_to_app_server_cli() {
        assert_eq!(
            scan_top_level_command(&[OsString::from("exec"), OsString::from("hello")]),
            Some(TopLevelCommand::Passthrough)
        );
    }

    #[test]
    fn resume_subcommand_delegates_to_app_server_cli() {
        assert_eq!(
            scan_top_level_command(&[OsString::from("resume"), OsString::from("--last")]),
            Some(TopLevelCommand::Passthrough)
        );
    }

    #[test]
    fn update_help_routes_to_app_server_cli() {
        assert_eq!(
            route_top_level_command(&[OsString::from("update"), OsString::from("--help")]),
            Some(TopLevelCommand::Passthrough)
        );
    }

    #[test]
    fn help_subcommand_routes_to_app_server_cli() {
        assert_eq!(
            route_top_level_command(&[OsString::from("help"), OsString::from("update")]),
            Some(TopLevelCommand::Passthrough)
        );
    }
}
