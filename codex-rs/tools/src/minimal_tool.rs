use crate::JsonSchema;
use crate::ResponsesApiTool;
use crate::ToolSpec;
use serde_json::json;
use std::collections::BTreeMap;

pub fn create_minimal_bash_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "bash".to_string(),
        description: "Run a bash command in a persistent shell. State (cwd, env, background processes) persists across calls. Starts in the user's working directory."
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            BTreeMap::from([
                (
                    "command".to_string(),
                    JsonSchema::string(/*description*/ None),
                ),
                (
                    "timeout".to_string(),
                    JsonSchema::integer(Some("Timeout in seconds. Defaults to 120.".to_string())),
                ),
            ]),
            Some(vec!["command".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

pub fn create_mini_swe_agent_bash_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "bash".to_string(),
        description: "Execute a bash command".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            BTreeMap::from([(
                "command".to_string(),
                JsonSchema::string(Some("The bash command to execute".to_string())),
            )]),
            Some(vec!["command".to_string()]),
            None,
        ),
        output_schema: None,
    })
}

pub fn create_minimal_str_replace_editor_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "str_replace_editor".to_string(),
        description: "View and edit files. All paths must be absolute.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            BTreeMap::from([
                (
                    "command".to_string(),
                    JsonSchema::string_enum(
                        vec![
                            json!("view"),
                            json!("create"),
                            json!("str_replace"),
                            json!("insert"),
                            json!("undo_edit"),
                        ],
                        /*description*/ None,
                    ),
                ),
                ("path".to_string(), JsonSchema::string(/*description*/ None)),
                (
                    "view_range".to_string(),
                    JsonSchema::array(JsonSchema::integer(/*description*/ None), None),
                ),
                (
                    "file_text".to_string(),
                    JsonSchema::string(/*description*/ None),
                ),
                (
                    "old_str".to_string(),
                    JsonSchema::string(/*description*/ None),
                ),
                (
                    "new_str".to_string(),
                    JsonSchema::string(/*description*/ None),
                ),
                (
                    "insert_line".to_string(),
                    JsonSchema::integer(/*description*/ None),
                ),
            ]),
            Some(vec!["command".to_string(), "path".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}
