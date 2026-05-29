use crate::exec::ExecCapturePolicy;
use crate::exec::ExecParams;
use crate::exec_env::create_env;
use crate::exec_policy::ExecApprovalRequest;
use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::events::ToolEmitter;
use crate::tools::events::ToolEventCtx;
use crate::tools::events::ToolEventFailure;
use crate::tools::events::ToolEventStage;
use crate::tools::handlers::apply_granted_turn_permissions;
use crate::tools::handlers::claude_code::effective_turn_file_system_policy;
use crate::tools::handlers::claude_code::ensure_readable_path;
use crate::tools::handlers::claude_code::ensure_writable_path;
use crate::tools::handlers::parse_arguments;
use crate::tools::orchestrator::ToolOrchestrator;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use crate::tools::runtimes::shell::ShellRequest;
use crate::tools::runtimes::shell::ShellRuntime;
use crate::tools::runtimes::shell::ShellRuntimeBackend;
use crate::tools::sandboxing::ToolError;
use codex_protocol::error::CodexErr;
use codex_protocol::error::SandboxErr;
use codex_protocol::models::SandboxPermissions;
use codex_protocol::protocol::ExecCommandSource;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde::Deserialize;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

pub struct PiBashHandler;
pub struct PiEditHandler;
pub struct PiReadHandler;
pub struct PiWriteHandler;

const PI_READ_MAX_LINES: usize = 2000;
const PI_READ_MAX_BYTES: usize = 50 * 1024;
const PI_BASH_MAX_TIMEOUT_MS: u64 = 600_000;

#[derive(Deserialize)]
struct PiReadArgs {
    path: String,
    offset: Option<usize>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct PiWriteArgs {
    path: String,
    content: String,
}

#[derive(Deserialize)]
struct PiEditArgs {
    path: String,
    edits: Vec<PiEditEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PiEditEntry {
    old_text: String,
    new_text: String,
}

#[derive(Deserialize)]
struct PiBashArgs {
    command: String,
    timeout: Option<u64>,
}

impl ToolHandler for PiReadHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "read received unsupported payload".to_string(),
            ));
        };
        let args: PiReadArgs = parse_arguments(&arguments)?;
        let path = resolve_path(turn.cwd.as_path(), &args.path)?;
        let file_system_policy =
            effective_turn_file_system_policy(session.as_ref(), turn.as_ref()).await;
        ensure_readable_path(&file_system_policy, turn.as_ref(), &path)?;
        let content = tokio::fs::read_to_string(path.as_path())
            .await
            .map_err(|err| FunctionCallError::RespondToModel(pi_read_error(&args.path, err)))?;
        Ok(FunctionToolOutput::from_text(
            pi_read_output(&content, args.offset.unwrap_or(1), args.limit),
            Some(true),
        ))
    }
}

impl ToolHandler for PiWriteHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn is_mutating(&self, _invocation: &ToolInvocation) -> bool {
        true
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "write received unsupported payload".to_string(),
            ));
        };
        let args: PiWriteArgs = parse_arguments(&arguments)?;
        let path = resolve_path(turn.cwd.as_path(), &args.path)?;
        let file_system_policy =
            effective_turn_file_system_policy(session.as_ref(), turn.as_ref()).await;
        ensure_writable_path(&file_system_policy, turn.as_ref(), &path)?;
        if let Some(parent) = path.as_path().parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|err| FunctionCallError::RespondToModel(format!("Write failed: {err}")))?;
        }
        let bytes = args.content.len();
        tokio::fs::write(path.as_path(), args.content)
            .await
            .map_err(|err| FunctionCallError::RespondToModel(format!("Write failed: {err}")))?;
        Ok(FunctionToolOutput::from_text(
            format!("Successfully wrote {bytes} bytes to {}", path.display()),
            Some(true),
        ))
    }
}

impl ToolHandler for PiEditHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn is_mutating(&self, _invocation: &ToolInvocation) -> bool {
        true
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "edit received unsupported payload".to_string(),
            ));
        };
        let args: PiEditArgs = parse_arguments(&arguments)?;
        let path = resolve_path(turn.cwd.as_path(), &args.path)?;
        let file_system_policy =
            effective_turn_file_system_policy(session.as_ref(), turn.as_ref()).await;
        ensure_readable_path(&file_system_policy, turn.as_ref(), &path)?;
        ensure_writable_path(&file_system_policy, turn.as_ref(), &path)?;
        let content = tokio::fs::read_to_string(path.as_path())
            .await
            .map_err(|err| FunctionCallError::RespondToModel(format!("Edit failed: {err}")))?;
        let updated = apply_pi_edits(&content, &args.edits)?;
        tokio::fs::write(path.as_path(), updated)
            .await
            .map_err(|err| FunctionCallError::RespondToModel(format!("Edit failed: {err}")))?;
        Ok(FunctionToolOutput::from_text(
            format!(
                "Successfully replaced {} block(s) in {}.",
                args.edits.len(),
                args.path
            ),
            Some(true),
        ))
    }
}

impl ToolHandler for PiBashHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn is_mutating(&self, _invocation: &ToolInvocation) -> bool {
        true
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            call_id,
            tool_name,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "bash received unsupported payload".to_string(),
            ));
        };
        let args: PiBashArgs = parse_arguments(&arguments)?;
        let Some(_environment) = turn.environment.as_ref() else {
            return Err(FunctionCallError::RespondToModel(
                "bash is unavailable in this session".to_string(),
            ));
        };
        let timeout_ms = args
            .timeout
            .map(|timeout| timeout.saturating_mul(1000).min(PI_BASH_MAX_TIMEOUT_MS));
        let command = session
            .user_shell()
            .derive_exec_args(&args.command, turn.tools_config.allow_login_shell);
        let exec_params = ExecParams {
            command: command.clone(),
            cwd: turn.cwd.clone(),
            expiration: timeout_ms.into(),
            capture_policy: ExecCapturePolicy::ShellToolFullOutput,
            env: create_env(
                &turn.shell_environment_policy,
                Some(session.conversation_id),
            ),
            network: turn.network.clone(),
            sandbox_permissions: SandboxPermissions::UseDefault,
            windows_sandbox_level: turn.windows_sandbox_level,
            windows_sandbox_private_desktop: turn
                .config
                .permissions
                .windows_sandbox_private_desktop,
            justification: None,
            arg0: None,
        };
        let emitter = ToolEmitter::shell(
            exec_params.command.clone(),
            exec_params.cwd.clone(),
            ExecCommandSource::Agent,
            /*freeform*/ false,
        );
        let event_ctx = ToolEventCtx::new(
            session.as_ref(),
            turn.as_ref(),
            &call_id,
            /*turn_diff_tracker*/ None,
        );
        emitter.begin(event_ctx).await;
        let effective_permissions = apply_granted_turn_permissions(
            session.as_ref(),
            turn.cwd.as_path(),
            SandboxPermissions::UseDefault,
            None,
        )
        .await;
        let exec_approval_requirement = session
            .services
            .exec_policy
            .create_exec_approval_requirement_for_command(ExecApprovalRequest {
                command: &exec_params.command,
                approval_policy: turn.approval_policy.value(),
                sandbox_policy: turn.sandbox_policy.get(),
                file_system_sandbox_policy: &turn.file_system_sandbox_policy,
                sandbox_permissions: if effective_permissions.permissions_preapproved {
                    SandboxPermissions::UseDefault
                } else {
                    effective_permissions.sandbox_permissions
                },
                prefix_rule: None,
            })
            .await;
        let request = ShellRequest {
            command: exec_params.command.clone(),
            hook_command: codex_shell_command::parse_command::shlex_join(&exec_params.command),
            cwd: exec_params.cwd.clone(),
            timeout_ms,
            capture_policy: exec_params.capture_policy,
            env: exec_params.env.clone(),
            explicit_env_overrides: turn.shell_environment_policy.r#set.clone(),
            network: exec_params.network.clone(),
            sandbox_permissions: effective_permissions.sandbox_permissions,
            additional_permissions: None,
            #[cfg(unix)]
            additional_permissions_preapproved: effective_permissions.permissions_preapproved,
            justification: exec_params.justification.clone(),
            exec_approval_requirement,
        };
        let mut orchestrator = ToolOrchestrator::new();
        let mut runtime = ShellRuntime::for_shell_command(ShellRuntimeBackend::ShellCommandClassic);
        let tool_ctx = crate::tools::sandboxing::ToolCtx {
            session: session.clone(),
            turn: turn.clone(),
            call_id: call_id.clone(),
            tool_name: tool_name.display(),
        };
        let result = orchestrator
            .run(
                &mut runtime,
                &request,
                &tool_ctx,
                &turn,
                turn.approval_policy.value(),
            )
            .await
            .map(|output| output.output);
        match result {
            Ok(output) => {
                emitter
                    .emit(event_ctx, ToolEventStage::Success(output.clone()))
                    .await;
                let text = pi_bash_output_text(&output.aggregated_output.text);
                if output.exit_code == 0 {
                    Ok(FunctionToolOutput::from_text(text, Some(true)))
                } else {
                    Err(FunctionCallError::RespondToModel(text))
                }
            }
            Err(ToolError::Codex(CodexErr::Sandbox(SandboxErr::Timeout { output })))
            | Err(ToolError::Codex(CodexErr::Sandbox(SandboxErr::Denied { output, .. }))) => {
                emitter
                    .emit(
                        event_ctx,
                        ToolEventStage::Failure(ToolEventFailure::Output((*output).clone())),
                    )
                    .await;
                Err(FunctionCallError::RespondToModel(
                    crate::tools::format_exec_output_str(output.as_ref(), turn.truncation_policy),
                ))
            }
            Err(ToolError::Rejected(message)) => {
                emitter
                    .emit(
                        event_ctx,
                        ToolEventStage::Failure(ToolEventFailure::Rejected(message.clone())),
                    )
                    .await;
                Err(FunctionCallError::RespondToModel(message))
            }
            Err(ToolError::Codex(err)) => {
                let message = format!("execution error: {err:?}");
                emitter
                    .emit(
                        event_ctx,
                        ToolEventStage::Failure(ToolEventFailure::Message(message.clone())),
                    )
                    .await;
                Err(FunctionCallError::RespondToModel(message))
            }
        }
    }
}

fn resolve_path(cwd: &Path, raw: &str) -> Result<AbsolutePathBuf, FunctionCallError> {
    let path = PathBuf::from(raw);
    let path = if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    };
    AbsolutePathBuf::try_from(path)
        .map_err(|_| FunctionCallError::RespondToModel(format!("Invalid path: {raw}")))
}

fn pi_read_error(path: &str, err: std::io::Error) -> String {
    match err.kind() {
        ErrorKind::NotFound => format!("ENOENT: no such file or directory, access '{path}'"),
        _ => err.to_string(),
    }
}

fn pi_read_output(content: &str, offset: usize, limit: Option<usize>) -> String {
    let start = offset.saturating_sub(1);
    let limit = limit.unwrap_or(PI_READ_MAX_LINES);
    let mut output = String::new();
    let mut bytes = 0usize;
    for (shown, line) in content.split_inclusive('\n').skip(start).enumerate() {
        if shown >= limit || bytes + line.len() > PI_READ_MAX_BYTES {
            break;
        }
        output.push_str(line);
        bytes += line.len();
    }
    output
}

fn apply_pi_edits(content: &str, edits: &[PiEditEntry]) -> Result<String, FunctionCallError> {
    let mut output = content.to_string();
    for edit in edits {
        if edit.old_text.is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "Edit failed: oldText must be non-empty".to_string(),
            ));
        }
        let matches = content.match_indices(&edit.old_text).count();
        if matches == 0 {
            return Err(FunctionCallError::RespondToModel(
                "Edit failed: oldText not found".to_string(),
            ));
        }
        if matches > 1 {
            return Err(FunctionCallError::RespondToModel(
                "Edit failed: oldText found multiple times".to_string(),
            ));
        }
        output = output.replacen(&edit.old_text, &edit.new_text, 1);
    }
    Ok(output)
}

fn pi_bash_output_text(output: &str) -> String {
    if output.is_empty() {
        "(no output)".to_string()
    } else {
        output.to_string()
    }
}
