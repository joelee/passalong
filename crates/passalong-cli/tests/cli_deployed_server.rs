//! The `passalong` binary against a deployed passalong-server (PLAN-00011).
//! Ignored by default; `just test-deployed` loads `.env` and runs them, one
//! at a time, because they share the deployment's test workspace. Their
//! names start with `deployed_`, so every other recipe leaves them out.

#[path = "../../passalong-https/tests/support/deployed.rs"]
mod deployed;

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use deployed::Deployed;
use passalong_core::api_key::{ApiKey, save_api_key};
use passalong_core::encryption::SystemGit;
use tempfile::TempDir;

/// Writes a device's config for `server` into `dir` with API key `key`,
/// named `name`, with `extra` at the end of its config.
fn device_as(server: &Deployed, dir: &Path, name: &str, key: &ApiKey, extra: &str) -> PathBuf {
    let config = dir.join("config.toml");
    std::fs::write(
        &config,
        Deployed::config_text(dir, &server.url, Some(&server.pin), name, extra),
    )
    .unwrap();
    save_api_key(&Deployed::key_file(dir), key, &SystemGit::new()).unwrap();
    config
}

/// A read-write device in `dir`.
fn device(server: &Deployed, dir: &Path) -> PathBuf {
    device_as(server, dir, "it-deployed", &server.key, "")
}

fn passalong(dir: &Path, config: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_passalong"));
    cmd.current_dir(dir)
        .env("HOME", dir)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_STATE_HOME")
        .env_remove("PASSALONG_CONFIG_FILE")
        .env_remove("PASSALONG_LOG_LEVEL")
        // Never the desktop's clipboard.
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .arg("--config")
        .arg(config);
    cmd
}

fn stdout(cmd: &mut Command) -> String {
    let out = cmd.assert().success().get_output().clone();
    String::from_utf8(out.stdout).unwrap()
}

#[test]
#[ignore = "needs the deployed server: just test-deployed"]
fn deployed_check_passes_with_the_read_write_key() {
    let server = Deployed::from_env();
    let dir = TempDir::new().unwrap();
    let config = device(&server, dir.path());
    let checked = stdout(passalong(dir.path(), &config).arg("check"));
    let lines: Vec<&str> = checked.lines().collect();
    assert!(
        lines[1].starts_with(&format!(
            "server         ok    https {}: passalong-server ",
            server.url
        )) && lines[1].ends_with(", API v1, TLS pinned"),
        "{checked}"
    );
    assert!(
        lines[3].starts_with(&format!("workspace      ok    {}: ", server.workspace)),
        "{checked}"
    );
}
