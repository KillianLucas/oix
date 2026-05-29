pub use crate::cli_common::AltScreenCli;
use crate::cli_common::FeatureToggles;
pub use crate::cli_common::KillCommand;
pub use crate::cli_common::LaunchOptions;
pub use crate::cli_common::daemon_startup_overrides;
use clap::Args;
use clap::FromArgMatches;
use clap::Parser;
use clap::Subcommand as ClapSubcommand;
use codex_cli::mcp_cmd::McpCli;
use codex_tui::Cli as TuiCli;
use codex_utils_cli::CliConfigOverrides;
use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "interpreter",
    about = "Open Interpreter app-server-backed TUI",
    version,
    bin_name = "interpreter",
    subcommand_negates_reqs = true,
    override_usage = "interpreter [OPTIONS] [PROMPT]\n       interpreter [OPTIONS] <COMMAND> [ARGS]"
)]
pub struct ServerCli {
    #[clap(flatten)]
    pub config_overrides: CliConfigOverrides,

    #[clap(flatten)]
    pub feature_toggles: FeatureToggles,

    #[clap(flatten)]
    pub launch: LaunchOptions,

    #[command(flatten)]
    pub alt_screen: AltScreenCli,

    #[command(flatten)]
    pub interactive: TuiCli,

    #[command(subcommand)]
    pub subcommand: Option<Subcommand>,
}

#[derive(Debug, clap::Subcommand)]
pub enum Subcommand {
    /// Resume a previous interactive session (picker by default; use --last to continue the most recent).
    Resume(ResumeCommand),

    /// Fork a previous interactive session (picker by default; use --last to fork the most recent).
    Fork(ForkCommand),

    /// Run Open Interpreter non-interactively through the app-server daemon.
    #[clap(visible_alias = "e")]
    Exec(ExecCommand),

    /// Stop the local Open Interpreter daemon.
    Kill(KillCommand),

    /// Manage external MCP servers.
    Mcp(McpCli),

    /// Manage standalone Open Interpreter updates.
    Update(UpdateCommand),

    /// [experimental] Run app-server protocol tooling.
    AppServer(AppServerCommand),
}

#[derive(Debug, Args)]
pub struct AppServerCommand {
    #[command(subcommand)]
    pub subcommand: AppServerSubcommand,
}

#[derive(Debug, ClapSubcommand)]
pub enum AppServerSubcommand {
    /// [experimental] Generate TypeScript bindings for the app server protocol.
    GenerateTs(GenerateTsCommand),

    /// [experimental] Generate JSON Schema for the app server protocol.
    GenerateJsonSchema(GenerateJsonSchemaCommand),

    /// [internal] Generate internal JSON Schema artifacts for Codex tooling.
    #[clap(hide = true)]
    GenerateInternalJsonSchema(GenerateInternalJsonSchemaCommand),
}

#[derive(Debug, Args)]
pub struct GenerateTsCommand {
    /// Output directory where .ts files will be written
    #[arg(short = 'o', long = "out", value_name = "DIR")]
    pub out_dir: PathBuf,

    /// Optional path to the Prettier executable to format generated files
    #[arg(short = 'p', long = "prettier", value_name = "PRETTIER_BIN")]
    pub prettier: Option<PathBuf>,

    /// Include experimental methods and fields in the generated output
    #[arg(long = "experimental", default_value_t = false)]
    pub experimental: bool,
}

#[derive(Debug, Args)]
pub struct GenerateJsonSchemaCommand {
    /// Output directory where the schema bundle will be written
    #[arg(short = 'o', long = "out", value_name = "DIR")]
    pub out_dir: PathBuf,

    /// Include experimental methods and fields in the generated output
    #[arg(long = "experimental", default_value_t = false)]
    pub experimental: bool,
}

#[derive(Debug, Args)]
pub struct GenerateInternalJsonSchemaCommand {
    /// Output directory where internal JSON Schema artifacts will be written
    #[arg(short = 'o', long = "out", value_name = "DIR")]
    pub out_dir: PathBuf,
}

#[derive(Debug, Parser)]
pub struct UpdateCommand {
    #[command(subcommand)]
    pub subcommand: Option<UpdateSubcommand>,
}

#[derive(Debug, ClapSubcommand)]
pub enum UpdateSubcommand {
    /// Show update status.
    Status,

    /// Run the standalone installer update now.
    Now,

    /// Enable automatic update checks.
    On,

    /// Disable automatic update checks.
    Off,
}

#[derive(Args, Debug)]
struct ResumeCommandRaw {
    /// Conversation/session id (UUID) or thread name. UUIDs take precedence if it parses.
    /// If omitted, use --last to pick the most recent recorded session.
    #[arg(value_name = "SESSION_ID")]
    session_id: Option<String>,

    /// Continue the most recent session without showing the picker.
    #[arg(long = "last", default_value_t = false)]
    last: bool,

    /// Show all sessions (disables cwd filtering and shows CWD column).
    #[arg(long = "all", default_value_t = false)]
    all: bool,

    /// Include non-interactive sessions in the resume picker and --last selection.
    #[arg(long = "include-non-interactive", default_value_t = false)]
    include_non_interactive: bool,

    #[clap(flatten)]
    launch: LaunchOptions,

    #[clap(flatten)]
    interactive: TuiCli,
}

#[derive(Debug)]
pub struct ResumeCommand {
    pub session_id: Option<String>,
    pub last: bool,
    pub all: bool,
    pub include_non_interactive: bool,
    pub launch: LaunchOptions,
    pub interactive: TuiCli,
}

impl From<ResumeCommandRaw> for ResumeCommand {
    fn from(raw: ResumeCommandRaw) -> Self {
        let mut interactive = raw.interactive;
        let (session_id, prompt) = if raw.last && interactive.prompt.is_none() {
            (None, raw.session_id)
        } else {
            (raw.session_id, interactive.prompt.take())
        };
        interactive.prompt = prompt;
        Self {
            session_id,
            last: raw.last,
            all: raw.all,
            include_non_interactive: raw.include_non_interactive,
            launch: raw.launch,
            interactive,
        }
    }
}

impl Args for ResumeCommand {
    fn augment_args(cmd: clap::Command) -> clap::Command {
        ResumeCommandRaw::augment_args(cmd)
    }

    fn augment_args_for_update(cmd: clap::Command) -> clap::Command {
        ResumeCommandRaw::augment_args_for_update(cmd)
    }
}

impl FromArgMatches for ResumeCommand {
    fn from_arg_matches(matches: &clap::ArgMatches) -> Result<Self, clap::Error> {
        ResumeCommandRaw::from_arg_matches(matches).map(Self::from)
    }

    fn update_from_arg_matches(&mut self, matches: &clap::ArgMatches) -> Result<(), clap::Error> {
        *self = ResumeCommandRaw::from_arg_matches(matches).map(Self::from)?;
        Ok(())
    }
}

#[derive(Debug, Parser)]
pub struct ForkCommand {
    /// Conversation/session id (UUID). When provided, forks this session.
    /// If omitted, use --last to pick the most recent recorded session.
    #[arg(value_name = "SESSION_ID")]
    pub session_id: Option<String>,

    /// Fork the most recent session without showing the picker.
    #[arg(long = "last", default_value_t = false, conflicts_with = "session_id")]
    pub last: bool,

    /// Show all sessions (disables cwd filtering and shows CWD column).
    #[arg(long = "all", default_value_t = false)]
    pub all: bool,

    #[clap(flatten)]
    pub launch: LaunchOptions,

    #[clap(flatten)]
    pub interactive: TuiCli,
}

#[derive(Debug, Parser)]
pub struct ExecCommand {
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "ARGS"
    )]
    pub args: Vec<OsString>,
}

pub fn apply_root_overrides(
    mut interactive: TuiCli,
    root_config_overrides: CliConfigOverrides,
) -> TuiCli {
    prepend_config_flags(&mut interactive.config_overrides, root_config_overrides);
    interactive
}

pub fn apply_mcp_root_overrides(
    mut mcp_cli: McpCli,
    root_config_overrides: CliConfigOverrides,
) -> McpCli {
    prepend_config_flags(&mut mcp_cli.config_overrides, root_config_overrides);
    mcp_cli
}

pub fn finalize_resume_interactive(
    mut interactive: TuiCli,
    root_config_overrides: CliConfigOverrides,
    session_id: Option<String>,
    last: bool,
    show_all: bool,
    include_non_interactive: bool,
    resume_cli: TuiCli,
) -> TuiCli {
    let resume_session_id = session_id;
    interactive.resume_picker = resume_session_id.is_none() && !last;
    interactive.resume_last = last;
    interactive.resume_session_id = resume_session_id;
    interactive.resume_show_all = show_all;
    interactive.resume_include_non_interactive = include_non_interactive;

    merge_interactive_cli_flags(&mut interactive, resume_cli);
    prepend_config_flags(&mut interactive.config_overrides, root_config_overrides);

    interactive
}

pub fn finalize_fork_interactive(
    mut interactive: TuiCli,
    root_config_overrides: CliConfigOverrides,
    session_id: Option<String>,
    last: bool,
    show_all: bool,
    fork_cli: TuiCli,
) -> TuiCli {
    let fork_session_id = session_id;
    interactive.fork_picker = fork_session_id.is_none() && !last;
    interactive.fork_last = last;
    interactive.fork_session_id = fork_session_id;
    interactive.fork_show_all = show_all;

    merge_interactive_cli_flags(&mut interactive, fork_cli);
    prepend_config_flags(&mut interactive.config_overrides, root_config_overrides);

    interactive
}

fn prepend_config_flags(
    subcommand_config_overrides: &mut CliConfigOverrides,
    cli_config_overrides: CliConfigOverrides,
) {
    subcommand_config_overrides
        .raw_overrides
        .splice(0..0, cli_config_overrides.raw_overrides);
}

fn merge_interactive_cli_flags(interactive: &mut TuiCli, subcommand_cli: TuiCli) {
    let TuiCli {
        prompt,
        prefill_prompt: _,
        submit_prefill_prompt: _,
        resume_picker: _,
        resume_last: _,
        resume_session_id: _,
        resume_show_all: _,
        resume_include_non_interactive: _,
        fork_picker: _,
        fork_last: _,
        fork_session_id: _,
        fork_show_all: _,
        shared,
        approval_policy,
        web_search,
        no_alt_screen,
        config_overrides,
    } = subcommand_cli;

    interactive.apply_subcommand_overrides(shared.into_inner());

    if let Some(approval) = approval_policy {
        interactive.approval_policy = Some(approval);
    }
    if web_search {
        interactive.web_search = true;
    }
    if no_alt_screen {
        interactive.no_alt_screen = true;
    }
    if let Some(prompt) = prompt {
        interactive.prompt = Some(prompt.replace("\r\n", "\n").replace('\r', "\n"));
    }

    interactive
        .config_overrides
        .raw_overrides
        .extend(config_overrides.raw_overrides);
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use pretty_assertions::assert_eq;
    use std::path::Path;

    #[test]
    fn app_server_generate_ts_parses_flags() {
        let cli = ServerCli::parse_from([
            "interpreter",
            "app-server",
            "generate-ts",
            "-o",
            "/tmp/out",
            "-p",
            "/usr/bin/prettier",
            "--experimental",
        ]);

        let Some(Subcommand::AppServer(AppServerCommand {
            subcommand: AppServerSubcommand::GenerateTs(ts_cmd),
        })) = cli.subcommand
        else {
            panic!("expected app-server generate-ts subcommand");
        };
        assert_eq!(ts_cmd.out_dir, Path::new("/tmp/out"));
        assert_eq!(
            ts_cmd.prettier.as_deref(),
            Some(Path::new("/usr/bin/prettier"))
        );
        assert!(ts_cmd.experimental);
    }

    #[test]
    fn app_server_generate_json_schema_parses_without_experimental() {
        let cli =
            ServerCli::parse_from(["interpreter", "app-server", "generate-json-schema", "--out", "/tmp/out"]);

        let Some(Subcommand::AppServer(AppServerCommand {
            subcommand: AppServerSubcommand::GenerateJsonSchema(schema_cmd),
        })) = cli.subcommand
        else {
            panic!("expected app-server generate-json-schema subcommand");
        };
        assert_eq!(schema_cmd.out_dir, Path::new("/tmp/out"));
        assert!(!schema_cmd.experimental);
    }

    #[test]
    fn forwards_feature_toggles_into_config_overrides() {
        let cli = ServerCli::parse_from([
            "interpreter",
            "--enable",
            "foo",
            "--disable",
            "bar",
            "-c",
            "model=\"gpt-5.4\"",
        ]);

        let mut root_overrides = cli.config_overrides;
        root_overrides
            .raw_overrides
            .extend(cli.feature_toggles.into_overrides());
        let interactive = apply_root_overrides(cli.interactive, root_overrides);

        assert_eq!(
            interactive.config_overrides.raw_overrides,
            vec![
                "model=\"gpt-5.4\"".to_string(),
                "features.foo=true".to_string(),
                "features.bar=false".to_string(),
            ]
        );
    }

    #[test]
    fn daemon_startup_overrides_do_not_reintroduce_removed_defaults() {
        let overrides = CliConfigOverrides {
            raw_overrides: vec![
                "model=\"gpt-5.4\"".to_string(),
                "features.foo=true".to_string(),
            ],
        };

        assert_eq!(daemon_startup_overrides(&overrides), Vec::<String>::new());
    }

    #[test]
    fn daemon_startup_overrides_keep_only_daemon_safe_feature_overrides() {
        let overrides = CliConfigOverrides {
            raw_overrides: vec![
                "features.apps=false".to_string(),
                "features.plugins=true".to_string(),
                "features.default_mode_request_user_input=true".to_string(),
                "model=\"gpt-5.4-mini\"".to_string(),
            ],
        };

        assert_eq!(
            daemon_startup_overrides(&overrides),
            vec![
                "features.apps=false".to_string(),
                "features.plugins=true".to_string(),
                "features.default_mode_request_user_input=true".to_string(),
            ]
        );
    }

    #[test]
    fn parses_remote_options_separately_from_tui_cli() {
        let cli = ServerCli::parse_from([
            "interpreter",
            "--remote",
            "ws://127.0.0.1:7777",
            "--remote-auth-token-env",
            "CODEX_TOKEN",
            "hello",
        ]);

        assert_eq!(cli.launch.remote, Some("ws://127.0.0.1:7777".to_string()));
        assert_eq!(
            cli.launch.remote_auth_token_env,
            Some("CODEX_TOKEN".to_string())
        );
        assert_eq!(cli.interactive.prompt, Some("hello".to_string()));
    }

    #[test]
    fn resume_subcommand_merges_root_and_subcommand_flags() {
        let cli = ServerCli::parse_from([
            "interpreter",
            "--enable",
            "foo",
            "--profile",
            "root-profile",
            "resume",
            "--last",
            "--profile",
            "resume-profile",
            "--search",
            "hello",
        ]);

        let ServerCli {
            config_overrides,
            feature_toggles,
            launch: _,
            alt_screen: _,
            interactive,
            subcommand,
        } = cli;
        let Some(Subcommand::Resume(resume)) = subcommand else {
            panic!("expected resume subcommand");
        };
        let mut root_overrides = config_overrides;
        root_overrides
            .raw_overrides
            .extend(feature_toggles.into_overrides());
        let interactive = finalize_resume_interactive(
            interactive,
            root_overrides,
            resume.session_id,
            resume.last,
            resume.all,
            resume.include_non_interactive,
            resume.interactive,
        );

        assert!(interactive.resume_last);
        assert_eq!(
            interactive.config_profile.as_deref(),
            Some("resume-profile")
        );
        assert!(interactive.web_search);
        assert_eq!(interactive.prompt.as_deref(), Some("hello"));
        assert_eq!(
            interactive.config_overrides.raw_overrides,
            vec!["features.foo=true".to_string(),]
        );
    }

    #[test]
    fn alt_screen_flag_parses_as_global_after_resume_subcommand() {
        let cli = ServerCli::parse_from(["interpreter", "resume", "--last", "--alt-screen"]);

        assert!(cli.alt_screen.alt_screen);
    }

    #[test]
    fn exec_subcommand_captures_trailing_args_verbatim() {
        let cli = ServerCli::parse_from([
            "interpreter",
            "exec",
            "--json",
            "--profile",
            "chat",
            "hello",
        ]);

        let Some(Subcommand::Exec(exec)) = cli.subcommand else {
            panic!("expected exec subcommand");
        };
        assert_eq!(
            exec.args,
            vec![
                OsString::from("--json"),
                OsString::from("--profile"),
                OsString::from("chat"),
                OsString::from("hello"),
            ]
        );
    }

    #[test]
    fn help_hides_internal_app_server_override() {
        let mut command = ServerCli::command();
        let mut help = Vec::new();
        command
            .write_long_help(&mut help)
            .expect("render interpreter help");
        let help = String::from_utf8(help).expect("help should be utf8");

        assert!(!help.contains("--app-server-bin"));
    }

    #[test]
    fn kill_subcommand_parses() {
        let cli = ServerCli::parse_from(["interpreter", "kill"]);

        let Some(Subcommand::Kill(kill)) = cli.subcommand else {
            panic!("expected kill subcommand");
        };
        assert_eq!(kill, KillCommand::default());
    }

    #[test]
    fn kill_subcommand_force_parses() {
        let cli = ServerCli::parse_from(["interpreter", "kill", "--force"]);

        let Some(Subcommand::Kill(kill)) = cli.subcommand else {
            panic!("expected kill subcommand");
        };
        assert_eq!(kill, KillCommand { force: true });
    }

    #[test]
    fn mcp_subcommand_parses() {
        let cli = ServerCli::parse_from(["interpreter", "mcp", "list"]);

        let Some(Subcommand::Mcp(mcp_cli)) = cli.subcommand else {
            panic!("expected mcp subcommand");
        };
        assert_eq!(mcp_cli.binary_name, "interpreter");
    }

    #[test]
    fn mcp_subcommand_merges_root_config_overrides() {
        let cli = ServerCli::parse_from(["interpreter", "mcp", "-c", "profile=\"work\"", "list"]);

        let Some(Subcommand::Mcp(mcp_cli)) = cli.subcommand else {
            panic!("expected mcp subcommand");
        };
        let config_overrides = CliConfigOverrides {
            raw_overrides: vec!["model=\"gpt-5.4\"".to_string()],
        };
        let mcp_cli = apply_mcp_root_overrides(mcp_cli, config_overrides);

        assert_eq!(
            mcp_cli.config_overrides.raw_overrides,
            vec![
                "model=\"gpt-5.4\"".to_string(),
                "profile=\"work\"".to_string(),
            ]
        );
    }
}
