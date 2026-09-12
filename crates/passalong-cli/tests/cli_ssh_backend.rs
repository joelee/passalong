//! End-to-end tests of the `passalong` binary against the Docker OpenSSH
//! server. Ignored by default; `just test-integration` runs them with the
//! `PASSALONG_IT_SSH_*` variables set, and they fail when those are missing.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use passalong_core::random::{RandomSource, StdRandom};
use predicates::prelude::*;
use tempfile::TempDir;

/// A valid key that the test server does not have.
const OTHER_KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMKy9BQGg0B6NYvYwyrJzGCOHCXKQBj7E/5jvWJSEDCi passalong-test-b";

fn var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!("{name} is not set: run these tests with `just test-integration`")
    })
}

/// Writes a config for the test server with its own remote directory.
fn write_config(dir: &Path, host_key: &str) -> PathBuf {
    let remote = format!(
        "{}/cli-{:016x}",
        var("PASSALONG_IT_SSH_REMOTE_PATH"),
        StdRandom::new().next_u64()
    );
    let text = format!(
        "[client]\ndevice_name = \"it-cli\"\n\n[server]\nkind = \"ssh\"\n\n[server.ssh]\nhost = \"{}\"\nport = {}\nuser = \"{}\"\nhost_key = \"{host_key}\"\nidentity_file = \"{}\"\nremote_path = \"{remote}\"\n\n[serve]\ndrop_folder = \"{}\"\n",
        var("PASSALONG_IT_SSH_HOST"),
        var("PASSALONG_IT_SSH_PORT"),
        var("PASSALONG_IT_SSH_USER"),
        var("PASSALONG_IT_SSH_IDENTITY"),
        dir.join("drop").display()
    );
    let path = dir.join("config.toml");
    std::fs::write(&path, text).unwrap();
    path
}

fn passalong(dir: &Path, config: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_passalong"));
    cmd.current_dir(dir)
        .env("HOME", dir)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("PASSALONG_CONFIG_FILE")
        .env_remove("PASSALONG_LOG_LEVEL")
        .env_remove("PASSALONG_SSH_KEY_PASSPHRASE")
        .arg("--config")
        .arg(config);
    cmd
}

#[test]
#[ignore = "needs the Docker SSH server: just test-integration"]
fn clipboard_list_load_round_trip_over_ssh() {
    let dir = TempDir::new().unwrap();
    let config = write_config(dir.path(), &var("PASSALONG_IT_SSH_HOST_KEY"));
    let out = passalong(dir.path(), &config)
        .args(["clipboard", "--stdin"])
        .write_stdin("sent over ssh")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let id = String::from_utf8(out).unwrap().trim().to_owned();

    let out = passalong(dir.path(), &config)
        .args(["list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json[0]["id"], id.as_str());
    assert_eq!(json[0]["device"], "it-cli");

    let dest = dir.path().join("out");
    std::fs::create_dir_all(&dest).unwrap();
    passalong(dir.path(), &config)
        .args(["load", &id[9..15]])
        .arg(&dest)
        .assert()
        .success();
    assert_eq!(
        std::fs::read_to_string(dest.join(format!("{id}.txt"))).unwrap(),
        "sent over ssh"
    );
}

#[test]
#[ignore = "needs the Docker SSH server: just test-integration"]
fn a_wrong_host_key_fails_with_a_mismatch_error() {
    let dir = TempDir::new().unwrap();
    let config = write_config(dir.path(), OTHER_KEY);
    passalong(dir.path(), &config)
        .arg("list")
        .assert()
        .code(1)
        .stderr(predicate::str::starts_with("error: host key mismatch"));
}

#[test]
#[ignore = "needs the Docker SSH server: just test-integration"]
fn scripted_init_pins_the_confirmed_key_and_connects() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("init.toml");
    let fingerprint =
        passalong_ssh::host_key::PinnedHostKey::parse(&var("PASSALONG_IT_SSH_HOST_KEY"))
            .unwrap()
            .fingerprint();
    let remote = format!(
        "{}/init-{:016x}",
        var("PASSALONG_IT_SSH_REMOTE_PATH"),
        StdRandom::new().next_u64()
    );
    Command::new(env!("CARGO_BIN_EXE_passalong"))
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("PASSALONG_CONFIG_FILE")
        .args(["--config"])
        .arg(&target)
        .args([
            "init",
            "--host",
            &var("PASSALONG_IT_SSH_HOST"),
            "--port",
            &var("PASSALONG_IT_SSH_PORT"),
        ])
        .args([
            "--user",
            &var("PASSALONG_IT_SSH_USER"),
            "--identity-file",
            &var("PASSALONG_IT_SSH_IDENTITY"),
        ])
        .args([
            "--remote-path",
            &remote,
            "--device-name",
            "it-init",
            "--fingerprint",
            &fingerprint,
            "--yes",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains(&fingerprint).and(predicate::str::ends_with(
                "connected: 0 items on the server\n",
            )),
        );
    let pinned = std::fs::read_to_string(&target).unwrap();
    let expected = var("PASSALONG_IT_SSH_HOST_KEY");
    assert!(
        pinned.contains(&expected),
        "config does not pin the server key:\n{pinned}"
    );
    passalong(dir.path(), &target)
        .arg("list")
        .assert()
        .success()
        .stdout("no items\n");
}
