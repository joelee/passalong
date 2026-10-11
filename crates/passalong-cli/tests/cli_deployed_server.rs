//! The `passalong` binary against a deployed passalong-server (PLAN-00011).
//! Ignored by default; `just test-deployed` loads `.env` and runs them, one
//! at a time, because they share the deployment's test workspace. Their
//! names start with `deployed_`, so every other recipe leaves them out.

#[path = "../../passalong-https/tests/support/deployed.rs"]
mod deployed;

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use assert_cmd::Command;
use deployed::Deployed;
use passalong_core::api_key::{ApiKey, save_api_key};
use passalong_core::encryption::SystemGit;
use sha2::{Digest, Sha256};
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
