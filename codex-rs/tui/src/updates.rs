#![cfg(all(feature = "startup-network", any(test, not(debug_assertions))))]
// The spawn/marker/lock helpers are only wired up in release builds (callers are
// gated on `not(debug_assertions)`), but `cfg(test)` still compiles this module to
// exercise the version-check logic. Allow the unused production-only items.
#![cfg_attr(test, allow(dead_code))]

use crate::legacy_core::config::Config;
use crate::update_action::UpdateAction;
use chrono::DateTime;
use chrono::Duration;
use chrono::Utc;
use codex_login::default_client::create_client;
use serde::Deserialize;
use serde::Serialize;
use std::fs::File;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;

use crate::version::CODEX_CLI_VERSION;

pub fn get_upgrade_version(config: &Config) -> Option<String> {
    if !config.check_for_update_on_startup {
        return None;
    }

    let version_file = version_filepath(config);
    let info = read_version_info(&version_file).ok();

    if should_check_for_update(info.as_ref(), Utc::now()) {
        // Refresh the cached latest version in the background so TUI startup
        // isn't blocked by a network call.
        tokio::spawn(async move {
            check_for_update(&version_file, RELEASES_API_URL)
                .await
                .inspect_err(|e| tracing::error!("Failed to update version: {e}"))
        });
    }

    info.and_then(|info| {
        if is_newer(&info.latest_version, CODEX_CLI_VERSION).unwrap_or(false) {
            Some(info.latest_version)
        } else {
            None
        }
    })
}

// UX: when we think we're current, re-check every startup. The 1h throttle would
// otherwise hide a release published just after our last check; a pending upgrade
// installs next launch regardless, so keep throttling that case.
fn should_check_for_update(info: Option<&VersionInfo>, now: DateTime<Utc>) -> bool {
    match info {
        None => true,
        Some(info) => {
            let pending = is_newer(&info.latest_version, CODEX_CLI_VERSION).unwrap_or(false);
            !pending || info.last_checked_at < now - Duration::hours(1)
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct VersionInfo {
    latest_version: String,
    // ISO-8601 timestamp (RFC3339)
    last_checked_at: DateTime<Utc>,
}

const VERSION_FILENAME: &str = "version.json";
const AUTO_UPDATE_MARKER_FILENAME: &str = "update-installed.json";
const AUTO_UPDATE_LOCK_FILENAME: &str = "update-running.lock";
const RELEASES_API_URL: &str = "https://api.github.com/repos/KillianLucas/oix/releases";

#[derive(Deserialize, Debug, Clone)]
struct ReleaseInfo {
    tag_name: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct InstalledUpdateInfo {
    version: String,
}

fn version_filepath(config: &Config) -> PathBuf {
    config.codex_home.join(VERSION_FILENAME).into_path_buf()
}

fn read_version_info(version_file: &Path) -> anyhow::Result<VersionInfo> {
    let contents = std::fs::read_to_string(version_file)?;
    Ok(serde_json::from_str(&contents)?)
}

async fn check_for_update(version_file: &Path, releases_api_url: &str) -> anyhow::Result<()> {
    let latest_tag_name = latest_release_tag_name(releases_api_url).await?;
    let latest_version = extract_version_from_latest_tag(&latest_tag_name)?;

    let info = VersionInfo {
        latest_version,
        last_checked_at: Utc::now(),
    };

    let json_line = format!("{}\n", serde_json::to_string(&info)?);
    if let Some(parent) = version_file.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(version_file, json_line).await?;
    Ok(())
}

async fn latest_release_tag_name(releases_api_url: &str) -> anyhow::Result<String> {
    let client = create_client();
    let latest_url = format!("{releases_api_url}/latest");
    let latest_response = client.get(&latest_url).send().await?;
    if latest_response.status().as_u16() != 404 {
        let ReleaseInfo { tag_name } = latest_response
            .error_for_status()?
            .json::<ReleaseInfo>()
            .await?;
        return Ok(tag_name);
    }

    // GitHub's /latest endpoint excludes prereleases. During early 0.x release
    // testing, fall back to the release list so self-update still has a channel.
    let releases = client
        .get(releases_api_url)
        .send()
        .await?
        .error_for_status()?
        .json::<Vec<ReleaseInfo>>()
        .await?;
    releases
        .into_iter()
        .map(|release| release.tag_name)
        .next()
        .ok_or_else(|| anyhow::anyhow!("No Open Interpreter releases found"))
}

fn is_newer(latest: &str, current: &str) -> Option<bool> {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => Some(l > c),
        _ => None,
    }
}

fn extract_version_from_latest_tag(latest_tag_name: &str) -> anyhow::Result<String> {
    latest_tag_name
        .strip_prefix('v')
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("Failed to parse latest tag name '{latest_tag_name}'"))
}

pub fn spawn_auto_update_if_needed(config: &Config) {
    if !config.check_for_update_on_startup {
        return;
    }
    let Some(update_action) = crate::update_action::get_update_action() else {
        return;
    };
    let marker_file = update_marker_filepath(config);
    let Some(latest_version) = get_upgrade_version(config) else {
        return;
    };
    let lock_file = update_lock_filepath(config);
    let Some(lock_guard) = try_acquire_update_lock(&lock_file) else {
        return;
    };
    spawn_update_command(update_action, latest_version, marker_file, Some(lock_guard));
}

pub fn spawn_manual_update(config: &Config) -> anyhow::Result<()> {
    let Some(update_action) = crate::update_action::get_update_action() else {
        anyhow::bail!(
            "This installation cannot self-update. Install with the standalone installer to enable updates."
        );
    };
    let marker_file = update_marker_filepath(config);
    let lock_file = update_lock_filepath(config);
    let Some(lock_guard) = try_acquire_update_lock(&lock_file) else {
        anyhow::bail!("An Open Interpreter update is already running.");
    };
    spawn_update_command(
        update_action,
        "latest".to_string(),
        marker_file,
        Some(lock_guard),
    );
    Ok(())
}

pub fn take_installed_update_notice(config: &Config) -> Option<String> {
    let marker_file = update_marker_filepath(config);
    let contents = std::fs::read_to_string(&marker_file).ok()?;
    let _ = std::fs::remove_file(&marker_file);
    let info: InstalledUpdateInfo = serde_json::from_str(&contents).ok()?;
    Some(format!("Updated to Open Interpreter {}.", info.version))
}

fn update_marker_filepath(config: &Config) -> PathBuf {
    config
        .codex_home
        .join(AUTO_UPDATE_MARKER_FILENAME)
        .into_path_buf()
}

fn update_lock_filepath(config: &Config) -> PathBuf {
    config
        .codex_home
        .join(AUTO_UPDATE_LOCK_FILENAME)
        .into_path_buf()
}

/// Acquire the single-flight update lock as an OS advisory lock.
///
/// The kernel releases an advisory lock when the returned `File` is dropped or
/// the process exits, so a crash, SIGKILL, or quit mid-update cannot leave a
/// stale lock that permanently disables updates. The file is only the lock
/// anchor; its existence no longer gates anything.
fn try_acquire_update_lock(lock_file: &Path) -> Option<File> {
    if let Some(parent) = lock_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_file)
        .ok()?;
    // `Ok` means we hold it. On any error (e.g. `WouldBlock` when another
    // interpreter is updating) we decline rather than risk a concurrent update.
    match file.try_lock() {
        Ok(()) => Some(file),
        Err(_) => None,
    }
}

fn spawn_update_command(
    update_action: UpdateAction,
    version: String,
    marker_file: PathBuf,
    lock_guard: Option<File>,
) {
    let marker_parent = marker_file.parent().map(Path::to_path_buf);
    std::thread::spawn(move || {
        // Keep the advisory lock held for the update's lifetime; thread return or
        // process exit drops the guard and releases it.
        let _lock_guard = lock_guard;
        let (command, args) = update_action.command_args();
        let command_status = std::process::Command::new(command)
            .args(args)
            // Re-running the installer must never block on a TTY prompt: it reads
            // /dev/tty directly even when stdio is null, which would otherwise
            // render onto the live terminal and park this thread on a tty read.
            .env("OPEN_INTERPRETER_NONINTERACTIVE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        match command_status {
            Ok(status) if status.success() => {
                if let Some(parent) = marker_parent {
                    let _ = std::fs::create_dir_all(parent);
                }
                let marker = InstalledUpdateInfo { version };
                if let Ok(json_line) =
                    serde_json::to_string(&marker).map(|line| format!("{line}\n"))
                {
                    let _ = std::fs::write(marker_file, json_line);
                }
            }
            Ok(status) => {
                tracing::warn!("Open Interpreter update command exited with status {status}");
            }
            Err(err) => {
                tracing::warn!("Failed to start Open Interpreter update command: {err}");
            }
        }
    });
}

fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut iter = v.trim().split('.');
    let maj = iter.next()?.parse::<u64>().ok()?;
    let min = iter.next()?.parse::<u64>().ok()?;
    let pat = iter.next()?.parse::<u64>().ok()?;
    Some((maj, min, pat))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_version_from_open_interpreter_latest_tag() {
        assert_eq!(
            extract_version_from_latest_tag("v1.5.0").expect("failed to parse version"),
            "1.5.0"
        );
    }

    #[test]
    fn latest_tag_without_known_prefix_is_invalid() {
        assert!(extract_version_from_latest_tag("1.5.0").is_err());
    }

    #[test]
    fn prerelease_version_is_not_considered_newer() {
        assert_eq!(is_newer("0.11.0-beta.1", "0.11.0"), None);
        assert_eq!(is_newer("1.0.0-rc.1", "1.0.0"), None);
    }

    #[test]
    fn plain_semver_comparisons_work() {
        assert_eq!(is_newer("0.11.1", "0.11.0"), Some(true));
        assert_eq!(is_newer("0.11.0", "0.11.1"), Some(false));
        assert_eq!(is_newer("1.0.0", "0.9.9"), Some(true));
        assert_eq!(is_newer("0.9.9", "1.0.0"), Some(false));
    }

    #[test]
    fn whitespace_is_ignored() {
        assert_eq!(parse_version(" 1.2.3 \n"), Some((1, 2, 3)));
        assert_eq!(is_newer(" 1.2.3 ", "1.2.2"), Some(true));
    }

    #[test]
    fn rechecks_each_startup_when_current_but_throttles_pending_upgrade() {
        let now = Utc::now();
        let fresh = now - Duration::minutes(1);
        let info = |latest: &str, checked| VersionInfo {
            latest_version: latest.to_string(),
            last_checked_at: checked,
        };

        // No cache: always check.
        assert!(should_check_for_update(None, now));
        // Believed current (cached == running): re-check even when freshly checked.
        assert!(should_check_for_update(
            Some(&info(CODEX_CLI_VERSION, fresh)),
            now
        ));
        // Known pending upgrade, freshly checked: stay throttled.
        assert!(!should_check_for_update(Some(&info("999.0.0", fresh)), now));
        // Known pending upgrade, stale: re-check.
        assert!(should_check_for_update(
            Some(&info("999.0.0", now - Duration::hours(2))),
            now
        ));
    }

    use tempfile::tempdir;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::method;
    use wiremock::matchers::path;

    async fn mount_latest_release(server: &MockServer, tag_name: &str) {
        Mock::given(method("GET"))
            .and(path("/releases/latest"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(format!(r#"{{"tag_name":"{tag_name}"}}"#)),
            )
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn check_for_update_records_version_from_latest_endpoint() {
        let server = MockServer::start().await;
        mount_latest_release(&server, "v9.9.9").await;

        let dir = tempdir().expect("create temp dir");
        let version_file = dir.path().join(VERSION_FILENAME);
        let releases_api_url = format!("{}/releases", server.uri());

        check_for_update(&version_file, &releases_api_url)
            .await
            .expect("check_for_update should succeed");

        let info = read_version_info(&version_file).expect("version file should be written");
        assert_eq!(info.latest_version, "9.9.9");
    }

    #[tokio::test]
    async fn check_for_update_falls_back_to_release_list_when_latest_missing() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/releases/latest"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/releases"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"[{"tag_name":"v1.2.3"},{"tag_name":"v1.2.2"}]"#),
            )
            .mount(&server)
            .await;

        let dir = tempdir().expect("create temp dir");
        let version_file = dir.path().join(VERSION_FILENAME);
        let releases_api_url = format!("{}/releases", server.uri());

        check_for_update(&version_file, &releases_api_url)
            .await
            .expect("check_for_update should succeed via fallback");

        let info = read_version_info(&version_file).expect("version file should be written");
        assert_eq!(info.latest_version, "1.2.3");
    }

    #[tokio::test]
    async fn latest_release_tag_name_errors_when_release_list_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/releases/latest"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/releases"))
            .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
            .mount(&server)
            .await;

        let releases_api_url = format!("{}/releases", server.uri());
        let result = latest_release_tag_name(&releases_api_url).await;
        assert!(result.is_err());
    }

    #[test]
    fn update_lock_is_exclusive_and_released_on_drop() {
        let dir = tempdir().expect("create temp dir");
        let lock_path = dir.path().join(AUTO_UPDATE_LOCK_FILENAME);

        let guard = try_acquire_update_lock(&lock_path).expect("first acquisition succeeds");
        assert!(
            try_acquire_update_lock(&lock_path).is_none(),
            "a second concurrent acquisition must fail while the lock is held"
        );

        drop(guard);
        assert!(
            try_acquire_update_lock(&lock_path).is_some(),
            "dropping the guard (proxy for process exit) must release the lock"
        );
    }
}
