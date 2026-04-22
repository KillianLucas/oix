use std::collections::HashMap;
use std::path::Path;

use crate::exec::ExecCapturePolicy;
use crate::exec::ExecParams;
use crate::exec_env::create_env;
use crate::exec_policy::ExecApprovalRequest;
use crate::function_tool::FunctionCallError;
use crate::session::Session;
use crate::session::TurnContext;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::events::ToolEmitter;
use crate::tools::events::ToolEventCtx;
use crate::tools::events::ToolEventFailure;
use crate::tools::events::ToolEventStage;
use crate::tools::handlers::apply_granted_turn_permissions;
use crate::tools::handlers::parse_arguments;
use crate::tools::handlers::plan::handle_update_plan;
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
use codex_protocol::plan_tool::PlanItemArg;
use codex_protocol::plan_tool::StepStatus;
use codex_protocol::plan_tool::UpdatePlanArgs;
use codex_protocol::protocol::ExecCommandSource;
use codex_protocol::protocol::SessionSource;
use codex_protocol::request_user_input::RequestUserInputArgs;
use codex_protocol::request_user_input::RequestUserInputQuestion;
use codex_protocol::request_user_input::RequestUserInputQuestionOption;
use codex_sandboxing::policy_transforms::effective_file_system_sandbox_policy;
use codex_sandboxing::policy_transforms::merge_permission_profiles;
use codex_tools::normalize_request_user_input_args;
use codex_tools::request_user_input_unavailable_message;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde::Deserialize;
use serde_json::Value as JsonValue;

pub struct ClaudeAskUserQuestionHandler;
pub struct ClaudeBashHandler;
pub struct ClaudeEditHandler;
pub struct ClaudeReadHandler;
pub struct ClaudeTodoWriteHandler;
pub struct ClaudeWriteHandler;

const CLAUDE_BASH_EMPTY_OUTPUT: &str = "(Bash completed with no output)";
const CLAUDE_BASH_DEFAULT_TIMEOUT_MS: u64 = 120_000;
const CLAUDE_BASH_MAX_TIMEOUT_MS: u64 = 600_000;
pub(crate) const CLAUDE_TODO_WRITE_SUCCESS_MESSAGE: &str = "Todos have been modified successfully. Ensure that you continue to use the todo list to track your progress. Please proceed with the current tasks if applicable";

#[derive(Deserialize)]
struct ClaudeReadArgs {
    file_path: String,
    offset: Option<usize>,
    limit: Option<usize>,
    #[allow(dead_code)]
    pages: Option<String>,
}

#[derive(Deserialize)]
struct ClaudeWriteArgs {
    file_path: String,
    content: String,
}

#[derive(Deserialize)]
struct ClaudeEditArgs {
    file_path: String,
    old_string: String,
    new_string: String,
    replace_all: Option<bool>,
}

#[derive(Deserialize)]
struct ClaudeBashArgs {
    command: String,
    description: Option<String>,
    timeout: Option<u64>,
    run_in_background: Option<bool>,
}

#[derive(Deserialize)]
struct ClaudeTodoWriteArgs {
    todos: Vec<ClaudeTodoItem>,
}

#[derive(Deserialize)]
struct ClaudeTodoItem {
    content: String,
    status: ClaudeTodoStatus,
    #[serde(rename = "activeForm")]
    active_form: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ClaudeTodoStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Deserialize)]
struct ClaudeAskUserQuestionArgs {
    questions: Vec<ClaudeAskUserQuestionItem>,
}

#[derive(Deserialize)]
struct ClaudeAskUserQuestionItem {
    question: String,
    header: String,
    options: Vec<ClaudeAskUserQuestionOption>,
    #[serde(rename = "multiSelect", default)]
    multi_select: bool,
}

#[derive(Deserialize)]
struct ClaudeAskUserQuestionOption {
    label: String,
    description: String,
    #[allow(dead_code)]
    preview: Option<String>,
}

impl ToolHandler for ClaudeReadHandler {
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
                "Read received unsupported payload".to_string(),
            ));
        };
        let args: ClaudeReadArgs = parse_arguments(&arguments)?;
        if args.pages.is_some() {
            return Err(FunctionCallError::RespondToModel(
                "PDF page selection is not implemented for the claude-code harness yet."
                    .to_string(),
            ));
        }
        let path = parse_absolute_path(&args.file_path)?;
        let file_system_policy =
            effective_turn_file_system_policy(session.as_ref(), turn.as_ref()).await;
        ensure_readable_path(&file_system_policy, turn.as_ref(), &path)?;
        let content = tokio::fs::read_to_string(path.as_path())
            .await
            .map_err(|err| FunctionCallError::RespondToModel(format!("Read failed: {err}")))?;
        let formatted = format_read_output(
            &content,
            args.offset.unwrap_or(1),
            args.limit.unwrap_or(2000),
        );
        Ok(FunctionToolOutput::from_text(formatted, Some(true)))
    }
}

impl ToolHandler for ClaudeAskUserQuestionHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            call_id,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "AskUserQuestion received unsupported payload".to_string(),
            ));
        };

        if matches!(turn.session_source, SessionSource::SubAgent(_)) {
            return Err(FunctionCallError::RespondToModel(
                "AskUserQuestion can only be used by the root thread".to_string(),
            ));
        }

        let mode = session.collaboration_mode().await.mode;
        if let Some(message) = request_user_input_unavailable_message(
            mode,
            turn.tools_config.default_mode_request_user_input,
        ) {
            return Err(FunctionCallError::RespondToModel(message));
        }

        let args = normalize_claude_ask_user_question_args(parse_arguments(&arguments)?)?;
        let response = session
            .request_user_input(turn.as_ref(), call_id, args)
            .await
            .ok_or_else(|| {
                FunctionCallError::RespondToModel(
                    "AskUserQuestion was cancelled before receiving a response".to_string(),
                )
            })?;

        let content = serde_json::to_string(&response).map_err(|err| {
            FunctionCallError::Fatal(format!(
                "failed to serialize AskUserQuestion response: {err}"
            ))
        })?;

        Ok(FunctionToolOutput::from_text(content, Some(true)))
    }
}

impl ToolHandler for ClaudeWriteHandler {
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
                "Write received unsupported payload".to_string(),
            ));
        };
        let args: ClaudeWriteArgs = parse_arguments(&arguments)?;
        let path = parse_absolute_path(&args.file_path)?;
        let file_system_policy =
            effective_turn_file_system_policy(session.as_ref(), turn.as_ref()).await;
        ensure_writable_path(&file_system_policy, turn.as_ref(), &path)?;
        tokio::fs::write(path.as_path(), args.content)
            .await
            .map_err(|err| FunctionCallError::RespondToModel(format!("Write failed: {err}")))?;
        Ok(FunctionToolOutput::from_text(
            format!("File created successfully at: {}", path.display()),
            Some(true),
        ))
    }
}

impl ToolHandler for ClaudeTodoWriteHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            call_id,
            payload,
            ..
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "TodoWrite received unsupported payload".to_string(),
            ));
        };

        let args: ClaudeTodoWriteArgs = parse_arguments(&arguments)?;
        let update_plan = UpdatePlanArgs {
            explanation: None,
            plan: args
                .todos
                .into_iter()
                .map(|todo| {
                    let _ = todo.active_form;
                    PlanItemArg {
                        step: todo.content,
                        status: match todo.status {
                            ClaudeTodoStatus::Pending => StepStatus::Pending,
                            ClaudeTodoStatus::InProgress => StepStatus::InProgress,
                            ClaudeTodoStatus::Completed => StepStatus::Completed,
                        },
                    }
                })
                .collect(),
        };
        let arguments = serde_json::to_string(&update_plan).map_err(|err| {
            FunctionCallError::Fatal(format!(
                "failed to serialize TodoWrite arguments as update_plan: {err}"
            ))
        })?;
        handle_update_plan(session.as_ref(), turn.as_ref(), arguments, call_id).await?;
        Ok(FunctionToolOutput::from_text(
            CLAUDE_TODO_WRITE_SUCCESS_MESSAGE.to_string(),
            Some(true),
        ))
    }
}

fn normalize_claude_ask_user_question_args(
    args: ClaudeAskUserQuestionArgs,
) -> Result<RequestUserInputArgs, FunctionCallError> {
    let mut question_ids = HashMap::<String, usize>::new();
    let request = RequestUserInputArgs {
        questions: args
            .questions
            .into_iter()
            .enumerate()
            .map(|(index, question)| {
                if question.multi_select {
                    return Err(FunctionCallError::RespondToModel(
                        "AskUserQuestion multiSelect is not implemented for the claude-code harness yet."
                            .to_string(),
                    ));
                }

                let base_id = slugify_identifier(&question.header);
                let next_index = question_ids.entry(base_id.clone()).or_insert(0);
                let id = if *next_index == 0 {
                    base_id
                } else {
                    format!("{base_id}_{}", *next_index + 1)
                };
                *next_index += 1;

                Ok(RequestUserInputQuestion {
                    id: if id.is_empty() {
                        format!("question_{}", index + 1)
                    } else {
                        id
                    },
                    header: question.header,
                    question: question.question,
                    is_other: true,
                    is_secret: false,
                    options: Some(
                        question
                            .options
                            .into_iter()
                            .map(|option| RequestUserInputQuestionOption {
                                label: option.label,
                                description: option.description,
                            })
                            .collect(),
                    ),
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
    };
    normalize_request_user_input_args(request).map_err(FunctionCallError::RespondToModel)
}

fn slugify_identifier(input: &str) -> String {
    let mut slug = String::new();
    let mut last_was_underscore = false;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_underscore = false;
        } else if !last_was_underscore && !slug.is_empty() {
            slug.push('_');
            last_was_underscore = true;
        }
    }
    while slug.ends_with('_') {
        slug.pop();
    }
    slug
}

impl ToolHandler for ClaudeEditHandler {
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
                "Edit received unsupported payload".to_string(),
            ));
        };
        let args: ClaudeEditArgs = parse_arguments(&arguments)?;
        if args.old_string == args.new_string {
            return Err(FunctionCallError::RespondToModel(
                "old_string and new_string must differ".to_string(),
            ));
        }
        let path = parse_absolute_path(&args.file_path)?;
        let file_system_policy =
            effective_turn_file_system_policy(session.as_ref(), turn.as_ref()).await;
        ensure_readable_path(&file_system_policy, turn.as_ref(), &path)?;
        ensure_writable_path(&file_system_policy, turn.as_ref(), &path)?;
        let content = tokio::fs::read_to_string(path.as_path())
            .await
            .map_err(|err| FunctionCallError::RespondToModel(format!("Edit failed: {err}")))?;
        let updated = replace_exact_text(&content, &args)?;
        tokio::fs::write(path.as_path(), updated)
            .await
            .map_err(|err| FunctionCallError::RespondToModel(format!("Edit failed: {err}")))?;
        let message = if args.replace_all.unwrap_or(false) {
            format!(
                "The file {} has been updated. All occurrences were successfully replaced.",
                path.display()
            )
        } else {
            format!("The file {} has been updated.", path.display())
        };
        Ok(FunctionToolOutput::from_text(message, Some(true)))
    }
}

impl ToolHandler for ClaudeBashHandler {
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
            tracker: _tracker,
            call_id,
            tool_name,
            payload,
        } = invocation;
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "Bash received unsupported payload".to_string(),
            ));
        };
        let args: ClaudeBashArgs = parse_arguments(&arguments)?;
        if args.run_in_background.unwrap_or(false) {
            return Err(FunctionCallError::RespondToModel(
                "run_in_background is not implemented for the claude-code harness yet.".to_string(),
            ));
        }
        let Some(_environment) = turn.environment.as_ref() else {
            return Err(FunctionCallError::RespondToModel(
                "Bash is unavailable in this session".to_string(),
            ));
        };

        let timeout_ms = Some(
            args.timeout
                .unwrap_or(CLAUDE_BASH_DEFAULT_TIMEOUT_MS)
                .min(CLAUDE_BASH_MAX_TIMEOUT_MS),
        );
        let command = session
            .user_shell()
            .derive_exec_args(&args.command, turn.tools_config.allow_login_shell);
        let exec_params = ExecParams {
            command: command.clone(),
            cwd: turn.cwd.clone(),
            expiration: timeout_ms.into(),
            capture_policy: ExecCapturePolicy::ShellTool,
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
            justification: args.description.clone(),
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

        let effective_permissions =
            apply_granted_turn_permissions(session.as_ref(), SandboxPermissions::UseDefault, None)
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
            env: exec_params.env.clone(),
            explicit_env_overrides: HashMap::new(),
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
                let text = bash_output_text(&output, turn.as_ref());
                if output.exit_code == 0 {
                    Ok(FunctionToolOutput {
                        body: vec![
                            codex_protocol::models::FunctionCallOutputContentItem::InputText {
                                text: text.clone(),
                            },
                        ],
                        success: Some(true),
                        post_tool_use_response: Some(JsonValue::String(text)),
                    })
                } else {
                    Err(FunctionCallError::RespondToModel(text))
                }
            }
            Err(ToolError::Codex(CodexErr::Sandbox(SandboxErr::Timeout { output })))
            | Err(ToolError::Codex(CodexErr::Sandbox(SandboxErr::Denied { output, .. }))) => {
                let output = *output;
                emitter
                    .emit(
                        event_ctx,
                        ToolEventStage::Failure(ToolEventFailure::Output(output.clone())),
                    )
                    .await;
                Err(FunctionCallError::RespondToModel(bash_output_text(
                    &output,
                    turn.as_ref(),
                )))
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

pub(super) fn parse_absolute_path(path: &str) -> Result<AbsolutePathBuf, FunctionCallError> {
    if !Path::new(path).is_absolute() {
        return Err(FunctionCallError::RespondToModel(
            "file_path must be an absolute path".to_string(),
        ));
    }
    AbsolutePathBuf::try_from(path.to_string()).map_err(|err| {
        FunctionCallError::RespondToModel(format!("invalid file_path `{path}`: {err}"))
    })
}

pub(super) async fn effective_turn_file_system_policy(
    session: &Session,
    turn: &TurnContext,
) -> codex_protocol::permissions::FileSystemSandboxPolicy {
    let granted_permissions = merge_permission_profiles(
        session.granted_session_permissions().await.as_ref(),
        session.granted_turn_permissions().await.as_ref(),
    );
    effective_file_system_sandbox_policy(
        &turn.file_system_sandbox_policy,
        granted_permissions.as_ref(),
    )
}

pub(super) fn ensure_readable_path(
    file_system_policy: &codex_protocol::permissions::FileSystemSandboxPolicy,
    turn: &TurnContext,
    path: &AbsolutePathBuf,
) -> Result<(), FunctionCallError> {
    if file_system_policy.can_read_path_with_cwd(path.as_path(), turn.cwd.as_path()) {
        Ok(())
    } else {
        Err(FunctionCallError::RespondToModel(format!(
            "Read is not allowed for {} in this session.",
            path.display()
        )))
    }
}

fn ensure_writable_path(
    file_system_policy: &codex_protocol::permissions::FileSystemSandboxPolicy,
    turn: &TurnContext,
    path: &AbsolutePathBuf,
) -> Result<(), FunctionCallError> {
    let writable_target = path.parent().unwrap_or_else(|| path.clone());
    if file_system_policy.can_write_path_with_cwd(writable_target.as_path(), turn.cwd.as_path()) {
        Ok(())
    } else {
        Err(FunctionCallError::RespondToModel(format!(
            "Write is not allowed for {} in this session.",
            path.display()
        )))
    }
}

fn format_read_output(content: &str, offset: usize, limit: usize) -> String {
    let start_index = offset.saturating_sub(1);
    content
        .split('\n')
        .enumerate()
        .skip(start_index)
        .take(limit)
        .map(|(index, line)| format!("{}\t{line}", index + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

fn replace_exact_text(content: &str, args: &ClaudeEditArgs) -> Result<String, FunctionCallError> {
    let replace_all = args.replace_all.unwrap_or(false);
    if replace_all {
        if !content.contains(&args.old_string) {
            return Err(FunctionCallError::RespondToModel(
                "old_string was not found in the file".to_string(),
            ));
        }
        return Ok(content.replace(&args.old_string, &args.new_string));
    }

    let match_count = content.match_indices(&args.old_string).count();
    match match_count {
        0 => Err(FunctionCallError::RespondToModel(
            "old_string was not found in the file".to_string(),
        )),
        1 => Ok(content.replacen(&args.old_string, &args.new_string, 1)),
        _ => Err(FunctionCallError::RespondToModel(
            "old_string is not unique in the file; provide more context or set replace_all to true"
                .to_string(),
        )),
    }
}

fn bash_output_text(
    output: &codex_protocol::exec_output::ExecToolCallOutput,
    turn: &TurnContext,
) -> String {
    let text = crate::tools::format_exec_output_str(output, turn.truncation_policy);
    if text.is_empty() {
        CLAUDE_BASH_EMPTY_OUTPUT.to_string()
    } else {
        text
    }
}

#[cfg(test)]
#[path = "claude_code_tests.rs"]
mod tests;
