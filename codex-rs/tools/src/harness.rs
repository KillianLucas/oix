#[derive(Debug, Clone, Eq, PartialEq, Default)]
pub enum Harness {
    #[default]
    Native,
    ClaudeCode,
    Other(String),
}

impl Harness {
    pub fn from_config_name(name: Option<&str>) -> Self {
        match name {
            None | Some("") => Self::Native,
            Some("claude-code") => Self::ClaudeCode,
            Some(other) => Self::Other(other.to_string()),
        }
    }

    pub fn is_claude_code(&self) -> bool {
        matches!(self, Self::ClaudeCode)
    }
}
