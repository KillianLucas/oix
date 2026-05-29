//! Smoke test for the auto-update non-interactive guard in
//! `scripts/install/install.sh`.
//!
//! The in-app auto-updater (see `crate::updates`) re-runs install.sh with stdio
//! redirected to /dev/null. install.sh's `prompt_yes_no` opens `/dev/tty`
//! directly, which is reachable whenever the process has a controlling terminal
//! (the live TUI). Without a guard the installer would render a launch prompt
//! onto the TUI and block on a tty read, parking the update. `updates.rs` now
//! sets `OPEN_INTERPRETER_NONINTERACTIVE=1` and install.sh honors it.
//!
//! This must run under a real PTY: only with a controlling terminal does the
//! unguarded path reach /dev/tty and block. Without a PTY, `prompt_yes_no`
//! returns "no" regardless of the env var, so the guard would be untestable.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use tokio::time::timeout;

fn install_sh_path() -> PathBuf {
    // codex-tui's manifest dir is `<repo>/codex-rs/tui`; install.sh lives at the
    // repo root under scripts/install.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/install/install.sh")
}

/// Extract the verbatim `prompt_yes_no` function from the shipped install.sh, so
/// the test exercises real installer code (and fails loudly if it is renamed)
/// without sourcing or running the whole installer (which would need network).
fn extract_prompt_yes_no(script: &str) -> String {
    let mut body = String::new();
    let mut in_fn = false;
    for line in script.lines() {
        if line.starts_with("prompt_yes_no() {") {
            in_fn = true;
        }
        if in_fn {
            body.push_str(line);
            body.push('\n');
            if line == "}" {
                return body;
            }
        }
    }
    panic!("prompt_yes_no() not found in install.sh; update this smoke test if it was renamed");
}

fn output_contains(buf: &[u8], needle: &[u8]) -> bool {
    buf.len() >= needle.len() && buf.windows(needle.len()).any(|window| window == needle)
}

/// Run the shipped `prompt_yes_no` under a PTY and return `(exited, output)`.
///
/// `exited` is false when the run timed out, i.e. the function was still blocked
/// on a /dev/tty read. When `answer` is set, it is written to the PTY once the
/// prompt text appears.
async fn run_prompt_yes_no(
    noninteractive: bool,
    answer: Option<&[u8]>,
    wait: Duration,
) -> anyhow::Result<(bool, String)> {
    let script = std::fs::read_to_string(install_sh_path())?;
    let prompt_fn = extract_prompt_yes_no(&script);
    let harness = format!(
        "{prompt_fn}\nif prompt_yes_no \"Start Open Interpreter now?\"; then echo __BRANCH_YES__; else echo __BRANCH_NO__; fi\n"
    );

    let mut env = HashMap::new();
    if noninteractive {
        env.insert(
            "OPEN_INTERPRETER_NONINTERACTIVE".to_string(),
            "1".to_string(),
        );
    }

    let cwd = std::env::current_dir()?;
    let args = vec!["-c".to_string(), harness];
    let codex_utils_pty::SpawnedProcess {
        session,
        stdout_rx,
        stderr_rx,
        exit_rx,
    } = codex_utils_pty::spawn_pty_process(
        "/bin/sh",
        &args,
        &cwd,
        &env,
        &None,
        codex_utils_pty::TerminalSize::default(),
    )
    .await?;

    let mut output_rx = codex_utils_pty::combine_output_receivers(stdout_rx, stderr_rx);
    let mut exit_rx = exit_rx;
    let writer_tx = session.writer_sender();
    let mut output: Vec<u8> = Vec::new();
    let mut answered = false;

    let exited = timeout(wait, async {
        loop {
            tokio::select! {
                chunk = output_rx.recv() => match chunk {
                    Ok(bytes) => {
                        output.extend_from_slice(&bytes);
                        if let Some(answer) = answer
                            && !answered
                            && output_contains(&output, b"Start Open Interpreter now?")
                        {
                            let _ = writer_tx.send(answer.to_vec()).await;
                            answered = true;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                },
                _ = &mut exit_rx => break,
            }
        }
    })
    .await
    .is_ok();

    if !exited {
        session.terminate();
    }
    while let Ok(bytes) = output_rx.try_recv() {
        output.extend_from_slice(&bytes);
    }

    Ok((exited, String::from_utf8_lossy(&output).into_owned()))
}

#[tokio::test]
async fn noninteractive_env_var_suppresses_install_launch_prompt() -> anyhow::Result<()> {
    if cfg!(windows) {
        return Ok(()); // install.sh is Unix-only; Windows uses install.ps1.
    }

    let (exited, output) = run_prompt_yes_no(
        /* noninteractive */ true,
        /* answer */ None,
        Duration::from_secs(10),
    )
    .await?;

    assert!(
        exited,
        "with OPEN_INTERPRETER_NONINTERACTIVE set, prompt_yes_no must return immediately \
         instead of blocking on /dev/tty. output:\n{output}"
    );
    assert!(
        !output.contains("Start Open Interpreter now?"),
        "the launch prompt must not be rendered in non-interactive mode. output:\n{output}"
    );
    assert!(
        output.contains("__BRANCH_NO__"),
        "prompt_yes_no should take the 'no' branch. output:\n{output}"
    );
    Ok(())
}

#[tokio::test]
async fn interactive_launch_prompt_renders_and_blocks_on_tty() -> anyhow::Result<()> {
    if cfg!(windows) {
        return Ok(());
    }

    // Without the env var and without input, the prompt must render to the
    // terminal and block on the /dev/tty read (the behavior the guard fixes).
    let (exited, output) = run_prompt_yes_no(
        /* noninteractive */ false,
        /* answer */ None,
        Duration::from_secs(4),
    )
    .await?;
    assert!(
        output.contains("Start Open Interpreter now?"),
        "the interactive prompt should render to the terminal. output:\n{output}"
    );
    assert!(
        !exited,
        "without the env var, prompt_yes_no must block waiting for /dev/tty input. output:\n{output}"
    );

    // When the prompt is answered, the script proceeds and finishes.
    let (exited, output) = run_prompt_yes_no(
        /* noninteractive */ false,
        /* answer */ Some(b"n\n"),
        Duration::from_secs(10),
    )
    .await?;
    assert!(
        exited,
        "after answering the prompt the script should finish. output:\n{output}"
    );
    assert!(output.contains("__BRANCH_NO__"), "output:\n{output}");
    Ok(())
}
