use crate::client_common::Prompt;
use crate::compact::KIMI_CLI_COMPACTION_SYSTEM_PROMPT;
use crate::event_mapping::is_contextual_dev_content_item;
use crate::event_mapping::is_contextual_user_message_content;
use codex_chat_wire_compat::ToolKinds;
use codex_chat_wire_compat::ToolOutputKind;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::LocalShellAction;
use codex_protocol::models::ReasoningItemContent;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ReasoningControl;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use serde_json::Value;
use serde_json::json;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::sync::Mutex;

const KIMI_CLI_DEFAULT_MAX_TOKENS: u32 = 32_000;
const KIMI_CLI_SYSTEM_PROMPT_TEMPLATE: &str = include_str!("kimi_cli_prompt.md");
const KIMI_LIST_DIR_ROOT_WIDTH: usize = 30;
const KIMI_LIST_DIR_CHILD_WIDTH: usize = 10;
const KIMI_AGENTS_MD_MAX_BYTES: usize = 32 * 1024;
static KIMI_WORK_DIR_LS_CACHE: LazyLock<Mutex<std::collections::HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));

pub(crate) fn build_request(
    prompt: &Prompt,
    model_info: &ModelInfo,
    reasoning_effort: Option<ReasoningEffort>,
    conversation_id: &str,
    session_source: Option<&SessionSource>,
    _yolo_mode: bool,
) -> Result<(Value, ToolKinds), serde_json::Error> {
    let system_prompt = build_system_prompt(prompt, session_source, conversation_id);
    let prompt_cache_key = kimi_prompt_cache_key(conversation_id);
    let mut messages = vec![json!({
        "role": "system",
        "content": system_prompt,
    })];
    messages.extend(build_messages(&prompt.get_formatted_input())?);
    let tools = build_tools(&prompt.tools)?;
    let tool_kinds = prompt
        .tools
        .iter()
        .map(|tool| (tool.name().to_string(), ToolOutputKind::Function))
        .collect();

    let mut request = json!({
            "model": model_info.slug,
            "messages": messages,
            "max_tokens": KIMI_CLI_DEFAULT_MAX_TOKENS,
            "prompt_cache_key": prompt_cache_key,
            "stream": true,
            "stream_options": {
                "include_usage": true,
            },
            "tools": tools,
    });
    if reasoning_effort.is_some() {
        apply_reasoning_effort(&mut request, reasoning_effort);
    }
    if model_info.reasoning_control == ReasoningControl::ThinkingToggle {
        request
            .as_object_mut()
            .expect("request is an object")
            .entry("thinking".to_string())
            .or_insert_with(|| json!({ "type": "disabled" }));
    }

    Ok((request, tool_kinds))
}

fn kimi_prompt_cache_key(conversation_id: &str) -> String {
    if let Ok(value) = std::env::var("OPEN_INTERPRETER_KIMI_PROMPT_CACHE_KEY_OVERRIDE")
        && !value.trim().is_empty()
    {
        return value;
    }

    conversation_id.to_string()
}

fn apply_reasoning_effort(request: &mut Value, reasoning_effort: Option<ReasoningEffort>) {
    let Some(effort) = reasoning_effort else {
        return;
    };
    let Some(request_object) = request.as_object_mut() else {
        return;
    };

    match effort {
        ReasoningEffort::None => {}
        ReasoningEffort::Minimal | ReasoningEffort::Low => {
            request_object.insert("reasoning_effort".to_string(), json!("low"));
            request_object.insert("thinking".to_string(), json!({ "type": "enabled" }));
        }
        ReasoningEffort::Medium => {
            request_object.insert("reasoning_effort".to_string(), json!("medium"));
            request_object.insert("thinking".to_string(), json!({ "type": "enabled" }));
        }
        ReasoningEffort::High | ReasoningEffort::XHigh => {
            request_object.insert("reasoning_effort".to_string(), json!("high"));
            request_object.insert("thinking".to_string(), json!({ "type": "enabled" }));
        }
    }
}

fn build_system_prompt(
    prompt: &Prompt,
    session_source: Option<&SessionSource>,
    conversation_id: &str,
) -> String {
    if prompt.tools.is_empty() && prompt.base_instructions.text == KIMI_CLI_COMPACTION_SYSTEM_PROMPT
    {
        return prompt.base_instructions.text.clone();
    }

    let work_dir = prompt
        .cwd
        .as_deref()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let kimi_os = current_kimi_os();
    let mut rendered = KIMI_CLI_SYSTEM_PROMPT_TEMPLATE.to_string();
    rendered = render_conditional_block(
        rendered,
        r#"{% if KIMI_OS == "Windows" %}"#,
        "{% endif %}",
        kimi_os == "Windows",
    );
    rendered = render_conditional_block(
        rendered,
        "{% if KIMI_ADDITIONAL_DIRS_INFO %}",
        "{% endif %}",
        /*include_block*/ false,
    );

    for (name, value) in [
        ("ROLE_ADDITIONAL", role_additional(session_source)),
        ("KIMI_OS", kimi_os),
        ("KIMI_SHELL", kimi_shell()),
        ("KIMI_NOW", current_kimi_now()),
        ("KIMI_WORK_DIR", work_dir.as_path().display().to_string()),
        (
            "KIMI_WORK_DIR_LS",
            cached_work_dir_listing(conversation_id, &work_dir),
        ),
        ("KIMI_AGENTS_MD", load_kimi_agents_md(&work_dir)),
        ("KIMI_SKILLS", discover_kimi_skills(&work_dir)),
        ("KIMI_ADDITIONAL_DIRS_INFO", String::new()),
    ] {
        rendered = rendered.replace(format!("${{{name}}}").as_str(), value.as_str());
    }

    let mut rendered = rendered.trim_end_matches('\n').to_string();
    let developer_instructions = collect_developer_instruction_text(&prompt.input);
    if !developer_instructions.is_empty() {
        rendered.push_str("\n\n# Additional Developer Instructions\n\n");
        rendered.push_str(&developer_instructions);
    }
    rendered
}

fn cached_work_dir_listing(conversation_id: &str, work_dir: &Path) -> String {
    let key = format!("{conversation_id}:{}", work_dir.display());
    let mut cache = KIMI_WORK_DIR_LS_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cache
        .entry(key)
        .or_insert_with(|| list_directory(work_dir))
        .clone()
}

#[derive(Clone, Copy)]
pub(super) struct MessageBuildOptions {
    pub omitted_reasoning_placeholder: Option<&'static str>,
    pub compact_function_arguments: bool,
    pub merge_assistant_text_with_following_tool_calls: bool,
    pub preserve_empty_tool_output: bool,
    pub trim_user_message_trailing_newlines: bool,
}

impl MessageBuildOptions {
    pub fn kimi_cli() -> Self {
        Self {
            omitted_reasoning_placeholder: None,
            compact_function_arguments: false,
            merge_assistant_text_with_following_tool_calls: false,
            preserve_empty_tool_output: false,
            trim_user_message_trailing_newlines: true,
        }
    }

    pub fn kimi_code() -> Self {
        Self {
            omitted_reasoning_placeholder: None,
            compact_function_arguments: true,
            merge_assistant_text_with_following_tool_calls: false,
            preserve_empty_tool_output: false,
            trim_user_message_trailing_newlines: false,
        }
    }

    pub fn deepseek_tui() -> Self {
        Self {
            omitted_reasoning_placeholder: Some("(reasoning omitted)"),
            compact_function_arguments: true,
            merge_assistant_text_with_following_tool_calls: true,
            preserve_empty_tool_output: true,
            trim_user_message_trailing_newlines: true,
        }
    }
}

pub(super) fn build_messages(
    items: &[ResponseItem],
) -> Result<impl Iterator<Item = Value>, serde_json::Error> {
    build_messages_with_options(items, MessageBuildOptions::kimi_cli())
}

pub(super) fn build_messages_with_options(
    items: &[ResponseItem],
    options: MessageBuildOptions,
) -> Result<impl Iterator<Item = Value>, serde_json::Error> {
    let mut messages = Vec::new();
    let mut pending_tool_calls = Vec::new();
    let mut awaiting_tool_call_ids = Vec::new();
    let mut pending_assistant_content = None;
    let mut pending_reasoning_content = String::new();

    for item in items {
        match item {
            ResponseItem::Message { role, content, .. } => match role.as_str() {
                "assistant" => {
                    if let Some(message_content) = convert_message_content(content) {
                        if message_content.as_str().is_some_and(str::is_empty) {
                            continue;
                        }
                        if !pending_tool_calls.is_empty() {
                            pending_assistant_content = Some(message_content);
                            continue;
                        }
                        discard_unanswered_tool_calls(
                            &mut pending_tool_calls,
                            &mut awaiting_tool_call_ids,
                            &mut pending_reasoning_content,
                        );
                        if options.merge_assistant_text_with_following_tool_calls {
                            push_pending_assistant_content(
                                &mut messages,
                                &mut pending_assistant_content,
                                &mut pending_reasoning_content,
                            );
                            pending_assistant_content = Some(message_content);
                            continue;
                        }
                        push_pending_assistant_content(
                            &mut messages,
                            &mut pending_assistant_content,
                            &mut pending_reasoning_content,
                        );
                        let mut message = json!({
                            "role": "assistant",
                            "content": message_content,
                        });
                        attach_reasoning_content(&mut message, &mut pending_reasoning_content);
                        messages.push(message);
                    }
                }
                "user" => {
                    if is_contextual_user_message_content(content)
                        || content.iter().any(is_contextual_dev_content_item)
                    {
                        continue;
                    }
                    discard_unanswered_tool_calls(
                        &mut pending_tool_calls,
                        &mut awaiting_tool_call_ids,
                        &mut pending_reasoning_content,
                    );
                    push_pending_assistant_content(
                        &mut messages,
                        &mut pending_assistant_content,
                        &mut pending_reasoning_content,
                    );
                    pending_reasoning_content.clear();
                    if let Some(message_content) = convert_user_message_content(content, options) {
                        messages.push(json!({
                            "role": "user",
                            "content": message_content,
                        }));
                    }
                }
                "developer" => {
                    discard_unanswered_tool_calls(
                        &mut pending_tool_calls,
                        &mut awaiting_tool_call_ids,
                        &mut pending_reasoning_content,
                    );
                    push_pending_assistant_content(
                        &mut messages,
                        &mut pending_assistant_content,
                        &mut pending_reasoning_content,
                    );
                    pending_reasoning_content.clear();
                }
                _ => {}
            },
            ResponseItem::FunctionCall {
                name,
                arguments,
                call_id,
                ..
            } => {
                if !options.merge_assistant_text_with_following_tool_calls {
                    push_pending_assistant_content(
                        &mut messages,
                        &mut pending_assistant_content,
                        &mut pending_reasoning_content,
                    );
                }
                awaiting_tool_call_ids.clear();
                pending_tool_calls.push(json!({
                    "type": "function",
                    "id": call_id,
                    "function": {
                        "name": name,
                        "arguments": format_function_arguments(
                            arguments,
                            options.compact_function_arguments
                        ),
                    }
                }));
            }
            ResponseItem::CustomToolCall {
                call_id,
                name,
                input,
                ..
            } => {
                if !options.merge_assistant_text_with_following_tool_calls {
                    push_pending_assistant_content(
                        &mut messages,
                        &mut pending_assistant_content,
                        &mut pending_reasoning_content,
                    );
                }
                awaiting_tool_call_ids.clear();
                pending_tool_calls.push(json!({
                    "type": "function",
                    "id": call_id,
                    "function": {
                        "name": name,
                        "arguments": json!({ "input": input }).to_string(),
                    }
                }));
            }
            ResponseItem::LocalShellCall {
                id,
                call_id,
                action,
                ..
            } => {
                if !options.merge_assistant_text_with_following_tool_calls {
                    push_pending_assistant_content(
                        &mut messages,
                        &mut pending_assistant_content,
                        &mut pending_reasoning_content,
                    );
                }
                let call_id = call_id.clone().or_else(|| id.clone()).ok_or_else(|| {
                    serde_json::Error::io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "local_shell history item missing call id",
                    ))
                })?;
                let arguments = match action {
                    LocalShellAction::Exec(exec) => json!({
                        "command": exec.command,
                        "timeout": exec.timeout_ms.map(|timeout_ms| timeout_ms / 1000),
                    })
                    .to_string(),
                };
                awaiting_tool_call_ids.clear();
                pending_tool_calls.push(json!({
                    "type": "function",
                    "id": call_id,
                    "function": {
                        "name": "Shell",
                        "arguments": arguments,
                    }
                }));
            }
            ResponseItem::FunctionCallOutput { call_id, output } => {
                push_tool_output_if_expected(
                    &mut messages,
                    &mut pending_tool_calls,
                    &mut awaiting_tool_call_ids,
                    &mut pending_assistant_content,
                    &mut pending_reasoning_content,
                    call_id,
                    kimi_tool_output_content(output, options),
                );
            }
            ResponseItem::CustomToolCallOutput {
                call_id, output, ..
            } => {
                push_tool_output_if_expected(
                    &mut messages,
                    &mut pending_tool_calls,
                    &mut awaiting_tool_call_ids,
                    &mut pending_assistant_content,
                    &mut pending_reasoning_content,
                    call_id,
                    kimi_tool_output_content(output, options),
                );
            }
            ResponseItem::Reasoning { content, .. } => {
                append_reasoning_content(
                    &mut pending_reasoning_content,
                    content.as_deref(),
                    options.omitted_reasoning_placeholder,
                );
            }
            ResponseItem::ToolSearchCall { .. }
            | ResponseItem::ToolSearchOutput { .. }
            | ResponseItem::WebSearchCall { .. }
            | ResponseItem::ImageGenerationCall { .. }
            | ResponseItem::GhostSnapshot { .. }
            | ResponseItem::Compaction { .. }
            | ResponseItem::Other => {}
        }
    }

    push_pending_assistant_content(
        &mut messages,
        &mut pending_assistant_content,
        &mut pending_reasoning_content,
    );
    discard_unanswered_tool_calls(
        &mut pending_tool_calls,
        &mut awaiting_tool_call_ids,
        &mut pending_reasoning_content,
    );
    Ok(messages.into_iter())
}

pub(super) fn build_tools(tools: &[ToolSpec]) -> Result<Vec<Value>, serde_json::Error> {
    let mut converted = Vec::new();
    for tool in tools {
        let ToolSpec::Function(ResponsesApiTool {
            name,
            description,
            parameters,
            ..
        }) = tool
        else {
            continue;
        };
        converted.push(json!({
            "type": "function",
            "function": {
                "name": name,
                "description": description,
                "parameters": parameters,
            }
        }));
    }
    Ok(converted)
}

fn flush_pending_tool_calls(
    messages: &mut Vec<Value>,
    pending_tool_calls: &mut Vec<Value>,
    awaiting_tool_call_ids: &mut Vec<String>,
    pending_reasoning_content: &mut String,
) {
    if pending_tool_calls.is_empty() {
        return;
    }
    awaiting_tool_call_ids.extend(
        pending_tool_calls
            .iter()
            .filter_map(|tool_call| tool_call.get("id").and_then(Value::as_str))
            .map(str::to_string),
    );
    let mut message = json!({
        "role": "assistant",
        "tool_calls": std::mem::take(pending_tool_calls),
    });
    attach_reasoning_content(&mut message, pending_reasoning_content);
    messages.push(message);
}

fn flush_pending_tool_calls_with_content(
    messages: &mut Vec<Value>,
    pending_tool_calls: &mut Vec<Value>,
    awaiting_tool_call_ids: &mut Vec<String>,
    pending_reasoning_content: &mut String,
    content: Value,
) {
    if pending_tool_calls.is_empty() {
        return;
    }
    awaiting_tool_call_ids.extend(
        pending_tool_calls
            .iter()
            .filter_map(|tool_call| tool_call.get("id").and_then(Value::as_str))
            .map(str::to_string),
    );
    let mut message = json!({
        "role": "assistant",
        "content": content,
        "tool_calls": std::mem::take(pending_tool_calls),
    });
    attach_reasoning_content(&mut message, pending_reasoning_content);
    messages.push(message);
}

fn discard_unanswered_tool_calls(
    pending_tool_calls: &mut Vec<Value>,
    awaiting_tool_call_ids: &mut Vec<String>,
    pending_reasoning_content: &mut String,
) {
    pending_tool_calls.clear();
    awaiting_tool_call_ids.clear();
    pending_reasoning_content.clear();
}

fn push_pending_assistant_content(
    messages: &mut Vec<Value>,
    pending_assistant_content: &mut Option<Value>,
    pending_reasoning_content: &mut String,
) {
    let Some(content) = pending_assistant_content.take() else {
        return;
    };
    let mut message = json!({
        "role": "assistant",
        "content": content,
    });
    attach_reasoning_content(&mut message, pending_reasoning_content);
    messages.push(message);
}

fn push_tool_output_if_expected(
    messages: &mut Vec<Value>,
    pending_tool_calls: &mut Vec<Value>,
    awaiting_tool_call_ids: &mut Vec<String>,
    pending_assistant_content: &mut Option<Value>,
    pending_reasoning_content: &mut String,
    call_id: &str,
    content: Value,
) {
    if let Some(content) = pending_assistant_content.take() {
        flush_pending_tool_calls_with_content(
            messages,
            pending_tool_calls,
            awaiting_tool_call_ids,
            pending_reasoning_content,
            content,
        );
    } else {
        flush_pending_tool_calls(
            messages,
            pending_tool_calls,
            awaiting_tool_call_ids,
            pending_reasoning_content,
        );
    }
    if let Some(index) = awaiting_tool_call_ids
        .iter()
        .position(|awaiting_call_id| awaiting_call_id == call_id)
    {
        awaiting_tool_call_ids.remove(index);
        messages.push(json!({
            "role": "tool",
            "content": content,
            "tool_call_id": call_id,
        }));
    }
}

fn append_reasoning_content(
    pending_reasoning_content: &mut String,
    content: Option<&[ReasoningItemContent]>,
    omitted_reasoning_placeholder: Option<&str>,
) {
    let Some(content) = content else {
        if let Some(placeholder) = omitted_reasoning_placeholder {
            pending_reasoning_content.push_str(placeholder);
        }
        return;
    };
    if content.is_empty() {
        if let Some(placeholder) = omitted_reasoning_placeholder {
            pending_reasoning_content.push_str(placeholder);
        }
        return;
    }
    for item in content {
        match item {
            ReasoningItemContent::ReasoningText { text } | ReasoningItemContent::Text { text } => {
                pending_reasoning_content.push_str(text);
            }
        }
    }
}

fn format_function_arguments(arguments: &str, compact: bool) -> String {
    if compact {
        return compact_json_whitespace_preserving_order(arguments);
    }
    arguments.to_string()
}

fn compact_json_whitespace_preserving_order(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_string = false;
    let mut escaped = false;
    for ch in input.chars() {
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
            out.push(ch);
        } else if !ch.is_whitespace() {
            out.push(ch);
        }
    }
    out
}

fn attach_reasoning_content(message: &mut Value, pending_reasoning_content: &mut String) {
    if pending_reasoning_content.is_empty() {
        return;
    }
    if let Some(message_object) = message.as_object_mut() {
        message_object.insert(
            "reasoning_content".to_string(),
            Value::String(std::mem::take(pending_reasoning_content)),
        );
    }
}

fn convert_message_content(content: &[ContentItem]) -> Option<Value> {
    collapse_message_parts(convert_message_parts(content))
}

fn convert_user_message_content(
    content: &[ContentItem],
    options: MessageBuildOptions,
) -> Option<Value> {
    collapse_message_parts(convert_user_message_parts(content, options))
}

fn convert_message_parts(content: &[ContentItem]) -> Vec<Value> {
    content
        .iter()
        .map(|item| match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => json!({
                "type": "text",
                "text": text,
            }),
            ContentItem::InputImage { image_url, .. } => json!({
                "type": "image_url",
                "image_url": {
                    "url": image_url,
                    "id": null,
                }
            }),
        })
        .collect()
}

fn convert_user_message_parts(content: &[ContentItem], options: MessageBuildOptions) -> Vec<Value> {
    content
        .iter()
        .map(|item| match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                let text = if options.trim_user_message_trailing_newlines {
                    text.trim_end_matches('\n')
                } else {
                    text
                };
                json!({
                    "type": "text",
                    "text": text,
                })
            }
            ContentItem::InputImage { image_url, .. } => json!({
                "type": "image_url",
                "image_url": {
                    "url": image_url,
                    "id": null,
                }
            }),
        })
        .collect()
}

fn collapse_message_parts(parts: Vec<Value>) -> Option<Value> {
    match parts.as_slice() {
        [] => None,
        [single]
            if single
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind == "text") =>
        {
            single.get("text").cloned()
        }
        _ => Some(Value::Array(parts)),
    }
}

fn kimi_tool_output_content(
    output: &FunctionCallOutputPayload,
    options: MessageBuildOptions,
) -> Value {
    match &output.body {
        codex_protocol::models::FunctionCallOutputBody::Text(text) => {
            if output.success == Some(false) {
                if is_kimi_system_tool_text(text) {
                    return json!(text);
                }
                return json!(format!("<system>ERROR: {text}</system>"));
            }
            let text = safe_kimi_tool_text(text, options);
            json!(text)
        }
        codex_protocol::models::FunctionCallOutputBody::ContentItems(items) => {
            let content = items
                .iter()
                .map(kimi_output_content_item)
                .collect::<Vec<_>>();
            collapse_message_parts(content).unwrap_or_else(|| Value::Array(Vec::new()))
        }
    }
}

fn safe_kimi_tool_text(text: &str, options: MessageBuildOptions) -> String {
    if text.trim().is_empty() {
        if options.preserve_empty_tool_output {
            String::new()
        } else {
            "<system>Tool output is empty.</system>".to_string()
        }
    } else if text.contains('\0') {
        "<system>Tool returned non-text content.</system>".to_string()
    } else {
        text.to_string()
    }
}

fn is_kimi_system_tool_text(text: &str) -> bool {
    text.starts_with("<system>ERROR:")
        || text.starts_with("<system>Command executed successfully.")
        || text == "<system>Tool output is empty.</system>"
        || text == "<system>Tool returned non-text content.</system>"
}

fn kimi_output_content_item(item: &FunctionCallOutputContentItem) -> Value {
    match item {
        FunctionCallOutputContentItem::InputText { text } => json!({
            "type": "text",
            "text": if is_kimi_system_tool_text(text) {
                text.clone()
            } else {
                safe_kimi_tool_text(text, MessageBuildOptions::kimi_cli())
            },
        }),
        FunctionCallOutputContentItem::InputImage { image_url, .. } => json!({
            "type": "image_url",
            "image_url": {
                "url": image_url,
                "id": null,
            }
        }),
    }
}

fn role_additional(session_source: Option<&SessionSource>) -> String {
    if matches!(
        session_source,
        Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn { .. }))
    ) {
        "You are operating as a subagent instance spawned from a parent Kimi Code CLI conversation. Focus on the assigned subtask, keep your response concise, and do not assume direct access to the human user.".to_string()
    } else {
        String::new()
    }
}

fn current_kimi_os() -> String {
    match std::env::consts::OS {
        "macos" => "macOS".to_string(),
        "windows" => "Windows".to_string(),
        "linux" => "Linux".to_string(),
        other => other.to_string(),
    }
}

fn kimi_shell() -> String {
    if cfg!(windows) {
        "powershell (`powershell.exe`)".to_string()
    } else {
        "bash (`/bin/bash`)".to_string()
    }
}

fn load_kimi_agents_md(work_dir: &Path) -> String {
    let project_root = find_kimi_project_root(work_dir);
    let mut dirs = dirs_root_to_leaf(work_dir, &project_root);
    let mut discovered = Vec::new();
    for dir in dirs.drain(..) {
        let kimi_agents = dir.join(".kimi").join("AGENTS.md");
        if let Some(content) = read_non_empty_file(&kimi_agents) {
            discovered.push((kimi_agents, content));
        }
        for candidate in [dir.join("AGENTS.md"), dir.join("agents.md")] {
            if let Some(content) = read_non_empty_file(&candidate) {
                discovered.push((candidate, content));
                break;
            }
        }
    }

    let mut remaining = KIMI_AGENTS_MD_MAX_BYTES;
    let mut budgeted = Vec::with_capacity(discovered.len());
    for (index, (path, content)) in discovered.iter().enumerate().rev() {
        let annotation = format!("<!-- From: {} -->\n", path.display());
        let separator_cost = if index < discovered.len() - 1 {
            "\n\n".len()
        } else {
            0
        };
        let overhead = annotation.len() + separator_cost;
        remaining = remaining.saturating_sub(overhead);
        if remaining == 0 {
            budgeted.push((path, String::new()));
            continue;
        }
        let mut content = content.clone();
        if content.len() > remaining {
            content.truncate(remaining);
            content = content.trim().to_string();
        }
        remaining = remaining.saturating_sub(content.len());
        budgeted.push((path, content));
    }

    budgeted
        .into_iter()
        .rev()
        .filter_map(|(path, content)| {
            (!content.is_empty()).then(|| format!("<!-- From: {} -->\n{content}", path.display()))
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn find_kimi_project_root(work_dir: &Path) -> PathBuf {
    let mut current = work_dir.to_path_buf();
    loop {
        if current.join(".git").exists() {
            return current;
        }
        let Some(parent) = current.parent() else {
            return work_dir.to_path_buf();
        };
        if parent == current {
            return work_dir.to_path_buf();
        }
        current = parent.to_path_buf();
    }
}

fn dirs_root_to_leaf(work_dir: &Path, project_root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut current = work_dir.to_path_buf();
    loop {
        dirs.push(current.clone());
        if current == project_root {
            break;
        }
        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent.to_path_buf();
    }
    dirs.reverse();
    dirs
}

fn read_non_empty_file(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?.trim().to_string();
    (!content.is_empty()).then_some(content)
}

fn collect_developer_instruction_text(items: &[ResponseItem]) -> String {
    items
        .iter()
        .filter_map(|item| match item {
            ResponseItem::Message { role, content, .. } if role == "developer" => Some(content),
            _ => None,
        })
        .flat_map(|content| content.iter())
        .filter_map(|item| match item {
            ContentItem::InputText { text } if !is_contextual_dev_content_item(item) => {
                let trimmed = text.trim();
                (!trimmed.is_empty()).then_some(trimmed)
            }
            ContentItem::OutputText { text } => {
                let trimmed = text.trim();
                (!trimmed.is_empty()).then_some(trimmed)
            }
            ContentItem::InputText { .. } => None,
            ContentItem::InputImage { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn discover_kimi_skills(work_dir: &Path) -> String {
    let mut seen = HashSet::new();
    let mut skills = kimi_skill_roots(
        work_dir,
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from),
        std::env::var_os("KIMI_CLI_SOURCE_DIR").map(PathBuf::from),
    )
    .into_iter()
    .flat_map(|root| discover_skills_in_root(&root.path, root.scope))
    .filter(|skill| seen.insert(skill.name.to_ascii_lowercase()))
    .collect::<Vec<_>>();
    skills.sort_by(|left, right| left.name.cmp(&right.name));
    if skills.is_empty() {
        builtin_kimi_skill_listing().to_string()
    } else {
        format_kimi_skills_for_prompt(skills)
    }
}

fn builtin_kimi_skill_listing() -> &'static str {
    "### Built-in\n- kimi-cli-help\n  - Path: /tmp/kimi-cli/src/kimi_cli/skills/kimi-cli-help/SKILL.md\n  - Description: Answer Kimi Code CLI usage, configuration, and troubleshooting questions. Use when user asks about Kimi Code CLI installation, setup, configuration, slash commands, keyboard shortcuts, MCP integration, providers, environment variables, how something works internally, or any questions about Kimi Code CLI itself.\n- skill-creator\n  - Path: /tmp/kimi-cli/src/kimi_cli/skills/skill-creator/SKILL.md\n  - Description: Guide for creating effective skills. This skill should be used when users want to create a new skill (or update an existing skill) that extends Kimi's capabilities with specialized knowledge, workflows, or tool integrations."
}

fn kimi_skill_roots(
    work_dir: &Path,
    home_dir: Option<PathBuf>,
    kimi_source_dir: Option<PathBuf>,
) -> Vec<KimiSkillRoot> {
    let mut roots = Vec::new();
    if let Some(root) = first_existing_dir([
        work_dir.join(".kimi/skills"),
        work_dir.join(".claude/skills"),
        work_dir.join(".codex/skills"),
    ]) {
        roots.push(KimiSkillRoot::new(root, KimiSkillScope::Project));
    }
    if let Some(root) = first_existing_dir([work_dir.join(".agents/skills")]) {
        roots.push(KimiSkillRoot::new(root, KimiSkillScope::Project));
    }
    if let Some(home) = home_dir {
        if let Some(root) = first_existing_dir([
            home.join(".kimi/skills"),
            home.join(".claude/skills"),
            home.join(".codex/skills"),
        ]) {
            roots.push(KimiSkillRoot::new(root, KimiSkillScope::User));
        }
        if let Some(root) = first_existing_dir([
            home.join(".config/agents/skills"),
            home.join(".agents/skills"),
        ]) {
            roots.push(KimiSkillRoot::new(root, KimiSkillScope::User));
        }
    }
    if let Some(root) = builtin_kimi_skills_root(kimi_source_dir.as_deref()) {
        roots.push(KimiSkillRoot::new(root, KimiSkillScope::BuiltIn));
    }
    roots
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum KimiSkillScope {
    Project,
    User,
    Extra,
    BuiltIn,
}

impl KimiSkillScope {
    fn heading(self) -> &'static str {
        match self {
            KimiSkillScope::Project => "Project",
            KimiSkillScope::User => "User",
            KimiSkillScope::Extra => "Extra",
            KimiSkillScope::BuiltIn => "Built-in",
        }
    }
}

struct KimiSkillRoot {
    path: PathBuf,
    scope: KimiSkillScope,
}

impl KimiSkillRoot {
    fn new(path: PathBuf, scope: KimiSkillScope) -> Self {
        Self { path, scope }
    }
}

fn builtin_kimi_skills_root(kimi_source_dir: Option<&Path>) -> Option<PathBuf> {
    let path = kimi_source_dir?.join("src/kimi_cli/skills");
    path.is_dir().then_some(path)
}

fn first_existing_dir<I>(candidates: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = PathBuf>,
{
    candidates.into_iter().find(|candidate| candidate.is_dir())
}

fn discover_skills_in_root(root: &Path, scope: KimiSkillScope) -> Vec<KimiSkill> {
    let mut skills = fs::read_dir(root)
        .ok()
        .into_iter()
        .flat_map(|iter| iter.filter_map(Result::ok))
        .filter_map(|entry| {
            let skill_md = entry.path().join("SKILL.md");
            parse_kimi_skill(&skill_md)
        })
        .map(|mut skill| {
            skill.scope = scope;
            skill
        })
        .collect::<Vec<_>>();
    skills.sort_by(|left, right| left.name.cmp(&right.name));
    skills
}

fn parse_kimi_skill(skill_md: &Path) -> Option<KimiSkill> {
    let text = fs::read_to_string(skill_md).ok()?;
    let (name, description) = parse_kimi_skill_frontmatter(text.as_str())?;
    Some(KimiSkill {
        name,
        description,
        path: skill_md.to_path_buf(),
        scope: KimiSkillScope::BuiltIn,
    })
}

fn parse_kimi_skill_frontmatter(text: &str) -> Option<(String, String)> {
    let frontmatter = text
        .strip_prefix("---\n")
        .and_then(|remaining| remaining.split_once("\n---\n"))
        .map(|(frontmatter, _)| frontmatter)?;
    let mut name = None;
    let mut description = None;
    let lines = frontmatter.lines().collect::<Vec<_>>();
    let mut index = 0;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if let Some(value) = trimmed.strip_prefix("name:") {
            name = Some(
                value
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string(),
            );
        } else if let Some(value) = trimmed.strip_prefix("description:") {
            let value = value.trim();
            if matches!(value, "|" | "|-" | "|+") {
                let mut block_lines = Vec::new();
                index += 1;
                while index < lines.len() {
                    let line = lines[index];
                    if !line.starts_with(' ') && !line.starts_with('\t') {
                        index -= 1;
                        break;
                    }
                    block_lines.push(line.trim().to_string());
                    index += 1;
                }
                description = Some(block_lines.join("\n"));
            } else {
                description = Some(value.trim_matches('"').trim_matches('\'').to_string());
            }
        }
        index += 1;
    }
    Some((name?, description?))
}

fn format_kimi_skills_for_prompt(skills: Vec<KimiSkill>) -> String {
    [
        KimiSkillScope::Project,
        KimiSkillScope::User,
        KimiSkillScope::Extra,
        KimiSkillScope::BuiltIn,
    ]
    .into_iter()
    .filter_map(|scope| {
        let lines = skills
            .iter()
            .filter(|skill| skill.scope == scope)
            .map(|skill| {
                format!(
                    "- {}\n  - Path: {}\n  - Description: {}",
                    skill.name,
                    skill.path.display(),
                    skill.description
                )
            })
            .collect::<Vec<_>>();
        (!lines.is_empty()).then(|| format!("### {}\n{}", scope.heading(), lines.join("\n")))
    })
    .collect::<Vec<_>>()
    .join("\n\n")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct KimiSkill {
    name: String,
    description: String,
    path: PathBuf,
    scope: KimiSkillScope,
}

fn render_conditional_block(
    mut template: String,
    start_marker: &str,
    end_marker: &str,
    include_block: bool,
) -> String {
    while let Some(start_index) = template.find(start_marker) {
        let block_with_start = &template[start_index + start_marker.len()..];
        let Some(end_offset) = block_with_start.find(end_marker) else {
            break;
        };
        let end_index = start_index + start_marker.len() + end_offset;
        let replacement = if include_block {
            block_with_start[..end_offset]
                .trim_matches('\n')
                .to_string()
        } else {
            String::new()
        };
        let mut replace_end = end_index + end_marker.len();
        if !include_block
            && template
                .as_bytes()
                .get(replace_end)
                .is_some_and(|byte| *byte == b'\n')
        {
            replace_end += 1;
        }
        template.replace_range(start_index..replace_end, replacement.as_str());
    }
    template
}

fn current_kimi_now() -> String {
    if let Ok(value) = std::env::var("OPEN_INTERPRETER_KIMI_NOW_OVERRIDE")
        && !value.trim().is_empty()
    {
        return value;
    }
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, false)
}

fn list_directory(work_dir: &Path) -> String {
    let (entries, total) = collect_entries(work_dir, KIMI_LIST_DIR_ROOT_WIDTH);
    let mut lines = Vec::new();
    let remaining = total.saturating_sub(entries.len());

    for (index, (name, is_dir)) in entries.iter().enumerate() {
        let is_last = index + 1 == entries.len() && remaining == 0;
        let connector = if is_last { "└── " } else { "├── " };
        if *is_dir {
            lines.push(format!("{connector}{name}/"));
            let child_prefix = if is_last { "    " } else { "│   " };
            let child_path = work_dir.join(name);
            let (children, child_total) = collect_entries(&child_path, KIMI_LIST_DIR_CHILD_WIDTH);
            let child_remaining = child_total.saturating_sub(children.len());
            for (child_index, (child_name, child_is_dir)) in children.iter().enumerate() {
                let child_is_last = child_index + 1 == children.len() && child_remaining == 0;
                let child_connector = if child_is_last {
                    "└── "
                } else {
                    "├── "
                };
                let suffix = if *child_is_dir { "/" } else { "" };
                lines.push(format!(
                    "{child_prefix}{child_connector}{child_name}{suffix}"
                ));
            }
            if child_remaining > 0 {
                lines.push(format!("{child_prefix}└── ... and {child_remaining} more"));
            }
        } else {
            lines.push(format!("{connector}{name}"));
        }
    }

    if remaining > 0 {
        lines.push(format!("└── ... and {remaining} more entries"));
    }

    if lines.is_empty() {
        "(empty directory)".to_string()
    } else {
        lines.join("\n")
    }
}

fn collect_entries(dir: &Path, max_width: usize) -> (Vec<(String, bool)>, usize) {
    let mut entries = fs::read_dir(dir)
        .ok()
        .into_iter()
        .flat_map(|iter| iter.filter_map(Result::ok))
        .map(|entry| {
            let path = entry.path();
            let is_dir = entry
                .file_type()
                .map(|file_type| file_type.is_dir())
                .unwrap_or(false);
            let name = entry.file_name().to_string_lossy().to_string();
            (name, is_dir, path)
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.1.cmp(&right.1).reverse().then(left.0.cmp(&right.0)));
    let total = entries.len();
    let collected = entries
        .into_iter()
        .take(max_width)
        .map(|(name, is_dir, _path)| (name, is_dir))
        .collect();
    (collected, total)
}

#[cfg(test)]
mod tests {
    use super::build_messages;
    use super::build_request;
    use super::build_system_prompt;
    use super::builtin_kimi_skill_listing;
    use super::discover_skills_in_root;
    use super::format_kimi_skills_for_prompt;
    use super::kimi_skill_roots;
    use super::load_kimi_agents_md;
    use super::parse_kimi_skill_frontmatter;
    use crate::client_common::Prompt;
    use codex_protocol::models::BaseInstructions;
    use codex_protocol::models::ContentItem;
    use codex_protocol::models::FunctionCallOutputContentItem;
    use codex_protocol::models::FunctionCallOutputPayload;
    use codex_protocol::models::ReasoningItemContent;
    use codex_protocol::models::ResponseItem;
    use codex_protocol::openai_models::ModelInfo;
    use codex_protocol::openai_models::ReasoningControl;
    use codex_protocol::openai_models::ReasoningEffort;
    use codex_tools::JsonSchema;
    use codex_tools::ResponsesApiTool;
    use codex_tools::ToolSpec;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::fs;

    #[test]
    fn kimi_user_messages_trim_trailing_newline() {
        let items = vec![ResponseItem::Message {
            id: Some("user".to_string()),
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "hello\n".to_string(),
            }],
            end_turn: None,
            phase: None,
        }];

        let messages = build_messages(&items)
            .expect("build messages")
            .collect::<Vec<_>>();

        assert_eq!(
            messages,
            vec![json!({
                "role": "user",
                "content": "hello",
            })]
        );
    }

    #[test]
    fn kimi_code_user_messages_preserve_trailing_newline() {
        let items = vec![ResponseItem::Message {
            id: Some("user".to_string()),
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "hello\n".to_string(),
            }],
            end_turn: None,
            phase: None,
        }];

        let messages =
            super::build_messages_with_options(&items, super::MessageBuildOptions::kimi_code())
                .expect("build messages")
                .collect::<Vec<_>>();

        assert_eq!(
            messages,
            vec![json!({
                "role": "user",
                "content": "hello\n",
            })]
        );
    }

    #[test]
    fn kimi_builtin_skills_are_rendered_when_source_tree_is_unavailable() {
        let temp = tempfile::tempdir().expect("tempdir");
        let found_skills = kimi_skill_roots(
            temp.path(),
            /*home_dir*/ None,
            /*kimi_source_dir*/ None,
        )
        .into_iter()
        .flat_map(|root| discover_skills_in_root(&root.path, root.scope))
        .any(|_| true);
        let listing = if found_skills {
            unreachable!("empty temp fixture should not discover skills")
        } else {
            builtin_kimi_skill_listing().to_string()
        };

        assert_eq!(
            listing,
            "### Built-in\n- kimi-cli-help\n  - Path: /tmp/kimi-cli/src/kimi_cli/skills/kimi-cli-help/SKILL.md\n  - Description: Answer Kimi Code CLI usage, configuration, and troubleshooting questions. Use when user asks about Kimi Code CLI installation, setup, configuration, slash commands, keyboard shortcuts, MCP integration, providers, environment variables, how something works internally, or any questions about Kimi Code CLI itself.\n- skill-creator\n  - Path: /tmp/kimi-cli/src/kimi_cli/skills/skill-creator/SKILL.md\n  - Description: Guide for creating effective skills. This skill should be used when users want to create a new skill (or update an existing skill) that extends Kimi's capabilities with specialized knowledge, workflows, or tool integrations."
        );
    }

    #[test]
    fn kimi_contextual_developer_messages_do_not_add_extra_user_messages() {
        let items = vec![
            ResponseItem::Message {
                id: Some("developer".to_string()),
                role: "developer".to_string(),
                content: vec![ContentItem::InputText {
                    text: "<skills_instructions>\n- imagegen\n</skills_instructions>".to_string(),
                }],
                end_turn: None,
                phase: None,
            },
            ResponseItem::Message {
                id: Some("user".to_string()),
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "$imagegen what is this".to_string(),
                }],
                end_turn: None,
                phase: None,
            },
        ];
        let prompt = Prompt {
            input: items.clone(),
            cwd: Some(std::env::temp_dir()),
            ..Prompt::default()
        };
        let system_prompt = build_system_prompt(&prompt, None, "conversation-id");

        let messages = build_messages(&items)
            .expect("build messages")
            .collect::<Vec<_>>();

        assert!(!system_prompt.contains("# Additional Developer Instructions"));
        assert!(!system_prompt.contains("<skills_instructions>"));
        assert_eq!(
            messages,
            vec![json!({
                "role": "user",
                "content": "$imagegen what is this",
            })]
        );
    }

    #[test]
    fn kimi_contextual_user_blocks_do_not_add_extra_user_messages() {
        let items = vec![
            ResponseItem::Message {
                id: Some("permissions".to_string()),
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "<permissions instructions>\nbody\n</permissions instructions>"
                        .to_string(),
                }],
                end_turn: None,
                phase: None,
            },
            ResponseItem::Message {
                id: Some("skills".to_string()),
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "<skills_instructions>\nbody\n</skills_instructions>".to_string(),
                }],
                end_turn: None,
                phase: None,
            },
            ResponseItem::Message {
                id: Some("user".to_string()),
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "do the task".to_string(),
                }],
                end_turn: None,
                phase: None,
            },
        ];

        let messages = build_messages(&items)
            .expect("build messages")
            .collect::<Vec<_>>();

        assert_eq!(
            messages,
            vec![json!({
                "role": "user",
                "content": "do the task",
            })]
        );
    }

    #[test]
    fn kimi_non_contextual_developer_messages_are_preserved_in_system_prompt() {
        let items = vec![ResponseItem::Message {
            id: Some("developer".to_string()),
            role: "developer".to_string(),
            content: vec![ContentItem::InputText {
                text: "Prefer small patches.".to_string(),
            }],
            end_turn: None,
            phase: None,
        }];
        let prompt = Prompt {
            input: items,
            cwd: Some(std::env::temp_dir()),
            ..Prompt::default()
        };

        let system_prompt = build_system_prompt(&prompt, None, "conversation-id");

        assert!(system_prompt.contains("# Additional Developer Instructions"));
        assert!(system_prompt.contains("Prefer small patches."));
    }

    #[test]
    fn kimi_system_prompt_omits_open_interpreter_extra_instruction() {
        let prompt = Prompt {
            base_instructions: BaseInstructions {
                text: "<extra_instruction>\nUse file tools first.\n</extra_instruction>\n\nCodex base prompt"
                    .to_string(),
            },
            cwd: Some(std::env::temp_dir()),
            ..Prompt::default()
        };

        let system_prompt = build_system_prompt(&prompt, None, "conversation-id");

        assert!(
            !system_prompt
                .contains("<extra_instruction>\nUse file tools first.\n</extra_instruction>")
        );
        assert!(!system_prompt.contains("Codex base prompt"));
    }

    #[test]
    fn kimi_agents_md_is_discovered_and_rendered_like_kimi_cli() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        fs::create_dir(root.join(".git")).expect("git dir");
        fs::write(root.join("AGENTS.md"), "root instructions\n").expect("root agents");
        let child = root.join("subdir");
        fs::create_dir_all(child.join(".kimi")).expect("child dir");
        fs::write(
            child.join(".kimi").join("AGENTS.md"),
            "child instructions\n",
        )
        .expect("child agents");

        let agents_md = load_kimi_agents_md(&child);

        assert_eq!(
            agents_md,
            format!(
                "<!-- From: {} -->\nroot instructions\n\n<!-- From: {} -->\nchild instructions",
                root.join("AGENTS.md").display(),
                child.join(".kimi").join("AGENTS.md").display()
            )
        );
    }

    #[test]
    fn kimi_request_omits_openai_specific_chat_fields_but_keeps_kimi_fields() {
        let prompt = Prompt {
            input: vec![ResponseItem::Message {
                id: Some("user".to_string()),
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "hello".to_string(),
                }],
                end_turn: None,
                phase: None,
            }],
            cwd: Some(std::env::temp_dir()),
            ..Prompt::default()
        };

        let (request, _) = build_request(
            &prompt,
            &test_model_info(),
            None,
            "conversation-id",
            None,
            /*yolo_mode*/ false,
        )
        .expect("build request");

        assert_eq!(
            request.get("prompt_cache_key"),
            Some(&json!("conversation-id"))
        );
        assert_eq!(request.get("tool_choice"), None);
        assert_eq!(request.get("parallel_tool_calls"), None);
        assert_eq!(request.get("store"), None);
    }

    #[test]
    fn kimi_request_keeps_reasoning_effort_even_without_catalog_reasoning_control() {
        let prompt = Prompt {
            input: vec![ResponseItem::Message {
                id: Some("user".to_string()),
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "think".to_string(),
                }],
                end_turn: None,
                phase: None,
            }],
            cwd: Some(std::env::temp_dir()),
            ..Prompt::default()
        };

        let (request, _) = build_request(
            &prompt,
            &test_model_info(),
            Some(ReasoningEffort::High),
            "conversation-id",
            None,
            /*yolo_mode*/ false,
        )
        .expect("build request");

        assert_eq!(request.get("reasoning_effort"), Some(&json!("high")));
        assert_eq!(request.get("thinking"), Some(&json!({ "type": "enabled" })));
    }

    #[test]
    fn kimi_request_maps_thinking_toggle_model_reasoning_effort_to_thinking() {
        let prompt = Prompt {
            input: vec![ResponseItem::Message {
                id: Some("user".to_string()),
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "think".to_string(),
                }],
                end_turn: None,
                phase: None,
            }],
            cwd: Some(std::env::temp_dir()),
            ..Prompt::default()
        };
        let mut model_info = test_model_info();
        model_info.reasoning_control = ReasoningControl::ThinkingToggle;

        let (request, _) = build_request(
            &prompt,
            &model_info,
            Some(ReasoningEffort::High),
            "conversation-id",
            None,
            /*yolo_mode*/ false,
        )
        .expect("build request");

        assert_eq!(request.get("reasoning_effort"), Some(&json!("high")));
        assert_eq!(request.get("thinking"), Some(&json!({ "type": "enabled" })));
    }

    #[test]
    fn kimi_yolo_mode_keeps_question_tool_without_prompt_reminder() {
        let ask_user_question = ResponsesApiTool {
            name: "AskUserQuestion".to_string(),
            description: "Ask the user a question.".to_string(),
            strict: false,
            defer_loading: None,
            parameters: JsonSchema::object(Vec::<(String, JsonSchema)>::new(), None, None),
            output_schema: None,
        };
        let shell = ResponsesApiTool {
            name: "Shell".to_string(),
            description: "Run a shell command.".to_string(),
            strict: false,
            defer_loading: None,
            parameters: JsonSchema::object(Vec::<(String, JsonSchema)>::new(), None, None),
            output_schema: None,
        };
        let prompt = Prompt {
            input: vec![ResponseItem::Message {
                id: Some("user".to_string()),
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "do the task".to_string(),
                }],
                end_turn: None,
                phase: None,
            }],
            tools: vec![
                ToolSpec::Function(ask_user_question),
                ToolSpec::Function(shell),
            ],
            cwd: Some(std::env::temp_dir()),
            ..Prompt::default()
        };

        let (request, _) = build_request(
            &prompt,
            &test_model_info(),
            None,
            "conversation-id",
            None,
            /*yolo_mode*/ true,
        )
        .expect("build request");

        let messages = request["messages"].as_array().expect("messages array");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert!(
            !messages[0]["content"]
                .as_str()
                .expect("system content")
                .contains("You are running in non-interactive mode")
        );
        assert_eq!(
            messages[1],
            json!({
                "role": "user",
                "content": "do the task",
            })
        );
        let tool_names = request["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .map(|tool| tool["function"]["name"].as_str().expect("tool name"))
            .collect::<Vec<_>>();
        assert_eq!(tool_names, vec!["AskUserQuestion", "Shell"]);

        let (interactive_request, _) = build_request(
            &prompt,
            &test_model_info(),
            None,
            "conversation-id",
            None,
            /*yolo_mode*/ false,
        )
        .expect("build request");
        let interactive_tool_names = interactive_request["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .map(|tool| tool["function"]["name"].as_str().expect("tool name"))
            .collect::<Vec<_>>();
        assert_eq!(interactive_tool_names, vec!["AskUserQuestion", "Shell"]);
    }

    #[test]
    fn kimi_messages_drop_unanswered_tool_call() {
        let items = vec![
            ResponseItem::FunctionCall {
                id: Some("fc-1".to_string()),
                name: "WriteFile".to_string(),
                namespace: None,
                arguments: r#"{"path":"/app/ars.R","content":"ok"}"#.to_string(),
                call_id: "WriteFile:6".to_string(),
            },
            ResponseItem::Message {
                id: Some("assistant".to_string()),
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "done".to_string(),
                }],
                end_turn: None,
                phase: None,
            },
        ];

        let messages = build_messages(&items)
            .expect("build messages")
            .collect::<Vec<_>>();

        assert_eq!(
            messages,
            vec![json!({
                "role": "assistant",
                "content": "done",
            }),]
        );
    }

    #[test]
    fn kimi_messages_ignore_empty_assistant_between_tool_call_and_output() {
        let items = vec![
            ResponseItem::FunctionCall {
                id: Some("fc-1".to_string()),
                name: "Shell".to_string(),
                namespace: None,
                arguments: r#"{"command":"which R && R --version"}"#.to_string(),
                call_id: "Shell:0".to_string(),
            },
            ResponseItem::Message {
                id: Some("chat-message-1".to_string()),
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: String::new(),
                }],
                end_turn: None,
                phase: None,
            },
            ResponseItem::FunctionCallOutput {
                call_id: "Shell:0".to_string(),
                output: FunctionCallOutputPayload::from_text(
                    "<system>Command executed successfully.</system>".to_string(),
                ),
            },
        ];

        let messages = build_messages(&items)
            .expect("build messages")
            .collect::<Vec<_>>();

        assert_eq!(
            messages,
            vec![
                json!({
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "type": "function",
                            "id": "Shell:0",
                            "function": {
                                "name": "Shell",
                                "arguments": r#"{"command":"which R && R --version"}"#,
                            },
                        }
                    ],
                }),
                json!({
                    "role": "tool",
                    "content": "<system>Command executed successfully.</system>",
                    "tool_call_id": "Shell:0",
                }),
            ]
        );
    }

    #[test]
    fn kimi_messages_attach_reasoning_content_to_tool_call_message() {
        let items = vec![
            ResponseItem::Reasoning {
                id: "rs-1".to_string(),
                summary: Vec::new(),
                content: Some(vec![ReasoningItemContent::ReasoningText {
                    text: "I need to inspect the files.".to_string(),
                }]),
                encrypted_content: None,
            },
            ResponseItem::FunctionCall {
                id: Some("fc-1".to_string()),
                name: "Shell".to_string(),
                namespace: None,
                arguments: r#"{"command":"ls"}"#.to_string(),
                call_id: "Shell:0".to_string(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: "Shell:0".to_string(),
                output: FunctionCallOutputPayload::from_text("ok".to_string()),
            },
        ];

        let messages = build_messages(&items)
            .expect("build messages")
            .collect::<Vec<_>>();

        assert_eq!(
            messages,
            vec![
                json!({
                    "role": "assistant",
                    "reasoning_content": "I need to inspect the files.",
                    "tool_calls": [
                        {
                            "type": "function",
                            "id": "Shell:0",
                            "function": {
                                "name": "Shell",
                                "arguments": r#"{"command":"ls"}"#,
                            },
                        }
                    ],
                }),
                json!({
                    "role": "tool",
                    "content": "ok",
                    "tool_call_id": "Shell:0",
                }),
            ]
        );
    }

    #[test]
    fn kimi_messages_merge_late_assistant_text_with_pending_tool_calls() {
        let items = vec![
            ResponseItem::Reasoning {
                id: "rs-1".to_string(),
                summary: Vec::new(),
                content: Some(vec![ReasoningItemContent::ReasoningText {
                    text: "I should inspect the runtime.".to_string(),
                }]),
                encrypted_content: None,
            },
            ResponseItem::FunctionCall {
                id: Some("fc-1".to_string()),
                name: "Shell".to_string(),
                namespace: None,
                arguments: r#"{"command":"which R && R --version"}"#.to_string(),
                call_id: "Shell:1".to_string(),
            },
            ResponseItem::Message {
                id: Some("msg-1".to_string()),
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "I'll check whether R is available.".to_string(),
                }],
                end_turn: None,
                phase: None,
            },
            ResponseItem::FunctionCallOutput {
                call_id: "Shell:1".to_string(),
                output: FunctionCallOutputPayload::from_text(
                    "<system>ERROR: Command failed with exit code: 1.</system>".to_string(),
                ),
            },
        ];

        let messages = build_messages(&items)
            .expect("build messages")
            .collect::<Vec<_>>();

        assert_eq!(
            messages,
            vec![
                json!({
                    "role": "assistant",
                    "content": "I'll check whether R is available.",
                    "reasoning_content": "I should inspect the runtime.",
                    "tool_calls": [
                        {
                            "type": "function",
                            "id": "Shell:1",
                            "function": {
                                "name": "Shell",
                                "arguments": r#"{"command":"which R && R --version"}"#,
                            },
                        }
                    ],
                }),
                json!({
                    "role": "tool",
                    "content": "<system>ERROR: Command failed with exit code: 1.</system>",
                    "tool_call_id": "Shell:1",
                }),
            ]
        );
    }

    #[test]
    fn kimi_messages_replace_non_text_only_tool_output() {
        let items = vec![
            ResponseItem::FunctionCall {
                id: Some("fc-1".to_string()),
                name: "Shell".to_string(),
                namespace: None,
                arguments: r#"{"command":"./a.out"}"#.to_string(),
                call_id: "Shell:0".to_string(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: "Shell:0".to_string(),
                output: FunctionCallOutputPayload::from_content_items(vec![
                    FunctionCallOutputContentItem::InputText {
                        text: "<system>Command executed successfully.</system>".to_string(),
                    },
                    FunctionCallOutputContentItem::InputText {
                        text: "\0\0\0".to_string(),
                    },
                ]),
            },
        ];

        let messages = build_messages(&items)
            .expect("build messages")
            .collect::<Vec<_>>();

        assert_eq!(
            messages,
            vec![
                json!({
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "type": "function",
                            "id": "Shell:0",
                            "function": {
                                "name": "Shell",
                                "arguments": r#"{"command":"./a.out"}"#,
                            },
                        }
                    ],
                }),
                json!({
                    "role": "tool",
                    "content": [
                        {
                            "type": "text",
                            "text": "<system>Command executed successfully.</system>",
                        },
                        {
                            "type": "text",
                            "text": "<system>Tool returned non-text content.</system>",
                        },
                    ],
                    "tool_call_id": "Shell:0",
                }),
            ]
        );
    }

    #[test]
    fn kimi_messages_preserve_control_bytes_from_tool_output() {
        let items = vec![
            ResponseItem::FunctionCall {
                id: Some("fc-1".to_string()),
                name: "Shell".to_string(),
                namespace: None,
                arguments: r#"{"command":"printf"}"#.to_string(),
                call_id: "Shell:0".to_string(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: "Shell:0".to_string(),
                output: FunctionCallOutputPayload::from_text("a\u{c}b\nc".to_string()),
            },
        ];

        let messages = build_messages(&items)
            .expect("build messages")
            .collect::<Vec<_>>();

        assert_eq!(
            messages[1],
            json!({
                "role": "tool",
                "content": "a\u{c}b\nc",
                "tool_call_id": "Shell:0",
            })
        );
    }

    #[test]
    fn kimi_messages_keep_actual_tool_output_after_call() {
        let items = vec![
            ResponseItem::FunctionCall {
                id: Some("fc-1".to_string()),
                name: "WriteFile".to_string(),
                namespace: None,
                arguments: r#"{"path":"/app/ars.R","content":"ok"}"#.to_string(),
                call_id: "WriteFile:6".to_string(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: "WriteFile:6".to_string(),
                output: FunctionCallOutputPayload::from_text("written".to_string()),
            },
        ];

        let messages = build_messages(&items)
            .expect("build messages")
            .collect::<Vec<_>>();

        assert_eq!(
            messages,
            vec![
                json!({
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "type": "function",
                            "id": "WriteFile:6",
                            "function": {
                                "name": "WriteFile",
                                "arguments": r#"{"path":"/app/ars.R","content":"ok"}"#,
                            },
                        }
                    ],
                }),
                json!({
                    "role": "tool",
                    "content": "written",
                    "tool_call_id": "WriteFile:6",
                }),
            ]
        );
    }

    #[test]
    fn kimi_messages_preserve_kimi_style_failed_tool_content_parts() {
        let items = vec![
            ResponseItem::FunctionCall {
                id: Some("fc-1".to_string()),
                name: "Shell".to_string(),
                namespace: None,
                arguments: r#"{"command":"false"}"#.to_string(),
                call_id: "Shell:7".to_string(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: "Shell:7".to_string(),
                output: FunctionCallOutputPayload {
                    body: codex_protocol::models::FunctionCallOutputBody::ContentItems(vec![
                        FunctionCallOutputContentItem::InputText {
                            text: "<system>ERROR: Command failed with exit code: 1.</system>"
                                .to_string(),
                        },
                        FunctionCallOutputContentItem::InputText {
                            text: "stderr text".to_string(),
                        },
                    ]),
                    success: Some(false),
                },
            },
        ];

        let messages = build_messages(&items)
            .expect("build messages")
            .collect::<Vec<_>>();

        assert_eq!(
            messages,
            vec![
                json!({
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "type": "function",
                            "id": "Shell:7",
                            "function": {
                                "name": "Shell",
                                "arguments": r#"{"command":"false"}"#,
                            },
                        }
                    ],
                }),
                json!({
                    "role": "tool",
                    "content": [
                        {
                            "type": "text",
                            "text": "<system>ERROR: Command failed with exit code: 1.</system>",
                        },
                        {
                            "type": "text",
                            "text": "stderr text",
                        },
                    ],
                    "tool_call_id": "Shell:7",
                }),
            ]
        );
    }

    #[test]
    fn kimi_messages_preserve_single_kimi_style_failed_tool_part() {
        let items = vec![
            ResponseItem::FunctionCall {
                id: Some("fc-1".to_string()),
                name: "Shell".to_string(),
                namespace: None,
                arguments: r#"{"command":"which R && R --version"}"#.to_string(),
                call_id: "Shell:7".to_string(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: "Shell:7".to_string(),
                output: FunctionCallOutputPayload {
                    body: codex_protocol::models::FunctionCallOutputBody::Text(
                        "<system>ERROR: Command failed with exit code: 1.</system>".to_string(),
                    ),
                    success: Some(false),
                },
            },
        ];

        let messages = build_messages(&items)
            .expect("build messages")
            .collect::<Vec<_>>();

        assert_eq!(
            messages,
            vec![
                json!({
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "type": "function",
                            "id": "Shell:7",
                            "function": {
                                "name": "Shell",
                                "arguments": r#"{"command":"which R && R --version"}"#,
                            },
                        }
                    ],
                }),
                json!({
                    "role": "tool",
                    "content": "<system>ERROR: Command failed with exit code: 1.</system>",
                    "tool_call_id": "Shell:7",
                }),
            ]
        );
    }

    #[test]
    fn kimi_messages_skip_orphaned_tool_output() {
        let items = vec![
            ResponseItem::FunctionCallOutput {
                call_id: "WriteFile:6".to_string(),
                output: FunctionCallOutputPayload::from_text("written".to_string()),
            },
            ResponseItem::Message {
                id: Some("assistant".to_string()),
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "done".to_string(),
                }],
                end_turn: None,
                phase: None,
            },
        ];

        let messages = build_messages(&items)
            .expect("build messages")
            .collect::<Vec<_>>();

        assert_eq!(
            messages,
            vec![json!({
                "role": "assistant",
                "content": "done",
            })]
        );
    }

    #[test]
    fn kimi_skill_parser_reads_frontmatter_name_and_description() {
        let text = r#"---
name: demo-guide
description: Read the demo creation workflow guide
---

Body
"#;

        assert_eq!(
            parse_kimi_skill_frontmatter(text),
            Some((
                "demo-guide".to_string(),
                "Read the demo creation workflow guide".to_string(),
            ))
        );
    }

    #[test]
    fn kimi_skill_parser_reads_block_description() {
        let text = r#"---
name: apply-to-ramp
description: |-
  Guide a user through submitting a Ramp financing application via the CLI.
  Use when: "apply to ramp", "create application", "sign up for ramp",
  "submit a ramp application", "onboard a new business".
---

Body
"#;

        assert_eq!(
            parse_kimi_skill_frontmatter(text),
            Some((
                "apply-to-ramp".to_string(),
                "Guide a user through submitting a Ramp financing application via the CLI.\nUse when: \"apply to ramp\", \"create application\", \"sign up for ramp\",\n\"submit a ramp application\", \"onboard a new business\".".to_string(),
            ))
        );
    }

    #[test]
    fn kimi_skill_discovery_uses_brand_then_generic_roots() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let work_dir = temp.path().join("workspace");
        fs::create_dir_all(home.join(".claude/skills/demo-guide")).expect("claude skills dir");
        fs::create_dir_all(home.join(".agents/skills/generic-guide")).expect("generic skills dir");
        fs::create_dir_all(&work_dir).expect("workspace dir");
        fs::write(
            home.join(".claude/skills/demo-guide/SKILL.md"),
            "---\nname: demo-guide\ndescription: Read the demo creation workflow guide\n---\n",
        )
        .expect("write claude skill");
        fs::write(
            home.join(".agents/skills/generic-guide/SKILL.md"),
            "---\nname: generic-guide\ndescription: Read the generic workflow guide\n---\n",
        )
        .expect("write generic skill");

        let skills = kimi_skill_roots(&work_dir, Some(home.clone()), None)
            .into_iter()
            .flat_map(|root| discover_skills_in_root(&root.path, root.scope))
            .collect::<Vec<_>>();
        let skills = format_kimi_skills_for_prompt(skills);

        assert_eq!(
            skills,
            format!(
                "### User\n- demo-guide\n  - Path: {}\n  - Description: Read the demo creation workflow guide\n- generic-guide\n  - Path: {}\n  - Description: Read the generic workflow guide",
                home.join(".claude/skills/demo-guide/SKILL.md").display(),
                home.join(".agents/skills/generic-guide/SKILL.md").display(),
            )
        );
    }

    fn test_model_info() -> ModelInfo {
        serde_json::from_value(json!({
            "slug": "kimi-k2.5",
            "display_name": "Kimi K2.5",
            "description": null,
            "supported_reasoning_levels": [],
            "shell_type": "shell_command",
            "visibility": "list",
            "supported_in_api": true,
            "priority": 1,
            "availability_nux": null,
            "upgrade": null,
            "base_instructions": "base",
            "model_messages": null,
            "supports_reasoning_summaries": false,
            "default_reasoning_summary": "auto",
            "support_verbosity": false,
            "default_verbosity": null,
            "apply_patch_tool_type": "freeform",
            "truncation_policy": {
                "mode": "bytes",
                "limit": 10000
            },
            "supports_parallel_tool_calls": false,
            "supports_image_detail_original": false,
            "context_window": null,
            "auto_compact_token_limit": null,
            "effective_context_window_percent": 95,
            "experimental_supported_tools": [],
            "input_modalities": ["text", "image"],
            "supports_search_tool": false
        }))
        .expect("deserialize test model")
    }
}
