//! The `passalong` binary against a deployed passalong-server (PLAN-00011).
//! Ignored by default; `just test-deployed` loads `.env` and runs them, one
//! at a time, because they share the deployment's test workspace. Their
//! names start with `deployed_`, so every other recipe leaves them out.

#[path = "../../passalong-https/tests/support/cli.rs"]
mod cli_support;
#[path = "../../passalong-https/tests/support/deployed.rs"]
mod deployed;

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use assert_cmd::Command;
#[cfg(unix)]
use cli_support::{Serving, quick_serve, wait_for_file};
use deployed::{Deployed, SKIPPED, ThrowawayWorkspace};
use passalong_core::api_key::{ApiKey, save_api_key};
use passalong_core::clock::SystemClock;
use passalong_core::crypto::{DataKey, KdfParams, Sealer, Words, wrap};
use passalong_core::encryption::{EncryptionAdmin, StoreHeader, SystemGit, save_key_file};
use passalong_core::store::Store;
use passalong_https::{Client, HttpEncryptionAdmin, HttpStore, Partition};
use predicates::prelude::*;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

/// Writes a device's config for `server` into `dir` with API key `key`,
/// named `name`, with `extra` at the end of its config. The key decides
/// the workspace: a throwaway workspace's key reaches that workspace.
fn device_as(server: &Deployed, dir: &Path, name: &str, key: &ApiKey, extra: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
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

fn stderr(cmd: &mut Command) -> String {
    let out = cmd.assert().failure().get_output().clone();
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Something no earlier run sent: the time, to the nanosecond.
fn nonce() -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    format!("{}.{:09}", now.as_secs(), now.subsec_nanos())
}

/// `len` bytes that no earlier run sent, from a small generator seeded by
/// the clock; the content only has to differ, not be unpredictable.
fn fresh_bytes(len: usize) -> Vec<u8> {
    let mut state = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
        | 1;
    (0..len)
        .map(|_| {
            // xorshift64
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// The ids `list --json` names, newest first.
fn listed_ids(dir: &Path, config: &Path) -> Vec<String> {
    let json: serde_json::Value = serde_json::from_str(&stdout(passalong(dir, config).args([
        "list",
        "--json",
        "--nocache",
    ])))
    .unwrap();
    json.as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_owned())
        .collect()
}

/// Items a test sent, deleted when it ends, however it ends.
struct Sent {
    dir: PathBuf,
    config: PathBuf,
    ids: Vec<String>,
}

impl Sent {
    fn new(dir: &Path, config: &Path) -> Self {
        Self {
            dir: dir.to_owned(),
            config: config.to_owned(),
            ids: Vec::new(),
        }
    }

    /// Sends `text` from the clipboard and remembers its id.
    fn text(&mut self, text: &str) -> String {
        let id = stdout(
            passalong(&self.dir, &self.config)
                .args(["clipboard", "--stdin"])
                .write_stdin(text.to_owned()),
        )
        .trim()
        .to_owned();
        self.remember(&id);
        id
    }

    /// Sends the file at `path` and remembers its id.
    fn file(&mut self, path: &Path) -> String {
        let id = stdout(passalong(&self.dir, &self.config).arg("file").arg(path))
            .trim()
            .to_owned();
        self.remember(&id);
        id
    }

    fn remember(&mut self, id: &str) {
        if !self.ids.contains(&id.to_owned()) {
            self.ids.push(id.to_owned());
        }
    }

    /// Deletes everything sent, now rather than at the end.
    fn delete_all(&mut self) {
        if self.ids.is_empty() {
            return;
        }
        let mut cmd = passalong(&self.dir, &self.config);
        cmd.arg("delete").args(&self.ids);
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "delete failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        self.ids.clear();
    }
}

impl Drop for Sent {
    fn drop(&mut self) {
        if !self.ids.is_empty() {
            // Best effort: a failing test must not leave items behind.
            let _ = passalong(&self.dir, &self.config)
                .arg("delete")
                .args(&self.ids)
                .output();
        }
    }
}

#[test]
#[ignore = "needs the deployed server: just test-deployed"]
fn deployed_init_yes_then_check_passes() {
    let server = Deployed::from_env();
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("cfg/config.toml");
    let api_key_file = dir.path().join("cfg/api.key");
    save_api_key(&api_key_file, &server.key, &SystemGit::new()).unwrap();
    let out = stdout(
        passalong(dir.path(), &config)
            .args(["init", "--url", &server.url, "--tls-pin", &server.pin])
            .arg("--api-key-file")
            .arg(&api_key_file)
            .args(["--device-name", "it-init", "--yes"]),
    );
    let pin = passalong_core::config::TlsPin::parse(&server.pin)
        .unwrap()
        .to_string();
    assert!(out.contains(&pin), "{out}");
    assert!(out.contains("connected to passalong-server"), "{out}");
    assert!(out.contains("(read-write), never expires"), "{out}");
    assert!(!out.contains(server.key.expose()), "the key is never shown");
    assert!(config.exists());
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
        lines[0].starts_with("config         ok    ") && lines[0].ends_with("config.toml"),
        "{checked}"
    );
    assert!(
        lines[1].starts_with(&format!(
            "server         ok    https {}: passalong-server ",
            server.url
        )) && lines[1].ends_with(", API v1, TLS pinned"),
        "{checked}"
    );
    assert!(
        lines[2].starts_with("api key        ok    ")
            && lines[2].ends_with("(read-write), never expires"),
        "{checked}"
    );
    assert!(
        lines[3].starts_with(&format!("workspace      ok    {}: ", server.workspace))
            && lines[3].contains(" of 256.0 MiB used, "),
        "{checked}"
    );
    assert_eq!(lines[4], "encryption     off   not encrypted");
    assert!(lines[5].starts_with("storage read   ok    "), "{checked}");
    assert!(
        lines[6].starts_with("storage write  ok    wrote and removed a 128-byte probe"),
        "{checked}"
    );
    assert!(lines[7].starts_with("serve          off   "), "{checked}");
}

#[test]
#[ignore = "needs the deployed server: just test-deployed"]
fn deployed_text_and_file_round_trip_byte_identically() {
    let server = Deployed::from_env();
    let dir = TempDir::new().unwrap();
    let config = device(&server, dir.path());
    let before = listed_ids(dir.path(), &config);
    let mut sent = Sent::new(dir.path(), &config);

    let text = format!("sent to the deployed server at {}", nonce());
    let text_id = sent.text(&text);
    let bytes = fresh_bytes(1024 * 1024);
    let report = dir.path().join("report.bin");
    std::fs::write(&report, &bytes).unwrap();
    let file_id = sent.file(&report);
    assert_ne!(text_id, file_id);

    // Listed, attributed to this device, with the content's hash.
    let json: serde_json::Value =
        serde_json::from_str(&stdout(passalong(dir.path(), &config).args([
            "list",
            "--json",
            "--nocache",
        ])))
        .unwrap();
    let items = json.as_array().unwrap();
    assert_eq!(items.len(), before.len() + 2, "{json}");
    for (id, hash, kind) in [
        (&text_id, sha256_hex(text.as_bytes()), "text"),
        (&file_id, sha256_hex(&bytes), "file"),
    ] {
        let item = items
            .iter()
            .find(|item| item["id"] == id.as_str())
            .unwrap_or_else(|| panic!("{id} not in {json}"));
        assert_eq!(item["device"], "it-deployed", "{item}");
        assert_eq!(item["kind"], kind, "{item}");
        assert_eq!(item["sha256"], hash.as_str(), "{item}");
    }
    let got = stdout(passalong(dir.path(), &config).args(["get", &file_id]));
    assert!(
        got.contains(&format!("sha256:  {}", sha256_hex(&bytes))),
        "{got}"
    );
    assert!(got.contains("name:    report.bin"), "{got}");
    assert!(
        got.contains(&format!("size:    1.0 MiB ({} bytes)", bytes.len())),
        "{got}"
    );

    // Read back three ways, byte for byte.
    passalong(dir.path(), &config)
        .args(["cat", &text_id])
        .assert()
        .success()
        .stdout(text.clone());
    let out = dir.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    passalong(dir.path(), &config)
        .args(["load", &file_id])
        .arg(&out)
        .assert()
        .success();
    assert_eq!(std::fs::read(out.join("report.bin")).unwrap(), bytes);
    passalong(dir.path(), &config)
        .args(["load", &text_id])
        .arg(&out)
        .assert()
        .success();
    assert_eq!(
        std::fs::read_to_string(out.join(format!("{text_id}.txt"))).unwrap(),
        text
    );

    // Gone, and the workspace as it was.
    sent.delete_all();
    assert_eq!(listed_ids(dir.path(), &config), before);
}

#[test]
#[ignore = "needs the deployed server: just test-deployed"]
fn deployed_identical_content_is_stored_once() {
    let server = Deployed::from_env();
    let dir = TempDir::new().unwrap();
    let config = device(&server, dir.path());
    let before = listed_ids(dir.path(), &config);
    let mut sent = Sent::new(dir.path(), &config);

    let text = format!("sent twice at {}", nonce());
    let first = sent.text(&text);
    let again = sent.text(&text);
    assert_eq!(again, first, "the same content has one id");
    let after = listed_ids(dir.path(), &config);
    assert_eq!(after.len(), before.len() + 1, "{after:?}");

    sent.delete_all();
    assert_eq!(listed_ids(dir.path(), &config), before);
}

#[test]
#[ignore = "needs the deployed server: just test-deployed"]
fn deployed_the_read_only_key_checks_lists_and_is_refused_to_send() {
    let server = Deployed::from_env();
    let dir = TempDir::new().unwrap();
    let config = device_as(&server, dir.path(), "it-reader", &server.read_only_key, "");
    let before = listed_ids(dir.path(), &config);

    let checked = stdout(passalong(dir.path(), &config).arg("check"));
    let lines: Vec<&str> = checked.lines().collect();
    assert!(
        lines[2].starts_with("api key        ok    ")
            && lines[2].ends_with("(read-only), never expires"),
        "{checked}"
    );
    assert_eq!(
        lines[6],
        "storage write  n/a   a read-only API key: this device lists and loads, and cannot send",
        "{checked}"
    );

    let err = stderr(
        passalong(dir.path(), &config)
            .args(["clipboard", "--stdin"])
            .write_stdin(format!("read-only at {}", nonce())),
    );
    assert!(err.contains("FORBIDDEN_ROLE"), "{err}");
    assert!(err.contains("read-only"), "{err}");
    assert_eq!(
        listed_ids(dir.path(), &config),
        before,
        "nothing was stored"
    );
}

#[test]
#[ignore = "needs the deployed server: just test-deployed"]
fn deployed_no_secret_reaches_the_output() {
    let server = Deployed::from_env();
    let dir = TempDir::new().unwrap();
    let config = device(&server, dir.path());
    let secrets: Vec<String> = [&server.key, &server.read_only_key]
        .iter()
        .map(|key| key.expose().rsplit('_').next().unwrap().to_owned())
        .collect();
    let mut logged = String::new();
    let mut session = |args: &[&str], stdin: Option<String>| {
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
    let id = session(
        &["clipboard", "--stdin"],
        Some(format!("logged at debug at {}", nonce())),
    )
    .trim()
    .to_owned();
    session(&["list", "--nocache"], None);
    session(&["cat", &id], None);
    session(&["get", &id], None);
    session(&["check"], None);
    session(&["delete", &id], None);
    assert!(
        logged.contains("DEBUG") || logged.contains("debug"),
        "{logged}"
    );
    for secret in &secrets {
        assert!(
            !logged.contains(secret),
            "an API key's secret was in the output"
        );
    }
}

#[test]
#[ignore = "needs the deployed server: just test-deployed"]
fn deployed_a_file_over_the_quota_is_refused_naming_it() {
    let server = Deployed::from_env();
    let dir = TempDir::new().unwrap();
    let config = device(&server, dir.path());
    let before = listed_ids(dir.path(), &config);
    let quota: u64 = 256 * 1024 * 1024;

    // Sparse: the server refuses at `beginUpload`, so no content is sent.
    let big = dir.path().join("too-big.bin");
    let file = std::fs::File::create(&big).unwrap();
    file.set_len(quota + 1024 * 1024).unwrap();
    drop(file);

    let started = std::time::Instant::now();
    let err = stderr(passalong(dir.path(), &config).arg("file").arg(&big));
    let elapsed = started.elapsed();
    assert!(err.contains("QUOTA_EXCEEDED"), "{err}");
    assert!(err.contains(&quota.to_string()), "names the quota: {err}");
    assert!(
        elapsed < std::time::Duration::from_secs(30),
        "refused only after {elapsed:?}: was the content sent?"
    );
    assert_eq!(
        listed_ids(dir.path(), &config),
        before,
        "nothing was stored"
    );
}

/// The id of the item named `name` in `list --json`, once it is there.
#[cfg(unix)]
fn wait_for_listed(dir: &Path, config: &Path, name: &str) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let json: serde_json::Value = serde_json::from_str(&stdout(passalong(dir, config).args([
            "list",
            "--json",
            "--nocache",
        ])))
        .unwrap();
        if let Some(item) = json
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == name)
        {
            return item["id"].as_str().unwrap().to_owned();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{name} never listed: {json}"
        );
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

#[cfg(unix)]
#[test]
#[ignore = "needs the deployed server: just test-deployed"]
fn deployed_serve_sends_a_dropped_file() {
    let server = Deployed::from_env();
    let dir = TempDir::new().unwrap();
    let config = device_as(
        &server,
        dir.path(),
        "it-deployed",
        &server.key,
        &quick_serve(dir.path(), false),
    );
    let before = listed_ids(dir.path(), &config);
    let mut sent = Sent::new(dir.path(), &config);
    let serving = Serving::start(dir.path(), &config);

    let name = format!("dropped-{}.bin", nonce());
    let bytes = fresh_bytes(64 * 1024);
    std::fs::write(dir.path().join("drop").join(&name), &bytes).unwrap();
    wait_for_file(&dir.path().join("drop/sent").join(&name));
    let id = wait_for_listed(dir.path(), &config, &name);
    sent.remember(&id);
    drop(serving);

    let got = stdout(passalong(dir.path(), &config).args(["get", &id]));
    assert!(
        got.contains(&format!("sha256:  {}", sha256_hex(&bytes))),
        "{got}"
    );
    assert!(got.contains("device:  it-deployed"), "{got}");
    sent.delete_all();
    assert_eq!(listed_ids(dir.path(), &config), before);
}

#[cfg(unix)]
#[test]
#[ignore = "needs the deployed server: just test-deployed"]
fn deployed_serve_pulls_an_item_from_another_device() {
    let server = Deployed::from_env();
    let dir = TempDir::new().unwrap();
    let config = device_as(
        &server,
        dir.path(),
        "it-deployed",
        &server.key,
        &quick_serve(dir.path(), true),
    );
    let before = listed_ids(dir.path(), &config);
    let serving = Serving::start(dir.path(), &config);

    // Pull mode applies what other device names sent, under any key.
    let other = TempDir::new().unwrap();
    let other_config = device_as(&server, other.path(), "it-other", &server.key, "");
    let mut sent = Sent::new(other.path(), &other_config);
    let name = format!("from-other-{}.bin", nonce());
    let bytes = fresh_bytes(64 * 1024);
    let report = other.path().join(&name);
    std::fs::write(&report, &bytes).unwrap();
    sent.file(&report);

    let pulled = dir.path().join("downloads").join(&name);
    wait_for_file(&pulled);
    // Written through a temporary file, so it is whole once it appears.
    assert_eq!(std::fs::read(&pulled).unwrap(), bytes);
    drop(serving);

    sent.delete_all();
    assert_eq!(listed_ids(dir.path(), &config), before);
}

#[test]
#[ignore = "needs the deployed server: just test-deployed"]
fn deployed_a_throwaway_workspace_is_created_and_deleted() {
    let server = Deployed::from_env();
    let Some(admin) = server.admin() else {
        eprintln!("{SKIPPED}");
        return;
    };
    let dir = TempDir::new().unwrap();
    let workspace = ThrowawayWorkspace::new(&admin, "64MiB");
    let name = workspace.name.clone();
    assert!(name.starts_with("it-") && name.len() == 11, "{name}");
    assert!(admin.workspace_names().contains(&name));
    let shown = admin.show(&name);
    assert_eq!(shown["effectiveQuotaBytes"], "67108864", "{shown}");
    assert_eq!(shown["keys"].as_array().unwrap().len(), 1, "{shown}");

    // Its key reaches it, and nothing else.
    let config = device_as(&server, dir.path(), "it-throwaway", &workspace.key, "");
    let checked = stdout(passalong(dir.path(), &config).arg("check"));
    assert!(
        checked.contains(&format!(
            "workspace      ok    {name}: 0 B of 64.0 MiB used, 0 items"
        )),
        "{checked}"
    );
    let mut sent = Sent::new(dir.path(), &config);
    sent.text(&format!("in a throwaway workspace at {}", nonce()));
    assert_eq!(admin.show(&name)["items"], 1);
    let home_config = device(&server, dir.path().join("home").as_path());
    assert!(
        !listed_ids(&dir.path().join("home"), &home_config)
            .iter()
            .any(|id| sent.ids.contains(id)),
        "the item stayed in the throwaway workspace"
    );
    sent.ids.clear();

    drop(workspace);
    assert!(!admin.workspace_names().contains(&name), "deleted");
    assert!(admin.key_ids_of(&name).is_empty(), "its keys went with it");
}

#[test]
#[ignore = "needs the deployed server: just test-deployed"]
fn deployed_a_revoked_key_is_refused_from_the_next_request() {
    let server = Deployed::from_env();
    let Some(admin) = server.admin() else {
        eprintln!("{SKIPPED}");
        return;
    };
    let dir = TempDir::new().unwrap();
    let workspace = ThrowawayWorkspace::new(&admin, "64MiB");
    let key = workspace.create_key(&[]);
    let config = device_as(&server, dir.path(), "it-revoked", &key, "");
    passalong(dir.path(), &config)
        .arg("check")
        .assert()
        .success();

    workspace.revoke(&key);
    let checked = passalong(dir.path(), &config)
        .arg("check")
        .assert()
        .failure()
        .get_output()
        .clone();
    let out = String::from_utf8_lossy(&checked.stdout);
    let err = String::from_utf8_lossy(&checked.stderr);
    assert!(out.contains("server         FAIL  "), "{out}");
    assert!(out.contains("KEY_REVOKED"), "{out}");
    assert!(
        err.contains("ask the server's operator for a new API key"),
        "{err}"
    );
    // The workspace's other key still works: only the one key was refused.
    let other = device_as(
        &server,
        dir.path().join("other").as_path(),
        "it-other",
        &workspace.key,
        "",
    );
    passalong(&dir.path().join("other"), &other)
        .arg("check")
        .assert()
        .success();
}

#[cfg(unix)]
#[test]
#[ignore = "needs the deployed server: just test-deployed"]
fn deployed_serve_stops_after_its_key_is_revoked() {
    let server = Deployed::from_env();
    let Some(admin) = server.admin() else {
        eprintln!("{SKIPPED}");
        return;
    };
    let dir = TempDir::new().unwrap();
    let workspace = ThrowawayWorkspace::new(&admin, "64MiB");
    let key = workspace.create_key(&[]);
    let config = device_as(
        &server,
        dir.path(),
        "it-serve",
        &key,
        &quick_serve(dir.path(), false),
    );
    let mut serving = Serving::start(dir.path(), &config);
    workspace.revoke(&key);
    let file = dir.path().join("drop/after.txt");
    std::fs::write(&file, "sent after the revocation").unwrap();
    let status = serving.exit();
    let log = serving.log();
    assert!(!status.success(), "{log}");
    assert!(log.contains("serve stopped: "), "{log}");
    assert!(log.contains("KEY_REVOKED"), "{log}");
    assert!(file.exists(), "the file stays for a later serve");
    assert_eq!(admin.show(&workspace.name)["items"], 0, "nothing was sent");
}

const OLD_WORDS: &str = "abacus abdomen abdominal abide abiding ability";
const NEW_WORDS: &str = "zoom zoom zoom zoom zoom zoom";
const THIRD_WORDS: &str = "cedar cedar cedar cedar cedar cedar";

fn words(text: &str) -> Words {
    Words::parse(text).unwrap()
}

/// Argon2 settings that act within a test's patience.
fn quick() -> KdfParams {
    KdfParams {
        m_kib: 64,
        t: 1,
        p: 1,
        salt: [7; 16],
    }
}

/// The client the config in `dir` connects with; `encrypt` needs a
/// terminal, so the encryption operations are the library's.
fn connect(config: &Path) -> Arc<Client> {
    let loaded = passalong_core::config::load(config, &passalong_core::config::StdEnv).unwrap();
    passalong_https::connect(&loaded).unwrap()
}

fn admin(client: &Arc<Client>) -> HttpEncryptionAdmin {
    HttpEncryptionAdmin::new(client.clone(), Arc::new(SystemClock))
}

/// Downloads every item in `dir`'s store into a fresh folder and returns
/// `(id, sha256)` of each, sorted by id. Ids are keyed by the store's key,
/// so a rotation renames every item; [`hashes`] is what stays.
fn loaded_hashes(dir: &Path, config: &Path) -> Vec<(String, String)> {
    let out = tempfile::tempdir_in(dir).unwrap();
    let mut hashes = Vec::new();
    for id in listed_ids(dir, config) {
        let before: Vec<_> = std::fs::read_dir(out.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        passalong(dir, config)
            .args(["load", &id])
            .arg(out.path())
            .assert()
            .success();
        let written = std::fs::read_dir(out.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| !before.contains(path))
            .unwrap_or_else(|| panic!("load {id} wrote nothing"));
        hashes.push((id, sha256_hex(&std::fs::read(written).unwrap())));
    }
    hashes.sort();
    hashes
}

/// The content hashes of `items`, sorted.
fn hashes(items: &[(String, String)]) -> Vec<String> {
    let mut hashes: Vec<String> = items.iter().map(|(_, hash)| hash.clone()).collect();
    hashes.sort();
    hashes
}

#[test]
#[ignore = "needs the deployed server: just test-deployed"]
fn deployed_encryption_set_up_join_rotate_and_recover() {
    let server = Deployed::from_env();
    let Some(ops) = server.admin() else {
        eprintln!("{SKIPPED}");
        return;
    };
    let workspace = ThrowawayWorkspace::new(&ops, "64MiB");
    let name = workspace.name.clone();
    let rt = tokio::runtime::Runtime::new().unwrap();

    // Device A encrypts the empty workspace and sends two items.
    let a = TempDir::new().unwrap();
    let config_a = device_as(&server, a.path(), "it-a", &workspace.key, "");
    let client_a = connect(&config_a);
    let key_1 = rt
        .block_on(admin(&client_a).set_up(&words(OLD_WORDS), quick()))
        .unwrap();
    save_key_file(&a.path().join("store.key"), &key_1, &SystemGit::new()).unwrap();
    let mut sent = Sent::new(a.path(), &config_a);
    let text = format!("sealed at {}", nonce());
    let text_id = sent.text(&text);
    let bytes = fresh_bytes(200 * 1024);
    let report = a.path().join("sealed.bin");
    std::fs::write(&report, &bytes).unwrap();
    let file_id = sent.file(&report);
    let shown = ops.show(&name);
    assert_eq!(shown["encryption"]["state"], "sealed", "{shown}");
    assert_eq!(
        shown["encryption"]["keyId"],
        key_1.key_id().to_string(),
        "{shown}"
    );
    assert_eq!(shown["items"], 2, "{shown}");
    let originals = loaded_hashes(a.path(), &config_a);
    assert_eq!(originals.len(), 2);
    assert!(
        originals
            .iter()
            .any(|(id, hash)| id == &text_id && hash == &sha256_hex(text.as_bytes()))
    );
    assert!(
        originals
            .iter()
            .any(|(id, hash)| id == &file_id && hash == &sha256_hex(&bytes))
    );

    // Device B, with the same API key but no store key, is told to join.
    let b = TempDir::new().unwrap();
    let config_b = device_as(&server, b.path(), "it-b", &workspace.key, "");
    let checked = passalong(b.path(), &config_b)
        .arg("check")
        .assert()
        .failure()
        .get_output()
        .clone();
    let out = String::from_utf8_lossy(&checked.stdout);
    let err = String::from_utf8_lossy(&checked.stderr);
    assert!(
        out.contains("encryption     FAIL  the store is encrypted and this device has no key"),
        "{out}"
    );
    assert!(err.contains("encrypt --join"), "{err}");

    // B joins with the words and reads both items.
    let client_b = connect(&config_b);
    let joined = rt
        .block_on(admin(&client_b).join(&words(OLD_WORDS)))
        .unwrap();
    assert_eq!(joined.key_id(), key_1.key_id());
    save_key_file(&b.path().join("store.key"), &joined, &SystemGit::new()).unwrap();
    passalong(b.path(), &config_b)
        .arg("check")
        .assert()
        .success();
    passalong(b.path(), &config_b)
        .args(["cat", &text_id])
        .assert()
        .success()
        .stdout(text.clone());
    assert_eq!(loaded_hashes(b.path(), &config_b), originals);

    // A rotates: B's key is stale until it joins again with the new words.
    let key_2 = rt
        .block_on(admin(&client_a).rotate(
            &key_1,
            &words(NEW_WORDS),
            quick(),
            Arc::new(SystemClock),
        ))
        .unwrap();
    assert_ne!(key_2.key_id(), key_1.key_id());
    save_key_file(&a.path().join("store.key"), &key_2, &SystemGit::new()).unwrap();
    // A rotation renames every item (ids are keyed by the store's key) and
    // keeps every byte.
    let rotated = loaded_hashes(a.path(), &config_a);
    assert_eq!(hashes(&rotated), hashes(&originals));
    assert!(
        rotated
            .iter()
            .all(|(id, _)| !originals.iter().any(|(old, _)| old == id)),
        "{rotated:?}"
    );
    let checked = passalong(b.path(), &config_b)
        .arg("check")
        .assert()
        .failure()
        .get_output()
        .clone();
    let out = String::from_utf8_lossy(&checked.stdout);
    let err = String::from_utf8_lossy(&checked.stderr);
    assert!(out.contains("encryption     FAIL  "), "{out}");
    assert!(err.contains("encrypt --join"), "{err}");
    assert!(
        rt.block_on(admin(&client_b).join(&words(OLD_WORDS)))
            .is_err(),
        "the old words are done"
    );
    let rejoined = rt
        .block_on(admin(&client_b).join(&words(NEW_WORDS)))
        .unwrap();
    assert_eq!(rejoined.key_id(), key_2.key_id());
    save_key_file(&b.path().join("store.key"), &rejoined, &SystemGit::new()).unwrap();
    assert_eq!(loaded_hashes(b.path(), &config_b), rotated);

    // A begins another rotation, copies one item, and stops; then A
    // recovers it, finishing with the new words.
    let key_3 = DataKey::generate().unwrap();
    let header = StoreHeader::new(wrap(&key_3, &words(THIRD_WORDS), quick()).unwrap());
    let header = String::from_utf8(header.to_json()).unwrap();
    let body = format!(
        "{{\"kind\":\"rotate\",\"expectedKeyId\":\"{}\",\"newKeyId\":\"{}\",\"newHeader\":{}}}",
        key_2.key_id(),
        key_3.key_id(),
        header.trim_end()
    );
    rt.block_on(async {
        let url = client_a.url("/rewrite");
        client_a
            .send("beginRewrite", |http| {
                http.post(&url)
                    .header("content-type", "application/json")
                    .body(body.clone())
            })
            .await
            .unwrap();
        let source = HttpStore::new(
            client_a.clone(),
            Some(Sealer::new(key_2.clone())),
            Arc::new(SystemClock),
        );
        let first = source.list().await.unwrap().into_iter().next().unwrap();
        let (_, mut content) = source.get(&first.id).await.unwrap();
        let mut first_bytes = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut content, &mut first_bytes)
            .await
            .unwrap();
        let staged = HttpStore::in_partition(
            client_a.clone(),
            Some(Sealer::new(key_3.clone())),
            Arc::new(SystemClock),
            Partition::Staged,
        );
        staged
            .import(&first, Box::new(Cursor::new(first_bytes)))
            .await
            .unwrap();
    });
    let shown = ops.show(&name);
    assert!(!shown["encryption"]["rewrite"].is_null(), "open: {shown}");
    let open = rt
        .block_on(admin(&client_a).open_rewrite())
        .unwrap()
        .unwrap();
    assert!(open.mine, "{open:?}");
    assert_eq!(open.staged, 1, "{open:?}");
    // While it is open, B cannot send.
    let err = stderr(
        passalong(b.path(), &config_b)
            .args(["clipboard", "--stdin"])
            .write_stdin(format!("during a rewrite at {}", nonce())),
    );
    assert!(
        err.contains("REWRITE_IN_PROGRESS") || err.contains("re-encrypt"),
        "{err}"
    );

    let finished = rt
        .block_on(admin(&client_a).resume_rewrite(&words(THIRD_WORDS), Some(&key_2), None))
        .unwrap();
    assert_eq!(finished.key_id(), key_3.key_id());
    save_key_file(&a.path().join("store.key"), &finished, &SystemGit::new()).unwrap();
    assert!(
        rt.block_on(admin(&client_a).open_rewrite())
            .unwrap()
            .is_none()
    );
    let shown = ops.show(&name);
    assert_eq!(shown["encryption"]["state"], "sealed", "{shown}");
    assert_eq!(
        shown["encryption"]["keyId"],
        key_3.key_id().to_string(),
        "{shown}"
    );
    assert!(shown["encryption"]["rewrite"].is_null(), "{shown}");
    assert_eq!(shown["items"], 2, "{shown}");
    assert_eq!(
        hashes(&loaded_hashes(a.path(), &config_a)),
        hashes(&originals),
        "every item, as it was"
    );

    sent.ids.clear();
    drop(workspace);
}

#[test]
#[ignore = "needs the deployed server: just test-deployed"]
fn deployed_a_fresh_start_leaves_plain_items_until_pruned() {
    let server = Deployed::from_env();
    let Some(ops) = server.admin() else {
        eprintln!("{SKIPPED}");
        return;
    };
    let workspace = ThrowawayWorkspace::new(&ops, "64MiB");
    let dir = TempDir::new().unwrap();
    let config = device_as(&server, dir.path(), "it-fresh", &workspace.key, "");
    passalong(dir.path(), &config)
        .args(["clipboard", "--stdin"])
        .write_stdin(format!("from before encryption at {}", nonce()))
        .assert()
        .success();

    let client = connect(&config);
    let key = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(admin(&client).fresh_start(&words(OLD_WORDS), quick()))
        .unwrap();
    save_key_file(&dir.path().join("store.key"), &key, &SystemGit::new()).unwrap();
    let shown = ops.show(&workspace.name);
    assert_eq!(shown["encryption"]["state"], "sealed", "{shown}");
    assert_eq!(shown["plainItems"], 1, "{shown}");

    passalong(dir.path(), &config)
        .arg("list")
        .assert()
        .success()
        .stdout("no items\n")
        .stderr(predicates::str::contains(
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
        .stderr(predicates::str::contains("unencrypted").not());
    passalong(dir.path(), &config)
        .args(["prune", "--plain", "--keep", "0", "--yes"])
        .assert()
        .success()
        .stdout("no unencrypted items remain\n");
    assert_eq!(ops.show(&workspace.name)["plainItems"], 0);
}
