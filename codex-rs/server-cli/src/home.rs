use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use crate::startup_trace::record_startup_trace_event;

pub const INTERPRETER_HOME_ENV_VAR: &str = "INTERPRETER_HOME";
pub const OPEN_INTERPRETER_HOME_ENV_VAR: &str = "OPEN_INTERPRETER_HOME";
pub const INTERPRETER_DISABLE_SYSTEM_IMPORT_ENV_VAR: &str = "INTERPRETER_DISABLE_SYSTEM_IMPORT";
pub const INTERPRETER_FORCE_PROVIDER_ONBOARDING_ENV_VAR: &str =
    "INTERPRETER_FORCE_PROVIDER_ONBOARDING";
pub const FRESH_HOME_PROVIDER_ONBOARDING_MARKER_FILE: &str = ".fresh_home_provider_onboarding";
const CODEX_HOME_ENV_VAR: &str = "CODEX_HOME";
const CODEX_AUTH_HOME_ENV_VAR: &str = "CODEX_AUTH_HOME";
const OPEN_INTERPRETER_BRAND_ENV_VAR: &str = "OPEN_INTERPRETER_BRAND";
const DEFAULT_OPEN_INTERPRETER_HOME_DIR: &str = ".openinterpreter";
const DEFAULT_CODEX_HOME_DIR: &str = ".codex";
const CONFIG_TOML_FILE: &str = "config.toml";
const AUTH_JSON_FILE: &str = "auth.json";

pub fn ensure_interpreter_home_env() -> io::Result<PathBuf> {
    let resolved = current_interpreter_home()?;
    std::fs::create_dir_all(&resolved)?;
    let fresh_home_provider_onboarding_marker =
        resolved.join(FRESH_HOME_PROVIDER_ONBOARDING_MARKER_FILE);
    let force_provider_onboarding = std::env::var_os(INTERPRETER_FORCE_PROVIDER_ONBOARDING_ENV_VAR)
        .is_some_and(|value| !value.is_empty())
        || fresh_home_provider_onboarding_marker.exists()
        || (!resolved.join(CONFIG_TOML_FILE).exists() && !resolved.join(AUTH_JSON_FILE).exists());
    record_startup_trace_event(if force_provider_onboarding {
        "interpreter.home.force_provider_onboarding.true"
    } else {
        "interpreter.home.force_provider_onboarding.false"
    });
    let canonical = resolved.canonicalize()?;
    let auth_home = resolve_interpreter_auth_home_from_env(
        &canonical,
        std::env::var_os(CODEX_AUTH_HOME_ENV_VAR).as_deref(),
        fallback_home_directory(),
    )?;
    // Keep interpreter state isolated while sharing Codex's mutable ChatGPT auth file.
    let canonical_auth_home = auth_home.canonicalize().unwrap_or(auth_home);
    if std::env::var_os(INTERPRETER_DISABLE_SYSTEM_IMPORT_ENV_VAR)
        .is_none_or(|value| value.is_empty())
    {
        crate::system_import::import_system_state(&canonical)?;
    }
    if force_provider_onboarding {
        let _ = std::fs::write(
            canonical.join(FRESH_HOME_PROVIDER_ONBOARDING_MARKER_FILE),
            "pending\n",
        );
    }
    // SAFETY: main() calls this before the tokio runtime starts any background
    // threads, so mutating the process environment here is safe.
    unsafe {
        std::env::set_var(CODEX_HOME_ENV_VAR, &canonical);
        std::env::set_var(CODEX_AUTH_HOME_ENV_VAR, &canonical_auth_home);
        std::env::set_var(INTERPRETER_HOME_ENV_VAR, &canonical);
        std::env::set_var(OPEN_INTERPRETER_HOME_ENV_VAR, &canonical);
        std::env::set_var(OPEN_INTERPRETER_BRAND_ENV_VAR, "1");
        if force_provider_onboarding {
            std::env::set_var(INTERPRETER_FORCE_PROVIDER_ONBOARDING_ENV_VAR, "1");
        } else {
            std::env::remove_var(INTERPRETER_FORCE_PROVIDER_ONBOARDING_ENV_VAR);
        }
    }
    Ok(canonical)
}

pub fn current_interpreter_home() -> io::Result<PathBuf> {
    resolve_interpreter_home_from_env(
        std::env::var_os(INTERPRETER_HOME_ENV_VAR).as_deref(),
        std::env::var_os(OPEN_INTERPRETER_HOME_ENV_VAR).as_deref(),
        fallback_home_directory(),
    )
}

fn resolve_interpreter_home_from_env(
    interpreter_home: Option<&OsStr>,
    open_interpreter_home: Option<&OsStr>,
    fallback_home_dir: Option<PathBuf>,
) -> io::Result<PathBuf> {
    if let Some(path) = non_empty_path(interpreter_home) {
        return Ok(path);
    }

    if let Some(path) = non_empty_path(open_interpreter_home) {
        return Ok(path);
    }

    let Some(home_dir) = fallback_home_dir else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Could not find a home directory for Open Interpreter",
        ));
    };

    Ok(home_dir.join(DEFAULT_OPEN_INTERPRETER_HOME_DIR))
}

fn resolve_interpreter_auth_home_from_env(
    interpreter_home: &Path,
    codex_auth_home: Option<&OsStr>,
    fallback_home_dir: Option<PathBuf>,
) -> io::Result<PathBuf> {
    if let Some(path) = non_empty_path(codex_auth_home) {
        return Ok(path);
    }

    let Some(home_dir) = fallback_home_dir else {
        return Ok(interpreter_home.to_path_buf());
    };
    let default_codex_home = home_dir.join(DEFAULT_CODEX_HOME_DIR);
    if interpreter_home == default_codex_home {
        return Ok(interpreter_home.to_path_buf());
    }
    // ChatGPT refresh tokens are single-use, so the interpreter must not copy them.
    if auth_file_is_chatgpt(&default_codex_home.join(AUTH_JSON_FILE))? {
        return Ok(default_codex_home);
    }
    Ok(interpreter_home.to_path_buf())
}

fn non_empty_path(value: Option<&OsStr>) -> Option<PathBuf> {
    value.filter(|value| !value.is_empty()).map(PathBuf::from)
}

fn fallback_home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
}

fn auth_file_is_chatgpt(path: &Path) -> io::Result<bool> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err),
    };
    let auth = match serde_json::from_str::<serde_json::Value>(&contents) {
        Ok(auth) => auth,
        Err(_) => return Ok(false),
    };
    let has_tokens = auth.get("tokens").is_some_and(serde_json::Value::is_object);
    let has_api_key = auth
        .get("openai_api_key")
        .or_else(|| auth.get("OPENAI_API_KEY"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| !value.trim().is_empty());
    let auth_mode = auth.get("auth_mode").and_then(serde_json::Value::as_str);
    Ok(auth_mode == Some("chatgpt") || (auth_mode.is_none() && has_tokens && !has_api_key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn resolve_prefers_interpreter_home() {
        let resolved = resolve_interpreter_home_from_env(
            Some(OsStr::new("/tmp/interpreter-home")),
            Some(OsStr::new("/tmp/open-interpreter-home")),
            Some(PathBuf::from("/Users/test")),
        )
        .expect("resolve INTERPRETER_HOME");

        assert_eq!(resolved, PathBuf::from("/tmp/interpreter-home"));
    }

    #[test]
    fn resolve_falls_back_to_open_interpreter_home() {
        let resolved = resolve_interpreter_home_from_env(
            /*interpreter_home*/ None,
            Some(OsStr::new("/tmp/open-interpreter-home")),
            Some(PathBuf::from("/Users/test")),
        )
        .expect("resolve OPEN_INTERPRETER_HOME");

        assert_eq!(resolved, PathBuf::from("/tmp/open-interpreter-home"));
    }

    #[test]
    fn resolve_defaults_to_dot_openinterpreter() {
        let resolved = resolve_interpreter_home_from_env(
            /*interpreter_home*/ None,
            /*open_interpreter_home*/ None,
            Some(PathBuf::from("/Users/test")),
        )
        .expect("resolve default home");

        assert_eq!(resolved, PathBuf::from("/Users/test/.openinterpreter"));
    }

    #[test]
    fn auth_home_prefers_explicit_env() {
        let resolved = resolve_interpreter_auth_home_from_env(
            Path::new("/Users/test/.openinterpreter"),
            Some(OsStr::new("/tmp/auth-home")),
            Some(PathBuf::from("/Users/test")),
        )
        .expect("resolve auth home");

        assert_eq!(resolved, PathBuf::from("/tmp/auth-home"));
    }

    #[test]
    fn auth_home_uses_default_codex_home_for_chatgpt_auth() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let interpreter_home = temp.path().join(".openinterpreter");
        let codex_home = temp.path().join(".codex");
        std::fs::create_dir_all(&codex_home).expect("create codex home");
        std::fs::write(
            codex_home.join(AUTH_JSON_FILE),
            r#"{"auth_mode":"chatgpt","tokens":{"access_token":"access"}}"#,
        )
        .expect("write codex auth");

        let resolved = resolve_interpreter_auth_home_from_env(
            &interpreter_home,
            None,
            Some(temp.path().to_path_buf()),
        )
        .expect("resolve auth home");

        assert_eq!(resolved, codex_home);
    }

    #[test]
    fn auth_home_uses_interpreter_home_without_default_chatgpt_auth() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let interpreter_home = temp.path().join(".openinterpreter");
        let codex_home = temp.path().join(".codex");
        std::fs::create_dir_all(&codex_home).expect("create codex home");
        std::fs::write(
            codex_home.join(AUTH_JSON_FILE),
            r#"{"auth_mode":"apikey","OPENAI_API_KEY":"secret"}"#,
        )
        .expect("write codex auth");

        let resolved = resolve_interpreter_auth_home_from_env(
            &interpreter_home,
            None,
            Some(temp.path().to_path_buf()),
        )
        .expect("resolve auth home");

        assert_eq!(resolved, interpreter_home);
    }

    #[test]
    fn resolve_rejects_missing_home_directory() {
        let err = resolve_interpreter_home_from_env(
            /*interpreter_home*/ None, /*open_interpreter_home*/ None,
            /*fallback_home_dir*/ None,
        )
        .expect_err("missing home dir");

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(err.to_string().contains("Open Interpreter"));
    }
}
