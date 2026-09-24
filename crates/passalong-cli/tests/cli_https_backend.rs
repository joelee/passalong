//! The `passalong` binary against a real passalong-server. Ignored by
//! default; `just test-https` runs them. Their names start with `https_`, so
//! `just test-integration` can leave them out.

#[path = "../../passalong-https/tests/support/mod.rs"]
mod support;

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use std::sync::Arc;

use passalong_core::api_key::{ApiKey, save_api_key};
use passalong_core::clock::SystemClock;
use passalong_core::crypto::{KdfParams, Words};
use passalong_core::encryption::{EncryptionAdmin, SystemGit, save_key_file};
use passalong_https::HttpEncryptionAdmin;
use predicates::prelude::*;
use support::TestServer;
use tempfile::TempDir;

/// Writes a device's config for `server` into `dir`, with its API key.
fn device(server: &TestServer, dir: &Path) -> PathBuf {
    device_as(server, dir, "it-cli", &server.key, "")
}

/// A device named `name` with API key `key`, and `extra` at the end of its
/// config.
fn device_as(server: &TestServer, dir: &Path, name: &str, key: &ApiKey, extra: &str) -> PathBuf {
    let config = dir.join("config.toml");
    std::fs::write(
        &config,
        format!(
            "[client]\ndevice_name = \"{name}\"\nkey_file = '{}'\ndownload_dir = '{}'\n\n[server]\nkind = \"https\"\n\n[server.https]\nurl = \"{}\"\ntls_pin = \"{}\"\napi_key_file = '{}'\n{extra}",
            dir.join("store.key").display(),
            dir.join("downloads").display(),
            server.url,
            server.pin,
            dir.join("api.key").display(),
        ),
    )
    .unwrap();
    save_api_key(&dir.join("api.key"), key, &SystemGit::new()).unwrap();
    config
}

/// `[serve]` settings that act within a test's patience.
#[cfg(unix)]
fn quick_serve(dir: &Path, pull: bool) -> String {
    std::fs::create_dir_all(dir.join("drop")).unwrap();
    std::fs::create_dir_all(dir.join("downloads")).unwrap();
    format!(
        "\n[serve]\ndrop_folder = '{}'\nfile_stable_wait_ms = 100\npull = {pull}\npull_interval_ms = 1000\n",
        dir.join("drop").display()
    )
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

/// `passalong serve` running in the background, stopped when dropped.
#[cfg(unix)]
struct Serving {
    child: std::process::Child,
    stderr: std::path::PathBuf,
}

#[cfg(unix)]
impl Serving {
    fn start(dir: &Path, config: &Path) -> Self {
        let stderr = dir.join("serve.stderr");
        let child = std::process::Command::new(env!("CARGO_BIN_EXE_passalong"))
            .current_dir(dir)
            .env("HOME", dir)
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_STATE_HOME")
            .env_remove("PASSALONG_CONFIG_FILE")
            .env_remove("PASSALONG_LOG_LEVEL")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .arg("--config")
            .arg(config)
            .arg("serve")
            .stdout(std::process::Stdio::null())
            .stderr(std::fs::File::create(&stderr).unwrap())
            .spawn()
            .unwrap();
        let mut serving = Self { child, stderr };
        serving.wait_for("serving:");
        serving
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.stderr).unwrap_or_default()
    }

    /// Waits up to 20 s for `text` in the log, while serve runs.
    fn wait_for(&mut self, text: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !self.log().contains(text) {
            assert!(self.running(), "serve exited:\n{}", self.log());
            assert!(
                std::time::Instant::now() < deadline,
                "no `{text}` in:\n{}",
                self.log()
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    /// Waits up to 20 s for serve to exit on its own.
    fn exit(&mut self) -> std::process::ExitStatus {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "serve kept running:\n{}",
                self.log()
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    fn running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }
}

#[cfg(unix)]
impl Drop for Serving {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Waits up to 20 s for `path` to exist.
#[cfg(unix)]
fn wait_for_file(path: &Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !path.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "{} never appeared",
            path.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
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

#[test]
#[ignore = "needs passalong-server: just test-https"]
fn https_a_fresh_start_warns_in_list_until_prune_plain_clears_it() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let config = device(&server, dir.path());
    passalong(dir.path(), &config)
        .args(["clipboard", "--stdin"])
        .write_stdin("from before encryption")
        .assert()
        .success();

    // `encrypt` needs a terminal, so the fresh start is the library's.
    let key = tokio::runtime::Runtime::new().unwrap().block_on(async {
        let loaded =
            passalong_core::config::load(&config, &passalong_core::config::StdEnv).unwrap();
        let client = passalong_https::connect(&loaded).unwrap();
        HttpEncryptionAdmin::new(client, Arc::new(SystemClock))
            .fresh_start(
                &Words::parse("abacus abdomen abdominal abide abiding ability").unwrap(),
                KdfParams {
                    m_kib: 64,
                    t: 1,
                    p: 1,
                    salt: [7; 16],
                },
            )
            .await
            .unwrap()
    });
    save_key_file(&dir.path().join("store.key"), &key, &SystemGit::new()).unwrap();

    passalong(dir.path(), &config)
        .arg("list")
        .assert()
        .success()
        .stdout("no items\n")
        .stderr(predicate::str::contains(
            "1 unencrypted item remains from before encryption; remove it with `passalong prune --plain`",
        ));
    passalong(dir.path(), &config)
        .args(["prune", "--plain", "--keep", "0", "--yes"])
        .assert()
        .success();
    passalong(dir.path(), &config)
        .arg("list")
        .assert()
        .success()
        .stdout("no items\n")
        .stderr(predicate::str::contains("unencrypted").not());
    passalong(dir.path(), &config)
        .args(["prune", "--plain", "--keep", "0", "--yes"])
        .assert()
        .success()
        .stdout("no unencrypted items remain\n");
}

#[cfg(unix)]
#[test]
#[ignore = "needs passalong-server: just test-https"]
fn https_serve_stops_for_good_when_its_key_is_revoked() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let key = server.create_key(&[]);
    let config = device_as(
        &server,
        dir.path(),
        "it-cli",
        &key,
        &quick_serve(dir.path(), false),
    );
    let mut serving = Serving::start(dir.path(), &config);
    server.run(&["key", "revoke", key.id()]);
    let file = dir.path().join("drop/after.txt");
    std::fs::write(&file, "sent after the revocation").unwrap();
    let status = serving.exit();
    let log = serving.log();
    assert!(!status.success(), "{log}");
    assert!(log.contains("serve stopped: "), "{log}");
    assert!(log.contains("KEY_REVOKED"), "{log}");
    assert!(
        log.contains("ask the server's operator for a new API key"),
        "{log}"
    );
    assert!(file.exists(), "the file stays for a later serve");
}

#[cfg(unix)]
#[test]
#[ignore = "needs passalong-server: just test-https"]
fn https_serve_in_pull_mode_stops_when_its_key_is_revoked() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let key = server.create_key(&[]);
    let config = device_as(
        &server,
        dir.path(),
        "it-cli",
        &key,
        &quick_serve(dir.path(), true),
    );
    let mut serving = Serving::start(dir.path(), &config);
    server.run(&["key", "revoke", key.id()]);
    // Nothing to send: the next poll finds out.
    let status = serving.exit();
    let log = serving.log();
    assert!(!status.success(), "{log}");
    assert!(log.contains("KEY_REVOKED"), "{log}");
}

#[cfg(unix)]
#[test]
#[ignore = "needs passalong-server: just test-https"]
fn https_serve_waits_out_a_rewrite_and_sends_after_it() {
    use passalong_core::crypto::{DataKey, KdfParams, Words, wrap};
    use passalong_core::encryption::StoreHeader;

    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let config = device_as(
        &server,
        dir.path(),
        "it-cli",
        &server.key,
        &quick_serve(dir.path(), false),
    );
    let mut serving = Serving::start(dir.path(), &config);

    // Another device begins migrating the workspace, and holds the lease.
    let other = TempDir::new().unwrap();
    let other_config = device_as(&server, other.path(), "other", &server.create_key(&[]), "");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let client = passalong_https::connect(
        &passalong_core::config::load(&other_config, &passalong_core::config::StdEnv).unwrap(),
    )
    .unwrap();
    let new_key = DataKey::generate().unwrap();
    let header = StoreHeader::new(
        wrap(
            &new_key,
            &Words::generate().unwrap(),
            KdfParams {
                m_kib: 64,
                t: 1,
                p: 1,
                salt: [3; 16],
            },
        )
        .unwrap(),
    );
    let header = String::from_utf8(header.to_json()).unwrap();
    let post = |path: &str, body: String| {
        let url = client.url(path);
        runtime
            .block_on(client.send("rewrite", |http| {
                http.post(&url)
                    .header("content-type", "application/json")
                    .body(body.clone())
            }))
            .unwrap();
    };
    post(
        "/rewrite",
        format!(
            "{{\"kind\":\"migrate\",\"expectedKeyId\":null,\"newKeyId\":\"{}\",\"newHeader\":{}}}",
            new_key.key_id(),
            header.trim_end()
        ),
    );

    let file = dir.path().join("drop/during.txt");
    std::fs::write(&file, "sent during a rewrite").unwrap();
    serving.wait_for("waiting for the store's re-encryption to end");
    assert!(file.exists());
    assert!(serving.running());

    post(
        "/rewrite/abort",
        format!("{{\"newKeyId\":\"{}\"}}", new_key.key_id()),
    );
    wait_for_file(&dir.path().join("drop/sent/during.txt"));
    serving.wait_for("re-encryption has ended");
    assert!(serving.running());
}

#[cfg(unix)]
#[test]
#[ignore = "needs passalong-server: just test-https"]
fn https_serve_keeps_a_file_when_the_workspace_s_key_changes() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let config = device_as(
        &server,
        dir.path(),
        "it-cli",
        &server.key,
        &quick_serve(dir.path(), false),
    );
    let mut serving = Serving::start(dir.path(), &config);
    let loaded = passalong_core::config::load(&config, &passalong_core::config::StdEnv).unwrap();
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(server.seal(&passalong_https::connect(&loaded).unwrap()));

    let file = dir.path().join("drop/plain.txt");
    std::fs::write(&file, "never sent unencrypted").unwrap();
    serving.wait_for("encrypt --join");
    std::thread::sleep(std::time::Duration::from_secs(2));
    assert!(serving.running(), "{}", serving.log());
    assert!(file.exists(), "the file is kept");
    let workspace = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(passalong_https::connect(&loaded).unwrap().workspace())
        .unwrap();
    assert_eq!(workspace.item_count, 0);
}

#[cfg(unix)]
#[test]
#[ignore = "needs passalong-server: just test-https"]
fn https_pull_mode_applies_an_item_another_key_sent() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let config = device_as(
        &server,
        dir.path(),
        "it-cli",
        &server.key,
        &quick_serve(dir.path(), true),
    );
    let _serving = Serving::start(dir.path(), &config);

    let other = TempDir::new().unwrap();
    let other_config = device_as(&server, other.path(), "other", &server.create_key(&[]), "");
    let report = other.path().join("from-other.txt");
    std::fs::write(&report, "sent by another key").unwrap();
    passalong(other.path(), &other_config)
        .arg("file")
        .arg(&report)
        .assert()
        .success();
    let pulled = dir.path().join("downloads/from-other.txt");
    wait_for_file(&pulled);
    // Written through a temporary file, so it is whole once it appears.
    assert_eq!(
        std::fs::read_to_string(pulled).unwrap(),
        "sent by another key"
    );
}

#[test]
#[ignore = "needs passalong-server: just test-https"]
fn https_list_uses_a_fresh_cache_without_connecting() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let config = device(&server, dir.path());
    let id = stdout(
        passalong(dir.path(), &config)
            .args(["clipboard", "--stdin"])
            .write_stdin("cached"),
    )
    .trim()
    .to_owned();
    let listed = stdout(passalong(dir.path(), &config).arg("list"));
    assert!(listed.contains(&id), "{listed}");

    drop(server);
    let cached = stdout(passalong(dir.path(), &config).arg("list"));
    assert_eq!(cached, listed, "listed from the cache, the server gone");
}

#[test]
#[ignore = "needs passalong-server: just test-https"]
fn https_init_writes_a_config_that_check_passes() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("cfg/config.toml");
    let api_key_file = dir.path().join("cfg/api.key");
    let key = server.create_key(&["--expires", "7d"]);
    save_api_key(&api_key_file, &key, &SystemGit::new()).unwrap();
    let pin = server.pin.to_string();
    let out = stdout(
        passalong(dir.path(), &config)
            .args([
                "init",
                "--backend",
                "https",
                "--url",
                &server.url,
                "--tls-pin",
                &pin,
            ])
            .arg("--api-key-file")
            .arg(&api_key_file)
            .args(["--device-name", "it-init", "--yes"]),
    );
    assert!(out.contains(&pin), "{out}");
    assert!(out.contains("connected to passalong-server"), "{out}");
    assert!(out.contains("connected: 0 items on the server"), "{out}");
    assert!(!out.contains(key.expose()), "the key is never shown");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&api_key_file)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    let checked = stdout(passalong(dir.path(), &config).arg("check"));
    let lines: Vec<&str> = checked.lines().collect();
    assert!(
        lines[1].starts_with(&format!(
            "server         ok    https {}: passalong-server ",
            server.url
        )) && lines[1].ends_with(", API v1, TLS pinned"),
        "{checked}"
    );
    assert!(lines[2].starts_with("api key        warn  "), "{checked}");
    assert!(lines[2].contains("(read-write), expires "), "{checked}");
    assert!(
        lines[2].contains("in 6 days") || lines[2].contains("in 7 days"),
        "{checked}"
    );
    assert!(
        lines[3].starts_with(&format!(
            "workspace      ok    {}: 0 B of ",
            support::WORKSPACE
        )),
        "{checked}"
    );
    assert_eq!(lines[4], "encryption     off   not encrypted");
    assert_eq!(lines[5], "storage read   ok    0 items");
    assert!(
        lines[6].starts_with("storage write  ok    wrote and removed"),
        "{checked}"
    );
}

#[test]
#[ignore = "needs passalong-server: just test-https"]
fn https_check_reports_a_read_only_key_without_probing() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let reader = server.create_key(&["--read-only"]);
    let config = device_as(&server, dir.path(), "it-cli", &reader, "");
    let checked = stdout(passalong(dir.path(), &config).arg("check"));
    assert!(
        checked.contains("api key        ok    ") && checked.contains("(read-only), expires "),
        "{checked}"
    );
    assert!(
        checked.contains(
            "storage write  n/a   a read-only API key: this device lists and loads, and cannot send\n"
        ),
        "{checked}"
    );
}

#[test]
#[ignore = "needs passalong-server: just test-https"]
fn https_init_refuses_a_wrong_pin_and_writes_nothing() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("config.toml");
    save_api_key(&dir.path().join("api.key"), &server.key, &SystemGit::new()).unwrap();
    passalong(dir.path(), &config)
        .args([
            "init",
            "--url",
            &server.url,
            "--tls-pin",
            "sha256/Zmh6rfhivXdsj8GLjp+OIAiXFIVu4jOzkCpZHQ1fKSU=",
            "--yes",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("mismatch"))
        .stderr(predicate::str::contains(server.pin.to_string()));
    assert!(!config.exists());
}

#[test]
#[ignore = "needs passalong-server: just test-https"]
fn https_a_debug_session_never_logs_the_api_key() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let config = device(&server, dir.path());
    let secret = server.key.expose().rsplit('_').next().unwrap().to_owned();
    let mut logged = String::new();
    let mut session = |args: &[&str], stdin: Option<&str>| {
        let mut cmd = passalong(dir.path(), &config);
        cmd.env("PASSALONG_LOG_LEVEL", "debug").args(args);
        if let Some(stdin) = stdin {
            cmd.write_stdin(stdin);
        }
        let out = cmd.assert().success().get_output().clone();
        logged.push_str(&String::from_utf8_lossy(&out.stdout));
        logged.push_str(&String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    };
    let id = session(&["clipboard", "--stdin"], Some("logged at debug"))
        .trim()
        .to_owned();
    session(&["list", "--nocache"], None);
    session(&["cat", &id], None);
    session(&["check"], None);
    session(&["delete", &id], None);
    assert!(
        logged.contains("DEBUG") || logged.contains("debug"),
        "{logged}"
    );
    assert!(!logged.contains(&secret), "the API key's secret was logged");
    assert!(
        !server.log().contains(&secret),
        "the server logged the secret"
    );
}
