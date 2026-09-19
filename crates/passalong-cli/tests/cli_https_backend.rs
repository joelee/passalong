//! The `passalong` binary against a real passalong-server. Ignored by
//! default; `just test-https` runs them. Their names start with `https_`, so
//! `just test-integration` can leave them out.

#[path = "../../passalong-https/tests/support/mod.rs"]
mod support;

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use passalong_core::api_key::save_api_key;
use passalong_core::encryption::SystemGit;
use predicates::prelude::*;
use support::TestServer;
use tempfile::TempDir;

/// Writes a device's config for `server` into `dir`, with its API key.
fn device(server: &TestServer, dir: &Path) -> PathBuf {
    let config = dir.join("config.toml");
    std::fs::write(
        &config,
        format!(
            "[client]\ndevice_name = \"it-cli\"\nkey_file = '{}'\ndownload_dir = '{}'\n\n[server]\nkind = \"https\"\n\n[server.https]\nurl = \"{}\"\ntls_pin = \"{}\"\napi_key_file = '{}'\n",
            dir.join("store.key").display(),
            dir.join("downloads").display(),
            server.url,
            server.pin,
            dir.join("api.key").display(),
        ),
    )
    .unwrap();
    save_api_key(&dir.join("api.key"), &server.key, &SystemGit::new()).unwrap();
    config
}

fn passalong(dir: &Path, config: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_passalong"));
    cmd.current_dir(dir)
        .env("HOME", dir)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_STATE_HOME")
        .env_remove("PASSALONG_CONFIG_FILE")
        .env_remove("PASSALONG_LOG_LEVEL")
        .arg("--config")
        .arg(config);
    cmd
}

fn stdout(cmd: &mut Command) -> String {
    String::from_utf8(cmd.assert().success().get_output().stdout.clone()).unwrap()
}

#[test]
#[ignore = "needs passalong-server: just test-https"]
fn https_the_cli_sends_lists_loads_and_deletes_over_a_server() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let config = device(&server, dir.path());

    let text_id = stdout(
        passalong(dir.path(), &config)
            .args(["clipboard", "--stdin"])
            .write_stdin("sent over https"),
    )
    .trim()
    .to_owned();
    // The same text again stores nothing new.
    let again = stdout(
        passalong(dir.path(), &config)
            .args(["clipboard", "--stdin"])
            .write_stdin("sent over https"),
    );
    assert_eq!(again.trim(), text_id);

    let report = dir.path().join("report.pdf");
    std::fs::write(&report, b"%PDF-1.7 over https").unwrap();
    let file_id = stdout(passalong(dir.path(), &config).arg("file").arg(&report))
        .trim()
        .to_owned();

    let listed = stdout(passalong(dir.path(), &config).arg("list"));
    assert!(
        listed.contains(&text_id) && listed.contains(&file_id),
        "{listed}"
    );
    let json: serde_json::Value = serde_json::from_str(&stdout(
        passalong(dir.path(), &config).args(["list", "--json"]),
    ))
    .unwrap();
    // Sent within one second, the two share their time and sort by
    // content key, as in any store.
    let items = json.as_array().unwrap();
    assert_eq!(items.len(), 2);
    for id in [&text_id, &file_id] {
        let item = items.iter().find(|item| item["id"] == id.as_str()).unwrap();
        assert_eq!(item["device"], "it-cli");
    }

    passalong(dir.path(), &config)
        .args(["cat", &text_id])
        .assert()
        .success()
        .stdout("sent over https");
    passalong(dir.path(), &config)
        .args(["get", &file_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("report.pdf"));
    let out = dir.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    passalong(dir.path(), &config)
        .args(["load", &file_id[9..15]])
        .arg(&out)
        .assert()
        .success();
    assert_eq!(
        std::fs::read(out.join("report.pdf")).unwrap(),
        b"%PDF-1.7 over https"
    );

    passalong(dir.path(), &config)
        .args(["delete", &file_id])
        .assert()
        .success();
    passalong(dir.path(), &config)
        .args(["prune", "--keep", "0", "--yes"])
        .assert()
        .success();
    passalong(dir.path(), &config)
        .arg("list")
        .assert()
        .success()
        .stdout("no items\n");
}
