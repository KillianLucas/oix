use codex_arg0::Arg0DispatchPaths;
use codex_arg0::arg0_dispatch_or_else_current_thread;
use codex_server_cli::home::ensure_interpreter_home_env;

fn main() -> anyhow::Result<()> {
    ensure_interpreter_home_env()?;
    arg0_dispatch_or_else_current_thread(|arg0_paths: Arg0DispatchPaths| async move {
        codex_acp_server::run_main(arg0_paths).await
    })
}
