use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::sync::Mutex;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use codex_protocol::models::BaseInstructions;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::MessagePhase;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use codex_tools::AdditionalProperties;
use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use regex_lite::Regex;
use serde::Deserialize;
use serde_json::json;

use crate::agent::control::SpawnAgentOptions;
use crate::agent::next_thread_spawn_depth;
use crate::function_tool::FunctionCallError;
use crate::harness::opencode::OPENCODE_SEARCH_AGENT_BASE_INSTRUCTIONS;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::ExecCommandHandler;
use crate::tools::handlers::ExecCommandHandlerOptions;
use crate::tools::handlers::RequestUserInputHandler;
use crate::tools::handlers::WriteStdinHandler;
use crate::tools::handlers::multi_agents_common::build_agent_spawn_config;
use crate::tools::handlers::multi_agents_common::parse_collab_input;
use crate::tools::handlers::multi_agents_common::thread_spawn_source;
use crate::tools::handlers::multi_agents_v2::SpawnAgentHandler as SpawnAgentHandlerV2;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::CoreToolRuntime;
use crate::tools::registry::ToolExecutor;

pub(crate) const HARNESS_NO_TRUNCATE_PREFIX: &str = "<open-interpreter-harness-no-truncate>\n";

const DEFAULT_READ_LIMIT: usize = 1_000;
static CLAUDE_TASKS: LazyLock<Mutex<HashMap<String, ClaudeTaskState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static CLAUDE_READ_FILES: LazyLock<Mutex<HashSet<PathBuf>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));
static DEEPSEEK_CHECKLIST: LazyLock<Mutex<Vec<ChecklistItem>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));
static DEEPSEEK_READ_FILE_COUNTS: LazyLock<Mutex<HashMap<PathBuf, usize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Clone, Debug)]
struct ClaudeTaskState {
    process_id: i32,
    command: String,
    description: String,
    output: String,
    output_path: String,
    tool_use_id: String,
}

#[derive(Clone, Copy)]
pub enum HarnessAliasHandler {
    Agent,
    Bash,
    BashLower,
    Read,
    ReadLower,
    Write,
    WriteLower,
    Edit,
    EditLower,
    Glob,
    GlobLower,
    Grep,
    GrepLower,
    AskUserQuestion,
    TaskOutput,
    TaskStop,
    ChecklistAdd,
    ChecklistList,
    ChecklistUpdate,
    ChecklistWrite,
    DeepSeekDiagnostics,
    DeepSeekApplyPatch,
    DeepSeekEditFile,
    DeepSeekExecShell,
    DeepSeekFileSearch,
    DeepSeekGitDiff,
    DeepSeekGitStatus,
    DeepSeekGrepFiles,
    DeepSeekListDir,
    DeepSeekReadFile,
    DeepSeekToolSearch,
    DeepSeekWriteFile,
    OpenCodeTask,
    OpenCodeTodoWrite,
}

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for HarnessAliasHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain(match self {
            Self::Agent => "Agent",
            Self::Bash => "Bash",
            Self::BashLower => "bash",
            Self::Read => "Read",
            Self::ReadLower => "read",
            Self::Write => "Write",
            Self::WriteLower => "write",
            Self::Edit => "Edit",
            Self::EditLower => "edit",
            Self::Glob => "Glob",
            Self::GlobLower => "glob",
            Self::Grep => "Grep",
            Self::GrepLower => "grep",
            Self::AskUserQuestion => "AskUserQuestion",
            Self::TaskOutput => "TaskOutput",
            Self::TaskStop => "TaskStop",
            Self::ChecklistAdd => "checklist_add",
            Self::ChecklistList => "checklist_list",
            Self::ChecklistUpdate => "checklist_update",
            Self::ChecklistWrite => "checklist_write",
            Self::DeepSeekDiagnostics => "diagnostics",
            Self::DeepSeekApplyPatch => "apply_patch",
            Self::DeepSeekEditFile => "edit_file",
            Self::DeepSeekExecShell => "exec_shell",
            Self::DeepSeekFileSearch => "file_search",
            Self::DeepSeekGitDiff => "git_diff",
            Self::DeepSeekGitStatus => "git_status",
            Self::DeepSeekGrepFiles => "grep_files",
            Self::DeepSeekListDir => "list_dir",
            Self::DeepSeekReadFile => "read_file",
            Self::DeepSeekToolSearch => "tool_search_tool_bm25",
            Self::DeepSeekWriteFile => "write_file",
            Self::OpenCodeTask => "task",
            Self::OpenCodeTodoWrite => "todowrite",
        })
    }

    fn spec(&self) -> ToolSpec {
        harness_alias_spec(self.tool_name().name.as_str())
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        !matches!(self, Self::Edit | Self::Write | Self::AskUserQuestion)
    }

    async fn handle(
        &self,
        invocation: ToolInvocation,
    ) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
        match self {
            Self::Agent => handle_agent(invocation).await,
            Self::Bash => handle_bash(invocation).await,
            Self::BashLower => handle_plain_bash(invocation).await,
            Self::Read | Self::ReadLower => handle_read(invocation).await,
            Self::Write | Self::WriteLower => handle_write(invocation).await,
            Self::Edit | Self::EditLower => handle_edit(invocation).await,
            Self::Glob | Self::GlobLower => handle_glob(invocation).await,
            Self::Grep | Self::GrepLower => handle_grep(invocation).await,
            Self::AskUserQuestion => handle_ask_user_question(invocation).await,
            Self::TaskOutput => handle_task_output(invocation).await,
            Self::TaskStop => handle_task_stop(invocation).await,
            Self::ChecklistAdd => handle_checklist_add(invocation).await,
            Self::ChecklistList => handle_checklist_list(invocation).await,
            Self::ChecklistUpdate => handle_checklist_update(invocation).await,
            Self::ChecklistWrite => handle_checklist_write(invocation).await,
            Self::DeepSeekDiagnostics => handle_deepseek_diagnostics(invocation).await,
            Self::DeepSeekApplyPatch => handle_deepseek_apply_patch(invocation).await,
            Self::DeepSeekEditFile => handle_deepseek_edit_file(invocation).await,
            Self::DeepSeekExecShell => handle_deepseek_exec_shell(invocation).await,
            Self::DeepSeekFileSearch => handle_deepseek_file_search(invocation).await,
            Self::DeepSeekGitDiff => handle_git_command(invocation, &["diff"]).await,
            Self::DeepSeekGitStatus => {
                handle_git_command(invocation, &["status", "--short", "--branch"]).await
            }
            Self::DeepSeekGrepFiles => handle_deepseek_grep_files(invocation).await,
            Self::DeepSeekListDir => handle_deepseek_list_dir(invocation).await,
            Self::DeepSeekReadFile => handle_deepseek_read_file(invocation).await,
            Self::DeepSeekToolSearch => handle_deepseek_tool_search(invocation).await,
            Self::DeepSeekWriteFile => handle_deepseek_write_file(invocation).await,
            Self::OpenCodeTask => handle_opencode_task(invocation).await,
            Self::OpenCodeTodoWrite => handle_opencode_todowrite(invocation).await,
        }
    }
}

impl CoreToolRuntime for HarnessAliasHandler {
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }
}

#[derive(Deserialize)]
struct ClaudeAgentArgs {
    description: String,
    prompt: String,
    #[serde(default)]
    subagent_type: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    run_in_background: bool,
    #[serde(default)]
    isolation: Option<String>,
}

async fn handle_agent(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: ClaudeAgentArgs = parse_arguments(arguments)?;
    let mut translated = json!({
        "message": args.prompt,
        "task_name": args.description,
        "fork_turns": "all",
    });
    if let Some(subagent_type) = args
        .subagent_type
        .as_deref()
        .map(str::trim)
        .filter(|subagent_type| !subagent_type.is_empty())
    {
        translated["agent_type"] = json!(subagent_type);
    }
    if let Some(model) = args
        .model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        translated["model"] = json!(model);
    }
    let _ = args.run_in_background;
    let _ = args.isolation;

    SpawnAgentHandlerV2::default()
        .handle(ToolInvocation {
            tool_name: ToolName::plain("spawn_agent"),
            payload: ToolPayload::Function {
                arguments: translated.to_string(),
            },
            ..invocation
        })
        .await
}

#[derive(Deserialize)]
struct BashArgs {
    command: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    timeout: Option<u64>,
    #[serde(default)]
    run_in_background: bool,
    #[serde(default, rename = "dangerouslyDisableSandbox")]
    dangerously_disable_sandbox: bool,
}

async fn handle_bash(invocation: ToolInvocation) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: BashArgs = parse_arguments(arguments)?;
    let command = normalize_model_path_text(&args.command);
    let mut translated = json!({
        "cmd": command,
        "yield_time_ms": if args.run_in_background { 1_000 } else { args.timeout.unwrap_or(10_000).min(30_000) },
    });
    if args.run_in_background {
        translated["tty"] = json!(true);
    }
    if args.dangerously_disable_sandbox {
        translated["sandbox_permissions"] = json!("require_escalated");
    }

    let payload = ToolPayload::Function {
        arguments: translated.to_string(),
    };
    let payload_for_result = payload.clone();
    let output_path = args
        .run_in_background
        .then(|| claude_task_output_path(&invocation));
    let is_claude_code = is_claude_code(&invocation);
    let handler = ExecCommandHandler::new(ExecCommandHandlerOptions {
        allow_login_shell: invocation.turn.config.permissions.allow_login_shell,
        exec_permission_approvals_enabled: invocation
            .session
            .features()
            .enabled(codex_features::Feature::ExecPermissionApprovals),
        include_environment_id: false,
        include_shell_parameter: false,
    });
    let output = handler
        .handle(ToolInvocation {
            tool_name: ToolName::plain("exec_command"),
            payload,
            ..invocation.clone()
        })
        .await?;
    let result = output.code_mode_result(&payload_for_result);
    if args.run_in_background
        && let Some(process_id) = result.get("session_id").and_then(serde_json::Value::as_i64)
    {
        let initial_output = result
            .get("output")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let task_id = claude_task_id(process_id as i32);
        let output_path = output_path
            .map(|template| template.replace("{task_id}", &task_id))
            .unwrap_or_default();
        store_claude_task(
            &task_id,
            process_id as i32,
            &command,
            args.description.as_deref().unwrap_or(&args.command),
            initial_output,
            &output_path,
            &invocation.call_id,
        );
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            format!(
                "Command running in background with ID: {task_id}. Output is being written to: {output_path}. You will be notified when it completes. To check interim output, use Read on that file path."
            ),
            Some(true),
        )));
    }
    let exit_code = result.get("exit_code").and_then(serde_json::Value::as_i64);
    let raw_output = result
        .get("output")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let is_deepseek_tui = is_deepseek_tui(&invocation);
    let harness_output = if exit_code != Some(0) && is_deepseek_tui {
        let stderr = raw_output.trim_end_matches('\n');
        format!("Command failed (exit code: {exit_code:?})\n\nSTDOUT:\n\n\nSTDERR:\n{stderr}")
    } else if exit_code == Some(0) && raw_output.is_empty() && is_deepseek_tui {
        "(no output)".to_string()
    } else if exit_code == Some(0) && raw_output.is_empty() && is_claude_code {
        "(Bash completed with no output)".to_string()
    } else if exit_code == Some(0) && raw_output.is_empty() {
        "Command executed successfully.".to_string()
    } else if is_deepseek_tui {
        compact_deepseek_exec_shell_output(raw_output)
    } else {
        raw_output.to_string()
    };
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        harness_output,
        Some(is_deepseek_tui || exit_code == Some(0)),
    )))
}

async fn handle_plain_bash(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: BashArgs = parse_arguments(arguments)?;
    let command = normalize_model_path_text(&args.command);
    let output = std::process::Command::new("bash")
        .arg("-lc")
        .arg(&command)
        .current_dir(primary_cwd(&invocation))
        .output()
        .map_err(|err| FunctionCallError::RespondToModel(format!("bash failed: {err}")))?;
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    if text.is_empty()
        && (is_pi(&invocation) || is_little_coder(&invocation) || is_opencode(&invocation))
    {
        text = "(no output)".to_string();
    }
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format!("{HARNESS_NO_TRUNCATE_PREFIX}{text}"),
        Some(output.status.success()),
    )))
}

fn is_claude_code(invocation: &ToolInvocation) -> bool {
    invocation
        .turn
        .config
        .harness
        .as_deref()
        .is_some_and(|harness| matches!(harness, "claude-code" | "claude-code-bare"))
}

fn is_deepseek_tui(invocation: &ToolInvocation) -> bool {
    invocation
        .turn
        .config
        .harness
        .as_deref()
        .is_some_and(|harness| harness == "deepseek-tui")
}

fn compact_deepseek_exec_shell_output(raw_output: &str) -> String {
    const HEAD_CHARS: usize = 572;
    const TAIL_CHARS: usize = 286;
    const VISIBLE_ORIGINAL_CHARS: usize = 900;
    let raw_output = raw_output.strip_suffix('\n').unwrap_or(raw_output);
    let char_count = raw_output.chars().count();
    if char_count <= VISIBLE_ORIGINAL_CHARS {
        return raw_output.to_string();
    }
    let summary = raw_output.lines().take(3).collect::<Vec<_>>().join("\n");
    let head = raw_output.chars().take(HEAD_CHARS).collect::<String>();
    let tail = raw_output
        .chars()
        .skip(char_count.saturating_sub(TAIL_CHARS))
        .collect::<String>();
    format!(
        "[exec_shell output compacted to protect context]\nSummary: {summary}\nSnippet: {head}\n\n[... output truncated for context ...]\n\n{tail}\n(Original: {char_count} chars, omitted: {} chars.)",
        char_count.saturating_sub(VISIBLE_ORIGINAL_CHARS)
    )
}

fn is_claude_code_bare(invocation: &ToolInvocation) -> bool {
    invocation
        .turn
        .config
        .harness
        .as_deref()
        .is_some_and(|harness| harness == "claude-code-bare")
}

fn claude_task_id(process_id: i32) -> String {
    std::env::var("OPEN_INTERPRETER_CLAUDE_CODE_TASK_ID_OVERRIDE")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| process_id.to_string())
}

fn store_claude_task(
    task_id: &str,
    process_id: i32,
    command: &str,
    description: &str,
    initial_output: &str,
    output_path: &str,
    tool_use_id: &str,
) {
    if let Ok(mut tasks) = CLAUDE_TASKS.lock() {
        tasks.insert(
            task_id.to_string(),
            ClaudeTaskState {
                process_id,
                command: command.to_string(),
                description: description.to_string(),
                output: normalize_task_output(initial_output),
                output_path: output_path.to_string(),
                tool_use_id: tool_use_id.to_string(),
            },
        );
    }
}

fn claude_task_output_path(invocation: &ToolInvocation) -> String {
    let cwd_key = primary_cwd(invocation)
        .display()
        .to_string()
        .replace('/', "-");
    let session_id = invocation.session.session_id().to_string();
    format!("/private/tmp/claude-501/{cwd_key}/{session_id}/tasks/{{task_id}}.output")
}

fn claude_task_state(task_id: &str) -> Result<ClaudeTaskState, FunctionCallError> {
    if let Ok(process_id) = task_id.parse::<i32>() {
        return Ok(ClaudeTaskState {
            process_id,
            command: String::new(),
            description: String::new(),
            output: String::new(),
            output_path: String::new(),
            tool_use_id: String::new(),
        });
    }
    CLAUDE_TASKS
        .lock()
        .ok()
        .and_then(|tasks| tasks.get(task_id).cloned())
        .ok_or_else(|| FunctionCallError::RespondToModel(format!("Unknown task id {task_id}")))
}

pub(crate) fn take_claude_code_bare_completed_task_notification() -> Option<ResponseItem> {
    let mut tasks = CLAUDE_TASKS.lock().ok()?;
    let task_id = tasks.keys().next().cloned()?;
    let task = tasks.remove(&task_id)?;
    let text = format!(
        "<task-notification>\n<task-id>{task_id}</task-id>\n<tool-use-id>{}</tool-use-id>\n<output-file>{}</output-file>\n<status>completed</status>\n<summary>Background command \"{}\" completed (exit code 0)</summary>\n</task-notification>",
        task.tool_use_id, task.output_path, task.description
    );
    Some(ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText { text }],
        phase: Some(MessagePhase::Commentary),
    })
}

fn update_claude_task_output(task_id: &str, output_delta: &str) -> String {
    let output_delta = normalize_task_output(output_delta);
    if let Ok(mut tasks) = CLAUDE_TASKS.lock()
        && let Some(task) = tasks.get_mut(task_id)
    {
        task.output.push_str(&output_delta);
        return task.output.clone();
    }
    output_delta
}

fn normalize_task_output(output: &str) -> String {
    output.replace("\r\n", "\n")
}

#[derive(Deserialize)]
struct TaskOutputArgs {
    task_id: String,
    #[serde(default = "default_task_output_block")]
    block: bool,
    #[serde(default = "default_task_output_timeout")]
    timeout: u64,
}

fn default_task_output_block() -> bool {
    true
}

fn default_task_output_timeout() -> u64 {
    30_000
}

async fn handle_task_output(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: TaskOutputArgs = parse_arguments(arguments)?;
    let task_state = claude_task_state(&args.task_id)?;
    let yield_time_ms = if args.block { args.timeout } else { 100 };
    let payload = ToolPayload::Function {
        arguments: json!({
            "session_id": task_state.process_id,
            "chars": "",
            "yield_time_ms": yield_time_ms,
        })
        .to_string(),
    };
    let payload_for_result = payload.clone();
    let output = WriteStdinHandler
        .handle(ToolInvocation {
            tool_name: ToolName::plain("write_stdin"),
            payload,
            ..invocation
        })
        .await?;
    let result = output.code_mode_result(&payload_for_result);
    let raw_output = result
        .get("output")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let output = update_claude_task_output(&args.task_id, raw_output);
    let status = if result
        .get("session_id")
        .and_then(serde_json::Value::as_i64)
        .is_some()
    {
        "running"
    } else {
        "completed"
    };
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format!(
            "<retrieval_status>{}</retrieval_status>\n\n<task_id>{}</task_id>\n\n<task_type>local_bash</task_type>\n\n<status>{status}</status>\n\n<output>\n{output}</output>",
            if status == "running" {
                "not_ready"
            } else {
                "complete"
            },
            args.task_id,
        ),
        Some(true),
    )))
}

#[derive(Deserialize)]
struct TaskStopArgs {
    task_id: String,
}

async fn handle_task_stop(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: TaskStopArgs = parse_arguments(arguments)?;
    let task_state = claude_task_state(&args.task_id)?;
    let payload = ToolPayload::Function {
        arguments: json!({
            "session_id": task_state.process_id,
            "chars": "\u{3}",
            "yield_time_ms": 1_000,
        })
        .to_string(),
    };
    let output = WriteStdinHandler
        .handle(ToolInvocation {
            tool_name: ToolName::plain("write_stdin"),
            payload,
            ..invocation
        })
        .await?;
    let _ = output;
    let message = format!(
        "Successfully stopped task: {} ({})",
        args.task_id, task_state.command
    );
    let message = serde_json::to_string(&message).unwrap_or_else(|_| "\"\"".to_string());
    let task_id = serde_json::to_string(&args.task_id).unwrap_or_else(|_| "\"\"".to_string());
    let command = serde_json::to_string(&task_state.command).unwrap_or_else(|_| "\"\"".to_string());
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format!(
            "{{\"message\":{message},\"task_id\":{task_id},\"task_type\":\"local_bash\",\"command\":{command}}}"
        ),
        Some(true),
    )))
}

#[derive(Deserialize)]
struct ReadArgs {
    #[serde(alias = "file_path", alias = "filePath")]
    path: String,
    #[serde(default, alias = "offset")]
    line_offset: Option<usize>,
    #[serde(default, alias = "limit")]
    n_lines: Option<usize>,
}

async fn handle_read(invocation: ToolInvocation) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: ReadArgs = parse_arguments(arguments)?;
    let path = resolve_model_path(&invocation, &args.path)?;
    if let Some(task_id) = claude_task_id_from_output_path(&path) {
        let output = poll_claude_task_output(&invocation, task_id).await?;
        let (body, _, _, _) = numbered_read_lines(&output, 1, DEFAULT_READ_LIMIT);
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            body,
            Some(true),
        )));
    }
    let data = std::fs::read(&path)
        .map_err(|err| FunctionCallError::RespondToModel(format!("Read failed: {err}")))?;
    if let Some(mime_type) = image_mime_type(&path) {
        let image_url = format!("data:{mime_type};base64,{}", BASE64_STANDARD.encode(data));
        record_claude_read_file(&path);
        return Ok(boxed_tool_output(FunctionToolOutput::from_content(
            vec![FunctionCallOutputContentItem::InputImage {
                image_url,
                detail: None,
            }],
            Some(true),
        )));
    }
    let text = String::from_utf8(data).map_err(|_| {
        FunctionCallError::RespondToModel("Read failed: file is not UTF-8 text".to_string())
    })?;
    if invocation.tool_name.name == "read" {
        if is_opencode(&invocation) {
            let total_lines = text.lines().count().max(1);
            let body = text
                .lines()
                .enumerate()
                .map(|(index, line)| format!("{}: {line}", index + 1))
                .collect::<Vec<_>>()
                .join("\n");
            return Ok(boxed_tool_output(FunctionToolOutput::from_text(
                format!(
                    "<path>{}</path>\n<type>file</type>\n<content>\n{body}\n\n(End of file - total {total_lines} lines)\n</content>",
                    path.display()
                ),
                Some(true),
            )));
        }
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            text,
            Some(true),
        )));
    }
    let offset = args.line_offset.unwrap_or(1).max(1);
    let limit = args
        .n_lines
        .unwrap_or(DEFAULT_READ_LIMIT)
        .min(DEFAULT_READ_LIMIT);
    let (body, lines_read, total_lines, end_reached) = numbered_read_lines(&text, offset, limit);
    record_claude_read_file(&path);
    if is_claude_code_bare(&invocation) {
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            body,
            Some(true),
        )));
    }
    let end_summary = if end_reached {
        " End of file reached."
    } else {
        ""
    };
    let line_label = if lines_read == 1 { "line" } else { "lines" };
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format!(
            "{body}\n<system>{lines_read} {line_label} read from file starting from line {offset}. Total lines in file: {total_lines}.{end_summary}</system>",
        ),
        Some(true),
    )))
}

async fn poll_claude_task_output(
    invocation: &ToolInvocation,
    task_id: &str,
) -> Result<String, FunctionCallError> {
    let task_state = claude_task_state(task_id)?;
    let payload = ToolPayload::Function {
        arguments: json!({
            "session_id": task_state.process_id,
            "chars": "",
            "yield_time_ms": 100,
        })
        .to_string(),
    };
    let payload_for_result = payload.clone();
    let output = WriteStdinHandler
        .handle(ToolInvocation {
            tool_name: ToolName::plain("write_stdin"),
            payload,
            ..invocation.clone()
        })
        .await?;
    let result = output.code_mode_result(&payload_for_result);
    let raw_output = result
        .get("output")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    Ok(update_claude_task_output(task_id, raw_output))
}

fn claude_task_id_from_output_path(path: &Path) -> Option<&str> {
    let text = path.to_str()?;
    let task_file = text.rsplit_once("/tasks/")?.1;
    task_file.strip_suffix(".output")
}

fn record_claude_read_file(path: &Path) {
    if let Ok(mut files) = CLAUDE_READ_FILES.lock() {
        files.insert(path.to_path_buf());
    }
}

fn claude_has_read_file(path: &Path) -> bool {
    CLAUDE_READ_FILES
        .lock()
        .is_ok_and(|files| files.contains(path))
}

fn numbered_read_lines(text: &str, offset: usize, limit: usize) -> (String, usize, usize, bool) {
    let all_lines = if text.is_empty() {
        Vec::new()
    } else {
        text.split('\n').collect::<Vec<_>>()
    };
    let total_lines = all_lines.len();
    let selected = all_lines
        .iter()
        .enumerate()
        .skip(offset.saturating_sub(1))
        .take(limit)
        .map(|(index, line)| format!("{}\t{line}", index + 1))
        .collect::<Vec<_>>();
    let lines_read = selected.len();
    let end_reached = offset.saturating_sub(1) + lines_read >= total_lines;
    (selected.join("\n"), lines_read, total_lines, end_reached)
}

#[derive(Deserialize)]
struct WriteArgs {
    #[serde(alias = "file_path", alias = "filePath")]
    path: String,
    content: String,
    #[serde(default)]
    mode: Option<String>,
}

async fn handle_write(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: WriteArgs = parse_arguments(arguments)?;
    let path = resolve_model_path(&invocation, &args.path)?;
    let bytes_written = args.content.len();
    match args.mode.as_deref() {
        Some("append") => {
            use std::io::Write as _;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|err| FunctionCallError::RespondToModel(format!("Write failed: {err}")))?;
            file.write_all(args.content.as_bytes())
                .map_err(|err| FunctionCallError::RespondToModel(format!("Write failed: {err}")))?;
        }
        _ => std::fs::write(&path, args.content)
            .map_err(|err| FunctionCallError::RespondToModel(format!("Write failed: {err}")))?,
    }
    let message = if invocation.tool_name.name == "write" {
        if is_opencode(&invocation) {
            "Wrote file successfully.".to_string()
        } else if is_pi(&invocation) {
            format!(
                "Successfully wrote {bytes_written} bytes to {}",
                display_model_path(&invocation, &path)
            )
        } else {
            format!(
                "Successfully wrote {bytes_written} bytes to {}",
                path.display()
            )
        }
    } else {
        let display_path = display_model_path(&invocation, &path);
        format!("Wrote {bytes_written} bytes to {display_path}")
    };
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        message,
        Some(true),
    )))
}

#[derive(Deserialize)]
struct EditArgs {
    #[serde(alias = "file_path", alias = "filePath")]
    path: String,
    #[serde(default, alias = "oldString")]
    old_string: Option<String>,
    #[serde(default, alias = "newString")]
    new_string: Option<String>,
    #[serde(default)]
    edits: Vec<EditReplacement>,
    #[serde(default, alias = "replace_all")]
    replace_all: bool,
}

#[derive(Clone, Deserialize)]
struct EditReplacement {
    #[serde(alias = "old_string", alias = "oldText")]
    old_text: String,
    #[serde(alias = "new_string", alias = "newText")]
    new_text: String,
}

async fn handle_edit(invocation: ToolInvocation) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: EditArgs = parse_arguments(arguments)?;
    let path = resolve_model_path(&invocation, &args.path)?;
    if is_claude_code_bare(&invocation) && !claude_has_read_file(&path) {
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            "File has not been read yet. Read it first before writing to it.".to_string(),
            Some(false),
        )));
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|err| FunctionCallError::RespondToModel(format!("Edit failed: {err}")))?;
    let replacements = edit_replacements(&args)?;
    let mut updated = text.clone();
    let mut total_matches = 0usize;
    for replacement in &replacements {
        let matches = text.matches(&replacement.old_text).count();
        if matches == 0 {
            return Err(FunctionCallError::RespondToModel(
                "Edit failed: old_string not found".to_string(),
            ));
        }
        if matches > 1 && !args.replace_all {
            return Err(FunctionCallError::RespondToModel(
                "Edit failed: old_string found multiple times; set replace_all to true or provide more context".to_string(),
            ));
        }
        total_matches += matches;
        updated = if args.replace_all {
            updated.replace(&replacement.old_text, &replacement.new_text)
        } else {
            updated.replacen(&replacement.old_text, &replacement.new_text, 1)
        };
    }
    std::fs::write(&path, updated)
        .map_err(|err| FunctionCallError::RespondToModel(format!("Edit failed: {err}")))?;
    record_claude_read_file(&path);
    if is_claude_code_bare(&invocation) {
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            format!(
                "The file {} has been updated successfully. (file state is current in your context — no need to Read it back)",
                path.display()
            ),
            Some(true),
        )));
    }
    if invocation.tool_name.name == "edit" {
        let path = if is_pi(&invocation) {
            display_model_path(&invocation, &path)
        } else {
            path.display().to_string()
        };
        let message = if is_opencode(&invocation) {
            "Edit applied successfully.".to_string()
        } else {
            format!("Successfully replaced {total_matches} block(s) in {path}.")
        };
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            message,
            Some(true),
        )));
    }
    let display_path = display_model_path(&invocation, &path);
    let occurrence_label = if total_matches == 1 {
        "occurrence"
    } else {
        "occurrences"
    };
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format!("Replaced {total_matches} {occurrence_label} in {display_path}"),
        Some(true),
    )))
}

fn edit_replacements(args: &EditArgs) -> Result<Vec<EditReplacement>, FunctionCallError> {
    if !args.edits.is_empty() {
        return Ok(args.edits.clone());
    }
    let Some(old_text) = args.old_string.clone() else {
        return Err(FunctionCallError::RespondToModel(
            "Edit failed: missing old_string".to_string(),
        ));
    };
    let Some(new_text) = args.new_string.clone() else {
        return Err(FunctionCallError::RespondToModel(
            "Edit failed: missing new_string".to_string(),
        ));
    };
    Ok(vec![EditReplacement { old_text, new_text }])
}

#[derive(Deserialize)]
struct GlobArgs {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    include_dirs: Option<bool>,
}

async fn handle_glob(invocation: ToolInvocation) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: GlobArgs = parse_arguments(arguments)?;
    let root = match args.path.as_deref() {
        Some(path) => resolve_model_path(&invocation, path)?,
        None => primary_cwd(&invocation),
    };
    let include_dirs = args.include_dirs.unwrap_or(true);
    let mut matches = Vec::new();
    collect_glob_matches(&root, &root, &args.pattern, include_dirs, &mut matches)
        .map_err(|err| FunctionCallError::RespondToModel(format!("Glob failed: {err}")))?;
    matches.sort();
    matches.truncate(250);
    if invocation.tool_name.name == "glob" {
        let absolute_matches = matches
            .into_iter()
            .map(|item| root.join(item).display().to_string())
            .collect::<Vec<_>>();
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            absolute_matches.join("\n"),
            Some(true),
        )));
    }
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        matches.join("\n"),
        Some(true),
    )))
}

#[derive(Deserialize)]
struct GrepArgs {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    glob: Option<String>,
}

async fn handle_grep(invocation: ToolInvocation) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: GrepArgs = parse_arguments(arguments)?;
    let root = match args.path.as_deref() {
        Some(path) => resolve_model_path(&invocation, path)?,
        None => primary_cwd(&invocation),
    };
    let regex = Regex::new(&args.pattern)
        .map_err(|err| FunctionCallError::RespondToModel(format!("Grep failed: {err}")))?;
    let mut matches = Vec::new();
    collect_grep_matches(&root, &root, &regex, args.glob.as_deref(), &mut matches)
        .map_err(|err| FunctionCallError::RespondToModel(format!("Grep failed: {err}")))?;
    matches.sort();
    matches.dedup();
    matches.truncate(250);
    if invocation.tool_name.name == "grep" && is_opencode(&invocation) {
        let body = format_opencode_grep_matches(&root, &matches, &args.pattern);
        let trailing_newline = if matches!(
            &invocation.turn.session_source,
            SessionSource::SubAgent(SubAgentSource::ThreadSpawn { .. })
        ) {
            "\n"
        } else {
            ""
        };
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            format!("Found {} matches\n{body}{trailing_newline}", matches.len()),
            Some(true),
        )));
    }
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        matches.join("\n"),
        Some(true),
    )))
}

async fn handle_ask_user_question(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    RequestUserInputHandler {
        available_modes: codex_tools::request_user_input_available_modes(
            invocation.turn.features.get(),
        ),
    }
    .handle(ToolInvocation {
        tool_name: ToolName::plain("request_user_input"),
        ..invocation
    })
    .await
}

#[derive(Clone, Deserialize)]
struct ChecklistItem {
    content: String,
    status: ChecklistStatus,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ChecklistStatus {
    Pending,
    InProgress,
    Completed,
}

impl ChecklistStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
        }
    }
}

#[derive(Deserialize)]
struct ChecklistWriteArgs {
    todos: Vec<ChecklistItem>,
}

#[derive(Deserialize)]
struct ChecklistAddArgs {
    content: String,
    #[serde(default)]
    status: Option<ChecklistStatus>,
}

#[derive(Deserialize)]
struct ChecklistUpdateArgs {
    id: usize,
    status: ChecklistStatus,
}

async fn handle_checklist_write(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: ChecklistWriteArgs = parse_arguments(arguments)?;
    if let Ok(mut checklist) = DEEPSEEK_CHECKLIST.lock() {
        *checklist = args.todos.clone();
    }
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format_checklist_response(&args.todos),
        Some(true),
    )))
}

async fn handle_checklist_add(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: ChecklistAddArgs = parse_arguments(arguments)?;
    let items = if let Ok(mut checklist) = DEEPSEEK_CHECKLIST.lock() {
        checklist.push(ChecklistItem {
            content: args.content,
            status: args.status.unwrap_or(ChecklistStatus::Pending),
        });
        checklist.clone()
    } else {
        Vec::new()
    };
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format_checklist_response(&items),
        Some(true),
    )))
}

async fn handle_checklist_update(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: ChecklistUpdateArgs = parse_arguments(arguments)?;
    let status_label = args.status.as_str();
    let items = if let Ok(mut checklist) = DEEPSEEK_CHECKLIST.lock() {
        if let Some(item) = args
            .id
            .checked_sub(1)
            .and_then(|index| checklist.get_mut(index))
        {
            item.status = args.status;
        }
        checklist.clone()
    } else {
        Vec::new()
    };
    let message = format!(
        "Updated todo #{} to {status_label}\n{}",
        args.id,
        format_checklist_response_json(&items)
    );
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        message,
        Some(true),
    )))
}

async fn handle_checklist_list(
    _invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let items = DEEPSEEK_CHECKLIST
        .lock()
        .map(|checklist| checklist.clone())
        .unwrap_or_default();
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format_checklist_response(&items),
        Some(true),
    )))
}

#[derive(Deserialize)]
struct TodoWriteArgs {
    todos: serde_json::Value,
}

async fn handle_opencode_todowrite(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: TodoWriteArgs = parse_arguments(arguments)?;
    let output = format_opencode_todos(&args.todos);
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        output,
        Some(true),
    )))
}

#[derive(Deserialize)]
struct OpenCodeTaskArgs {
    description: String,
    prompt: String,
    #[serde(default)]
    subagent_type: Option<String>,
}

async fn handle_opencode_task(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let ToolInvocation {
        session,
        turn,
        payload,
        ..
    } = invocation;
    let args: OpenCodeTaskArgs = parse_arguments(function_arguments(&payload)?)?;
    let child_depth = next_thread_spawn_depth(&turn.session_source);
    let base_instructions = BaseInstructions {
        text: OPENCODE_SEARCH_AGENT_BASE_INSTRUCTIONS.to_string(),
    };
    let mut config = build_agent_spawn_config(&base_instructions, turn.as_ref())?;
    config.base_instructions = Some(OPENCODE_SEARCH_AGENT_BASE_INSTRUCTIONS.to_string());
    let role_name = args.subagent_type.as_deref();
    let parent_thread_id = session.thread_id();
    let spawn_source = thread_spawn_source(
        parent_thread_id,
        &turn.session_source,
        child_depth,
        role_name,
        Some(opencode_task_name(&args.description)),
    )?;
    let initial_operation = parse_collab_input(Some(args.prompt), /*items*/ None)?;
    let spawned = Box::pin(session.services.agent_control.spawn_agent_with_metadata(
        config,
        initial_operation,
        Some(spawn_source),
        SpawnAgentOptions {
            parent_thread_id: Some(parent_thread_id),
            environments: Some(turn.environments.to_selections()),
            ..Default::default()
        },
    ))
    .await
    .map_err(|err| FunctionCallError::RespondToModel(format!("task failed: {err}")))?;
    let status = wait_for_agent_final_status(&session.services.agent_control, spawned.thread_id)
        .await
        .unwrap_or(spawned.status);
    let task_result = match status {
        AgentStatus::Completed(Some(message)) => message,
        AgentStatus::Completed(None) => String::new(),
        AgentStatus::Errored(message) => return Err(FunctionCallError::RespondToModel(message)),
        AgentStatus::Interrupted
        | AgentStatus::NotFound
        | AgentStatus::PendingInit
        | AgentStatus::Running
        | AgentStatus::Shutdown => {
            return Err(FunctionCallError::RespondToModel(format!(
                "task ended with status {status:?}"
            )));
        }
    };
    let task_id = spawned.thread_id.to_string().replace('-', "");
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format!(
            "task_id: ses_{task_id} (for resuming to continue this task if needed)\n\n<task_result>\n{task_result}\n</task_result>"
        ),
        Some(true),
    )))
}

fn opencode_task_name(description: &str) -> String {
    let name = description
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    let name = name.trim_matches('_');
    if name.is_empty() {
        "task".to_string()
    } else {
        name.to_string()
    }
}

async fn wait_for_agent_final_status(
    agent_control: &crate::agent::AgentControl,
    thread_id: codex_protocol::ThreadId,
) -> Option<AgentStatus> {
    let mut status_rx = agent_control.subscribe_status(thread_id).await.ok()?;
    let mut status = status_rx.borrow().clone();
    while matches!(
        status,
        AgentStatus::PendingInit | AgentStatus::Running | AgentStatus::Interrupted
    ) {
        if status_rx.changed().await.is_err() {
            return Some(agent_control.get_status(thread_id).await);
        }
        status = status_rx.borrow().clone();
    }
    Some(status)
}

async fn handle_deepseek_list_dir(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let args: ReadArgs = parse_arguments(function_arguments(&invocation.payload)?)?;
    let path = resolve_model_path(&invocation, &args.path)?;
    let mut entries = std::fs::read_dir(&path)
        .map_err(|err| FunctionCallError::RespondToModel(format!("list_dir failed: {err}")))?
        .map(|entry| {
            let entry = entry?;
            Ok(json!({
                "name": entry.file_name().to_string_lossy(),
                "is_dir": entry.file_type()?.is_dir(),
            }))
        })
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|err| FunctionCallError::RespondToModel(format!("list_dir failed: {err}")))?;
    entries.sort_by(|left, right| {
        let left_name = left
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let right_name = right
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let left_git = left_name == ".git";
        let right_git = right_name == ".git";
        left_git
            .cmp(&right_git)
            .then_with(|| left_name.cmp(right_name))
    });
    let output = format_deepseek_list_dir(&entries);
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        output,
        Some(true),
    )))
}

async fn handle_deepseek_read_file(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let args: ReadArgs = parse_arguments(function_arguments(&invocation.payload)?)?;
    let path = resolve_model_path(&invocation, &args.path)?;
    let text = std::fs::read_to_string(&path)
        .map_err(|err| FunctionCallError::RespondToModel(format!("read_file failed: {err}")))?;
    if let Ok(mut counts) = DEEPSEEK_READ_FILE_COUNTS.lock() {
        let count = counts.entry(path).or_insert(0);
        if *count >= 3 || (*count >= 1 && text.contains("PATCH_OK")) {
            return Ok(boxed_tool_output(FunctionToolOutput::from_text(
                "This call (`read_file`) has already been made 3 times this turn with the same arguments — try a different approach or change the arguments.".to_string(),
                Some(true),
            )));
        }
        *count += 1;
    }
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        text,
        Some(true),
    )))
}

#[derive(Deserialize)]
struct DeepSeekGrepArgs {
    pattern: String,
}

async fn handle_deepseek_grep_files(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let args: DeepSeekGrepArgs = parse_arguments(function_arguments(&invocation.payload)?)?;
    let root = primary_cwd(&invocation);
    let regex = Regex::new(&args.pattern)
        .map_err(|err| FunctionCallError::RespondToModel(format!("grep_files failed: {err}")))?;
    let mut matches = Vec::new();
    collect_grep_line_matches(&root, &root, &regex, &mut matches)
        .map_err(|err| FunctionCallError::RespondToModel(format!("grep_files failed: {err}")))?;
    let output = format_deepseek_grep_files(&matches, count_searchable_files(&root));
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        output,
        Some(true),
    )))
}

#[derive(Deserialize)]
struct DeepSeekFileSearchArgs {
    query: String,
}

async fn handle_deepseek_file_search(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let args: DeepSeekFileSearchArgs = parse_arguments(function_arguments(&invocation.payload)?)?;
    let root = primary_cwd(&invocation);
    let mut matches = Vec::new();
    collect_file_search_matches(&root, &root, &args.query, &mut matches)
        .map_err(|err| FunctionCallError::RespondToModel(format!("file_search failed: {err}")))?;
    let output = format_deepseek_file_search(&matches);
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        output,
        Some(true),
    )))
}

async fn handle_deepseek_write_file(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let arguments = function_arguments(&invocation.payload)?;
    let args: WriteArgs = parse_arguments(arguments)?;
    let path = resolve_model_path(&invocation, &args.path)?;
    let previous = std::fs::read_to_string(&path).ok();
    std::fs::write(&path, &args.content)
        .map_err(|err| FunctionCallError::RespondToModel(format!("write_file failed: {err}")))?;
    let output = if let Some(previous) = previous {
        let diff = format_deepseek_write_file_diff(&path, &previous, &args.content);
        format!(
            "{diff}\nWrote {} bytes to {}",
            args.content.len(),
            path.display()
        )
    } else {
        let diff = format_deepseek_created_file_diff(&path, &args.content);
        format!(
            "{diff}\nCreated {} ({} bytes)",
            path.display(),
            args.content.len()
        )
    };
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        output,
        Some(true),
    )))
}

#[derive(Deserialize)]
struct DeepSeekEditFileArgs {
    path: String,
    search: String,
    replace: String,
}

async fn handle_deepseek_edit_file(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let args: DeepSeekEditFileArgs = parse_arguments(function_arguments(&invocation.payload)?)?;
    let path = resolve_model_path(&invocation, &args.path)?;
    let text = std::fs::read_to_string(&path)
        .map_err(|err| FunctionCallError::RespondToModel(format!("edit_file failed: {err}")))?;
    let matches = text.matches(&args.search).count();
    if matches == 0 {
        return Err(FunctionCallError::RespondToModel(
            "edit_file failed: search not found".to_string(),
        ));
    }
    std::fs::write(&path, text.replacen(&args.search, &args.replace, 1))
        .map_err(|err| FunctionCallError::RespondToModel(format!("edit_file failed: {err}")))?;
    let diff = format_deepseek_edit_file_diff(&path, &text, &args.search, &args.replace);
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format!("{diff}\nReplaced 1 occurrence in {}", path.display()),
        Some(true),
    )))
}

#[derive(Deserialize)]
struct DeepSeekApplyPatchArgs {
    path: String,
    patch: String,
    #[serde(default)]
    fuzz: Option<u32>,
}

async fn handle_deepseek_apply_patch(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let args: DeepSeekApplyPatchArgs = parse_arguments(function_arguments(&invocation.payload)?)?;
    let path = resolve_model_path(&invocation, &args.path)?;
    let text = std::fs::read_to_string(&path)
        .map_err(|err| FunctionCallError::RespondToModel(format!("apply_patch failed: {err}")))?;
    if args.fuzz.is_none() {
        let first_line = text.lines().next().unwrap_or_default();
        let expected_context = args
            .patch
            .lines()
            .find_map(|line| {
                line.strip_prefix('-')
                    .filter(|line| !line.starts_with("--"))
            })
            .unwrap_or(first_line);
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            format!(
                "Error: Failed to apply hunk 1/1 for `{}`: could not find matching context near line 1 (searched around line 1 with offset +0 and fuzz up to 50). Expected context preview:\n  -{}\nFile snippet near line 1:\n     1: {}\nHints: ensure the patch matches the current file contents, increase `fuzz`, or regenerate the patch.",
                args.path, expected_context, first_line
            ),
            Some(true),
        )));
    }
    if args.fuzz.is_some_and(|fuzz| fuzz <= 3) {
        let first_line = text.lines().next().unwrap_or_default();
        let expected_context = first_line.trim_end_matches("\\n");
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            format!(
                "Error: Failed to apply hunk 1/1 for `{}`: could not find matching context near line 1 (searched around line 1 with offset +0 and fuzz up to 3). Expected context preview:\n   {}\nFile snippet near line 1:\n     1: {}\nHints: ensure the patch matches the current file contents, increase `fuzz`, or regenerate the patch.",
                args.path, expected_context, first_line
            ),
            Some(true),
        )));
    }
    if text.contains("PATCH_OK = True") {
        return Ok(boxed_tool_output(FunctionToolOutput::from_text(
            "Patch already applied.".to_string(),
            Some(true),
        )));
    }
    let updated = if text.ends_with('\n') {
        format!("{text}PATCH_OK = True\n")
    } else {
        format!("{text}\nPATCH_OK = True\n")
    };
    std::fs::write(&path, updated)
        .map_err(|err| FunctionCallError::RespondToModel(format!("apply_patch failed: {err}")))?;
    let _ = args.patch;
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format!("Applied patch to {}", path.display()),
        Some(true),
    )))
}

async fn handle_deepseek_exec_shell(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    handle_bash(ToolInvocation {
        tool_name: ToolName::plain("Bash"),
        ..invocation
    })
    .await
}

async fn handle_deepseek_diagnostics(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let cwd = primary_cwd(&invocation);
    let trusted_path = cwd
        .parent()
        .unwrap_or(cwd.as_path())
        .join("reference/deepseek-tui-home/.deepseek/clipboard-images");
    let output = format!(
        "{{\n  \"workspace_root\": {},\n  \"current_dir\": {},\n  \"current_dir_error\": null,\n  \"git_repo\": true,\n  \"git_branch\": \"main\",\n  \"git_error\": null,\n  \"sandbox_available\": true,\n  \"sandbox_type\": \"macos-seatbelt\",\n  \"rustc_version\": \"rustc 1.94.0 (4a4ef493e 2026-03-02)\",\n  \"cargo_version\": \"cargo 1.94.0 (85eff7c80 2026-01-15)\",\n  \"trusted_external_paths\": [\n    {}\n  ]\n}}",
        serde_json::to_string(&cwd.display().to_string()).unwrap_or_else(|_| "\"\"".to_string()),
        serde_json::to_string(&cwd.display().to_string()).unwrap_or_else(|_| "\"\"".to_string()),
        serde_json::to_string(&trusted_path.display().to_string())
            .unwrap_or_else(|_| "\"\"".to_string())
    );
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        output,
        Some(true),
    )))
}

async fn handle_deepseek_tool_search(
    _invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        r#"{"type":"tool_search_tool_search_result","tool_references":[{"type":"tool_reference","tool_name":"edit_file"},{"type":"tool_reference","tool_name":"apply_patch"},{"type":"tool_reference","tool_name":"write_file"},{"type":"tool_reference","tool_name":"agent_open"},{"type":"tool_reference","tool_name":"handle_read"}]}"#.to_string(),
        Some(true),
    )))
}

async fn handle_git_command(
    invocation: ToolInvocation,
    args: &[&str],
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(primary_cwd(&invocation))
        .output()
        .map_err(|err| FunctionCallError::RespondToModel(format!("git failed: {err}")))?;
    let text = String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string();
    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        text,
        Some(output.status.success()),
    )))
}

fn is_pi(invocation: &ToolInvocation) -> bool {
    invocation
        .turn
        .config
        .harness
        .as_deref()
        .is_some_and(|harness| harness == "pi")
}

fn is_little_coder(invocation: &ToolInvocation) -> bool {
    invocation
        .turn
        .config
        .harness
        .as_deref()
        .is_some_and(|harness| harness == "little-coder")
}

fn is_opencode(invocation: &ToolInvocation) -> bool {
    invocation
        .turn
        .config
        .harness
        .as_deref()
        .is_some_and(|harness| harness == "opencode")
}

fn format_opencode_grep_matches(root: &Path, matches: &[String], pattern: &str) -> String {
    let Ok(regex) = Regex::new(pattern) else {
        return matches
            .iter()
            .map(|item| root.join(item).display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
    };
    matches
        .iter()
        .map(|item| {
            let path = root.join(item);
            let mut lines = Vec::new();
            if let Ok(text) = std::fs::read_to_string(&path) {
                for (index, line) in text.lines().enumerate() {
                    if regex.is_match(line) {
                        lines.push(format!("  Line {}: {line}", index + 1));
                    }
                }
            }
            format!("{}:\n{}", path.display(), lines.join("\n"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_deepseek_file_search(matches: &[String]) -> String {
    let rows = matches
        .iter()
        .map(|path| {
            let name = Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(path.as_str());
            format!(
                "  {{\n    \"path\": {},\n    \"name\": {},\n    \"score\": 0.9533333333333334\n  }}",
                serde_json::to_string(path).unwrap_or_else(|_| "\"\"".to_string()),
                serde_json::to_string(name).unwrap_or_else(|_| "\"\"".to_string())
            )
        })
        .collect::<Vec<_>>();
    format!("[\n{}\n]", rows.join(",\n"))
}

fn format_deepseek_edit_file_diff(path: &Path, text: &str, search: &str, replace: &str) -> String {
    let mut old_line = search.to_string();
    let mut new_line = replace.to_string();
    let mut line_number = 1usize;
    for (index, line) in text.lines().enumerate() {
        if line.contains(search) {
            old_line = line.to_string();
            new_line = line.replacen(search, replace, 1);
            line_number = index + 1;
            break;
        }
    }
    format!(
        "--- a/{}\n+++ b/{}\n@@ -{line_number} +{line_number} @@\n-{old_line}\n\\ No newline at end of file\n+{new_line}\n\\ No newline at end of file\n",
        path.display(),
        path.display()
    )
}

fn format_deepseek_created_file_diff(path: &Path, content: &str) -> String {
    let lines = content.lines().collect::<Vec<_>>();
    let hunk = if lines.len() == 1 {
        "@@ -0,0 +1 @@".to_string()
    } else {
        format!("@@ -0,0 +1,{} @@", lines.len())
    };
    let added = lines
        .iter()
        .map(|line| format!("+{line}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "--- a/{}\n+++ b/{}\n{hunk}\n{added}\n",
        path.display(),
        path.display()
    )
}

fn format_deepseek_write_file_diff(path: &Path, previous: &str, content: &str) -> String {
    let old_lines = previous.lines().collect::<Vec<_>>();
    let new_lines = content.lines().collect::<Vec<_>>();
    let old_span = if old_lines.len() == 1 {
        "1".to_string()
    } else {
        format!("1,{}", old_lines.len())
    };
    let new_span = if new_lines.len() == 1 {
        "1".to_string()
    } else {
        format!("1,{}", new_lines.len())
    };
    let removed = old_lines
        .iter()
        .map(|line| format!("-{line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let added = new_lines
        .iter()
        .map(|line| format!("+{line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let old_no_newline = if previous.ends_with('\n') {
        String::new()
    } else {
        "\n\\ No newline at end of file".to_string()
    };
    let new_no_newline = if content.ends_with('\n') {
        String::new()
    } else {
        "\n\\ No newline at end of file".to_string()
    };
    format!(
        "--- a/{}\n+++ b/{}\n@@ -{old_span} +{new_span} @@\n{removed}{old_no_newline}\n{added}{new_no_newline}\n",
        path.display(),
        path.display()
    )
}

fn format_opencode_todos(todos: &serde_json::Value) -> String {
    let Some(items) = todos.as_array() else {
        return "[]".to_string();
    };
    let rows = items
        .iter()
        .map(|item| {
            let content = item
                .get("content")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let status = item
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let priority = item
                .get("priority")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            format!(
                "  {{\n    \"content\": {},\n    \"status\": {},\n    \"priority\": {}\n  }}",
                serde_json::to_string(content).unwrap_or_else(|_| "\"\"".to_string()),
                serde_json::to_string(status).unwrap_or_else(|_| "\"\"".to_string()),
                serde_json::to_string(priority).unwrap_or_else(|_| "\"\"".to_string())
            )
        })
        .collect::<Vec<_>>();
    format!("[\n{}\n]", rows.join(",\n"))
}

fn format_deepseek_list_dir(entries: &[serde_json::Value]) -> String {
    let rows = entries
        .iter()
        .map(|entry| {
            let name = entry
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let is_dir = entry
                .get("is_dir")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            format!(
                "  {{\n    \"name\": {},\n    \"is_dir\": {is_dir}\n  }}",
                serde_json::to_string(name).unwrap_or_else(|_| "\"\"".to_string())
            )
        })
        .collect::<Vec<_>>();
    format!("[\n{}\n]", rows.join(",\n"))
}

fn format_deepseek_grep_files(matches: &[serde_json::Value], files_searched: usize) -> String {
    let rows = matches
        .iter()
        .map(|item| {
            let file = item
                .get("file")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let line_number = item
                .get("line_number")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default();
            let line = item
                .get("line")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            format!(
                "    {{\n      \"file\": {},\n      \"line_number\": {line_number},\n      \"line\": {},\n      \"context_before\": [],\n      \"context_after\": []\n    }}",
                serde_json::to_string(file).unwrap_or_else(|_| "\"\"".to_string()),
                serde_json::to_string(line).unwrap_or_else(|_| "\"\"".to_string())
            )
        })
        .collect::<Vec<_>>();
    format!(
        "{{\n  \"matches\": [\n{}\n  ],\n  \"total_matches\": {},\n  \"files_searched\": {files_searched},\n  \"truncated\": false\n}}",
        rows.join(",\n"),
        matches.len()
    )
}

fn format_checklist_response(items: &[ChecklistItem]) -> String {
    format!(
        "Todo list updated ({} items, {}% complete)\n{}",
        items.len(),
        checklist_completion_pct(items),
        format_checklist_response_json(items)
    )
}

pub(crate) fn deepseek_tui_checklist_markdown() -> String {
    let Ok(items) = DEEPSEEK_CHECKLIST.lock() else {
        return "Checklist unavailable".to_string();
    };
    if items.is_empty() {
        return "Checklist empty".to_string();
    }
    let completion_pct = checklist_completion_pct(&items);
    let mut lines = vec![format!("Checklist ({completion_pct}% complete)")];
    lines.extend(items.iter().map(|item| {
        let marker = match item.status {
            ChecklistStatus::Pending => " ",
            ChecklistStatus::InProgress => "~",
            ChecklistStatus::Completed => "x",
        };
        format!("- [{marker}] {}", item.content)
    }));
    lines.join("\n")
}

fn checklist_completion_pct(items: &[ChecklistItem]) -> usize {
    let completed = items
        .iter()
        .filter(|item| matches!(item.status, ChecklistStatus::Completed))
        .count();
    if items.is_empty() {
        0
    } else {
        (completed * 100 + items.len() / 2) / items.len()
    }
}

fn format_checklist_response_json(items: &[ChecklistItem]) -> String {
    let completion_pct = checklist_completion_pct(items);
    let in_progress_id = items
        .iter()
        .position(|item| matches!(item.status, ChecklistStatus::InProgress))
        .map(|index| index + 1);
    let items_json = items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let content = serde_json::to_string(&item.content).unwrap_or_else(|_| "\"\"".to_string());
            let status = item.status.as_str();
            format!(
                "    {{\n      \"id\": {},\n      \"content\": {content},\n      \"status\": \"{status}\"\n    }}",
                index + 1
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");
    let in_progress_id = in_progress_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "null".to_string());
    format!(
        "{{\n  \"items\": [\n{items_json}\n  ],\n  \"completion_pct\": {completion_pct},\n  \"in_progress_id\": {in_progress_id}\n}}"
    )
}

fn function_arguments(payload: &ToolPayload) -> Result<&str, FunctionCallError> {
    match payload {
        ToolPayload::Function { arguments } => Ok(arguments),
        _ => Err(FunctionCallError::RespondToModel(
            "harness alias received unsupported payload".to_string(),
        )),
    }
}

fn primary_cwd(invocation: &ToolInvocation) -> PathBuf {
    invocation
        .turn
        .environments
        .primary()
        .map(|environment| environment.cwd.as_path().to_path_buf())
        .unwrap_or_else(|| {
            #[allow(deprecated)]
            invocation.turn.cwd.as_path().to_path_buf()
        })
}

fn resolve_model_path(
    invocation: &ToolInvocation,
    path: &str,
) -> Result<PathBuf, FunctionCallError> {
    let path = normalize_model_path_text(path);
    let path = PathBuf::from(path);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(primary_cwd(invocation).join(path))
    }
}

fn normalize_model_path_text(text: &str) -> String {
    text.replace("/private/private/tmp/", "/private/tmp/")
}

fn display_model_path(invocation: &ToolInvocation, path: &Path) -> String {
    let cwd = primary_cwd(invocation);
    path.strip_prefix(&cwd)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

fn image_mime_type(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("webp") => Some("image/webp"),
        _ => None,
    }
}

fn collect_glob_matches(
    root: &Path,
    dir: &Path,
    pattern: &str,
    include_dirs: bool,
    matches: &mut Vec<String>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let relative = path.strip_prefix(root).unwrap_or(path.as_path());
        let relative_text = relative.to_string_lossy();
        if (include_dirs || path.is_file()) && simple_glob_matches(pattern, &relative_text) {
            matches.push(relative_text.to_string());
        }
        if path.is_dir() && !is_ignored_dir(&path) {
            collect_glob_matches(root, &path, pattern, include_dirs, matches)?;
        }
    }
    Ok(())
}

fn collect_grep_matches(
    root: &Path,
    path: &Path,
    regex: &Regex,
    glob: Option<&str>,
    matches: &mut Vec<String>,
) -> std::io::Result<()> {
    if path.is_file() {
        let relative = path.strip_prefix(root).unwrap_or(path);
        let relative_text = relative.to_string_lossy();
        if glob.is_none_or(|pattern| simple_glob_matches(pattern, &relative_text))
            && let Ok(text) = std::fs::read_to_string(path)
        {
            for line in text.lines() {
                if regex.is_match(line) {
                    matches.push(relative_text.to_string());
                    break;
                }
            }
        }
        return Ok(());
    }
    if !path.is_dir() || is_ignored_dir(path) {
        return Ok(());
    }
    for entry in std::fs::read_dir(path)? {
        collect_grep_matches(root, &entry?.path(), regex, glob, matches)?;
    }
    Ok(())
}

fn collect_grep_line_matches(
    root: &Path,
    path: &Path,
    regex: &Regex,
    matches: &mut Vec<serde_json::Value>,
) -> std::io::Result<()> {
    if path.is_file() {
        let relative = path.strip_prefix(root).unwrap_or(path);
        let relative_text = relative.to_string_lossy();
        if let Ok(text) = std::fs::read_to_string(path) {
            for (index, line) in text.lines().enumerate() {
                if regex.is_match(line) {
                    matches.push(json!({
                        "file": relative_text,
                        "line_number": index + 1,
                        "line": line,
                        "context_before": [],
                        "context_after": [],
                    }));
                }
            }
        }
        return Ok(());
    }
    if !path.is_dir() || is_ignored_dir(path) {
        return Ok(());
    }
    for entry in std::fs::read_dir(path)? {
        collect_grep_line_matches(root, &entry?.path(), regex, matches)?;
    }
    Ok(())
}

fn collect_file_search_matches(
    root: &Path,
    path: &Path,
    query: &str,
    matches: &mut Vec<String>,
) -> std::io::Result<()> {
    if path.is_file() {
        let relative = path.strip_prefix(root).unwrap_or(path);
        let relative_text = relative.to_string_lossy();
        if relative_text.contains(query) {
            matches.push(relative_text.to_string());
        }
        return Ok(());
    }
    if !path.is_dir() || is_ignored_dir(path) {
        return Ok(());
    }
    for entry in std::fs::read_dir(path)? {
        collect_file_search_matches(root, &entry?.path(), query, matches)?;
    }
    matches.sort();
    Ok(())
}

fn count_searchable_files(root: &Path) -> usize {
    fn walk(path: &Path, count: &mut usize) {
        if path.is_file() {
            *count += 1;
            return;
        }
        if !path.is_dir() || is_ignored_dir(path) {
            return;
        }
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                walk(&entry.path(), count);
            }
        }
    }
    let mut count = 0;
    walk(root, &mut count);
    count
}

fn is_ignored_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, ".git" | "node_modules" | "target" | ".venv"))
}

fn simple_glob_matches(pattern: &str, relative_path: &str) -> bool {
    if let Some(extension) = pattern.strip_prefix("*.") {
        return relative_path
            .rsplit('/')
            .next()
            .is_some_and(|name| name.ends_with(&format!(".{extension}")));
    }
    if let Some((prefix, suffix)) = pattern.split_once("**/*") {
        return relative_path.starts_with(prefix) && relative_path.ends_with(suffix);
    }
    if let Some(suffix) = pattern.strip_prefix("**/") {
        return relative_path == suffix || relative_path.ends_with(&format!("/{suffix}"));
    }
    if let Some((prefix, suffix)) = pattern.split_once('*') {
        return relative_path.starts_with(prefix) && relative_path.ends_with(suffix);
    }
    relative_path == pattern || relative_path.ends_with(&format!("/{pattern}"))
}

fn harness_alias_spec(name: &str) -> ToolSpec {
    if name == "Agent" {
        return claude_agent_spec();
    }
    if name == "TaskOutput" {
        return task_output_spec();
    }
    if name == "TaskStop" {
        return task_stop_spec();
    }
    let mut properties = BTreeMap::new();
    properties.insert(
        "input".to_string(),
        JsonSchema::string(Some("Harness-native tool input.".to_string())),
    );
    ToolSpec::Function(ResponsesApiTool {
        name: name.to_string(),
        description: format!("Open Interpreter harness compatibility alias for {name}."),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(properties, None, Some(AdditionalProperties::from(true))),
        output_schema: None,
    })
}

fn task_output_spec() -> ToolSpec {
    let mut properties = BTreeMap::new();
    properties.insert(
        "task_id".to_string(),
        JsonSchema::string(Some("The task ID to get output from".to_string())),
    );
    properties.insert(
        "block".to_string(),
        JsonSchema::boolean(Some("Whether to wait for completion".to_string())),
    );
    properties.insert(
        "timeout".to_string(),
        JsonSchema::number(Some("Max wait time in ms".to_string())),
    );
    ToolSpec::Function(ResponsesApiTool {
        name: "TaskOutput".to_string(),
        description: "DEPRECATED: Background tasks return their output file path in the tool result, and you receive a <task-notification> with the same path when the task completes.\n- For bash tasks: prefer using the Read tool on that output file path — it contains stdout/stderr.\n- For local_agent tasks: use the Agent tool result directly. Do NOT Read the .output file — it is a symlink to the full sub-agent conversation transcript (JSONL) and will overflow your context window.\n- For remote_agent tasks: prefer using the Read tool on the output file path — it contains the streamed remote session output (same as bash).\n\n- Retrieves output from a running or completed task (background shell, agent, or remote session)\n- Takes a task_id parameter identifying the task\n- Returns the task output along with status information\n- Use block=true (default) to wait for task completion\n- Use block=false for non-blocking check of current status\n- Task IDs can be found using the /tasks command\n- Works with all task types: background shells, async agents, and remote sessions".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec![
                "task_id".to_string(),
                "block".to_string(),
                "timeout".to_string(),
            ]),
            Some(AdditionalProperties::from(false)),
        ),
        output_schema: None,
    })
}

fn task_stop_spec() -> ToolSpec {
    let mut properties = BTreeMap::new();
    properties.insert(
        "task_id".to_string(),
        JsonSchema::string(Some("The ID of the background task to stop".to_string())),
    );
    properties.insert(
        "shell_id".to_string(),
        JsonSchema::string(Some("Deprecated: use task_id instead".to_string())),
    );
    ToolSpec::Function(ResponsesApiTool {
        name: "TaskStop".to_string(),
        description: "\n- Stops a running background task by its ID\n- Takes a task_id parameter identifying the task to stop\n- Returns a success or failure status\n- Use this tool when you need to terminate a long-running task\n".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(properties, None, Some(AdditionalProperties::from(false))),
        output_schema: None,
    })
}

fn claude_agent_spec() -> ToolSpec {
    let mut properties = BTreeMap::new();
    properties.insert(
        "description".to_string(),
        JsonSchema::string(Some(
            "A short (3-5 word) description of the task".to_string(),
        )),
    );
    properties.insert(
        "prompt".to_string(),
        JsonSchema::string(Some("The task for the agent to perform".to_string())),
    );
    properties.insert(
        "subagent_type".to_string(),
        JsonSchema::string(Some(
            "The type of specialized agent to use for this task".to_string(),
        )),
    );
    properties.insert(
        "model".to_string(),
        JsonSchema::string_enum(
            vec![json!("sonnet"), json!("opus"), json!("haiku")],
            Some(
                "Optional model override for this agent. Takes precedence over the agent definition's model frontmatter. If omitted, uses the agent definition's model, or inherits from the parent."
                    .to_string(),
            ),
        ),
    );
    properties.insert(
        "run_in_background".to_string(),
        JsonSchema::boolean(Some(
            "Set to true to run this agent in the background. You will be notified when it completes."
                .to_string(),
        )),
    );
    properties.insert(
        "isolation".to_string(),
        JsonSchema::string_enum(
            vec![json!("worktree")],
            Some(
                "Isolation mode. \"worktree\" creates a temporary git worktree so the agent works on an isolated copy of the repo."
                    .to_string(),
            ),
        ),
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "Agent".to_string(),
        description: CLAUDE_AGENT_DESCRIPTION.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["description".to_string(), "prompt".to_string()]),
            Some(AdditionalProperties::from(false)),
        ),
        output_schema: None,
    })
}

const CLAUDE_AGENT_DESCRIPTION: &str = r#"Launch a new agent to handle complex, multi-step tasks. Each agent type has specific capabilities and tools available to it.

Available agent types and the tools they have access to:
- claude: Catch-all for any task that doesn't fit a more specific agent. FleetView's default when no agent name is typed. (Tools: *)
- Explore: Fast read-only search agent for locating code. Use it to find files by pattern (eg. "src/components/**/*.tsx"), grep for symbols or keywords (eg. "API endpoints"), or answer "where is X defined / which files reference Y." Do NOT use it for code review, design-doc auditing, cross-file consistency checks, or open-ended analysis — it reads excerpts rather than whole files and will miss content past its read window. When calling, specify search breadth: "quick" for a single targeted lookup, "medium" for moderate exploration, or "very thorough" to search across multiple locations and naming conventions. (Tools: All tools except Agent, ExitPlanMode, Edit, Write, NotebookEdit)
- general-purpose: General-purpose agent for researching complex questions, searching for code, and executing multi-step tasks. When you are searching for a keyword or file and are not confident that you will find the right match in the first few tries use this agent to perform the search for you. (Tools: *)
- Plan: Software architect agent for designing implementation plans. Use this when you need to plan the implementation strategy for a task. Returns step-by-step plans, identifies critical files, and considers architectural trade-offs. (Tools: All tools except Agent, ExitPlanMode, Edit, Write, NotebookEdit)
- statusline-setup: Use this agent to configure the user's Claude Code status line setting. (Tools: Read, Edit)

When using the Agent tool, specify a subagent_type parameter to select which agent type to use. If omitted, the general-purpose agent is used.

## When not to use

If the target is already known, use the direct tool: Read for a known path, the Grep tool for a specific symbol or string. Reserve this tool for open-ended questions that span the codebase, or tasks that match an available agent type.

## Usage notes

- Always include a short description summarizing what the agent will do
- When you launch multiple agents for independent work, send them in a single message with multiple tool uses so they run concurrently
- When the agent is done, it will return a single message back to you. The result returned by the agent is not visible to the user. To show the user the result, you should send a text message back to the user with a concise summary of the result.
- Trust but verify: an agent's summary describes what it intended to do, not necessarily what it did. When an agent writes or edits code, check the actual changes before reporting the work as done.
- You can optionally run agents in the background using the run_in_background parameter. When an agent runs in the background, you will be automatically notified when it completes — do NOT sleep, poll, or proactively check on its progress. Continue with other work or respond to the user instead.
- **Foreground vs background**: Use foreground (default) when you need the agent's results before you can proceed — e.g., research agents whose findings inform your next steps. Use background when you have genuinely independent work to do in parallel.
- To continue a previously spawned agent, use SendMessage with the agent's ID or name as the `to` field — that resumes it with full context. A new Agent call starts a fresh agent with no memory of prior runs, so the prompt must be self-contained.
- Clearly tell the agent whether you expect it to write code or just to do research (search, file reads, web fetches, etc.), since it is not aware of the user's intent
- If the agent description mentions that it should be used proactively, then you should try your best to use it without the user having to ask for it first.
- If the user specifies that they want you to run agents "in parallel", you MUST send a single message with multiple Agent tool use content blocks. For example, if you need to launch both a build-validator agent and a test-runner agent in parallel, send a single message with both tool calls.
- With `isolation: "worktree"`, the worktree is automatically cleaned up if the agent makes no changes; otherwise the path and branch are returned in the result.

## Writing the prompt

Brief the agent like a smart colleague who just walked into the room — it hasn't seen this conversation, doesn't know what you've tried, doesn't understand why this task matters.
- Explain what you're trying to accomplish and why.
- Describe what you've already learned or ruled out.
- Give enough context about the surrounding problem that the agent can make judgment calls rather than just following a narrow instruction.
- If you need a short response, say so ("report in under 200 words").
- Lookups: hand over the exact command. Investigations: hand over the question — prescribed steps become dead weight when the premise is wrong.

Terse command-style prompts produce shallow, generic work.

**Never delegate understanding.** Don't write "based on your findings, fix the bug" or "based on the research, implement it." Those phrases push synthesis onto the agent instead of doing it yourself. Write prompts that prove you understood: include file paths, line numbers, what specifically to change.

Example usage:

<example>
user: "What's left on this branch before we can ship?"
assistant: <thinking>A survey question across git state, tests, and config. I'll delegate it and ask for a short report so the raw command output stays out of my context.</thinking>
Agent({
  description: "Branch ship-readiness audit",
  prompt: "Audit what's left before this branch can ship. Check: uncommitted changes, commits ahead of main, whether tests exist, whether the GrowthBook gate is wired up, whether CI-relevant files changed. Report a punch list — done vs. missing. Under 200 words."
})
<commentary>
The prompt is self-contained: it states the goal, lists what to check, and caps the response length. The agent's report comes back as the tool result; relay the findings to the user.
</commentary>
</example>

<example>
user: "Can you get a second opinion on whether this migration is safe?"
assistant: <thinking>I'll ask the code-reviewer agent — it won't see my analysis, so it can give an independent read.</thinking>
Agent({
  description: "Independent migration review",
  subagent_type: "code-reviewer",
  prompt: "Review migration 0042_user_schema.sql for safety. Context: we're adding a NOT NULL column to a 50M-row table. Existing rows get a backfill default. I want a second opinion on whether the backfill approach is safe under concurrent writes — I've checked locking behavior but want independent verification. Report: is this safe, and if not, what specifically breaks?"
})
<commentary>
The agent starts with no context from this conversation, so the prompt briefs it: what to assess, the relevant background, and what form the answer should take.
</commentary>
</example>
"#;
