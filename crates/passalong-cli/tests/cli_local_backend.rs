//! End-to-end tests of the `passalong` binary against a `local` backend in a
//! temporary directory. Each test isolates the environment so the host's
//! real configuration is never read.

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;

use assert_cmd::Command;
use passalong_core::fs::LocalFs;
use passalong_core::model::{ItemMeta, NewItem};
use passalong_core::random::StdRandom;
use passalong_core::store::{FsStore, Store};
use passalong_core::testing::ManualClock;
use predicates::prelude::*;
use tempfile::TempDir;

struct Sandbox {
    dir: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        for sub in ["home", "work", "cfg", "store", "drop"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        Self { dir }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    /// Writes a config for the local backend and returns its path.
    fn config(&self) -> PathBuf {
        let path = self.path("cfg/config.toml");
        let text = format!(
            "[client]\ndevice_name = \"test-box\"\n\n[server]\nkind = \"local\"\n\n[server.local]\npath = \"{}\"\n\n[serve]\ndrop_folder = \"{}\"\n",
            self.path("store").display(),
            self.path("drop").display()
        );
        std::fs::write(&path, text).unwrap();
        path
    }

    /// The binary with a clean environment: HOME inside the sandbox, no
    /// passalong variables, and an empty working directory.
    fn cmd(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_passalong"));
        cmd.current_dir(self.path("work"))
            .env("HOME", self.path("home"))
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("PASSALONG_CONFIG_FILE")
            .env_remove("PASSALONG_LOG_LEVEL")
            .env_remove("PASSALONG_SSH_KEY_PASSPHRASE");
        cmd
    }

    fn with_config(&self) -> Command {
        let config = self.config();
        let mut cmd = self.cmd();
        cmd.arg("--config").arg(config);
        cmd
    }

    async fn seed(&self, texts: &[&str]) -> Vec<ItemMeta> {
        let clock = Arc::new(ManualClock::at("2026-09-12T09:53:11Z"));
        let store = FsStore::new(
            LocalFs::new(self.path("store")),
            clock.clone(),
            Box::new(StdRandom::new()),
        );
        let mut metas = Vec::new();
        for text in texts {
            let content = Box::new(Cursor::new(text.as_bytes().to_vec()));
            metas.push(
                store
                    .put(NewItem::text("seed"), content)
                    .await
                    .unwrap()
                    .meta,
            );
            clock.advance(1);
        }
        metas
    }
}

#[test]
fn help_and_version() {
    let sb = Sandbox::new();
    sb.cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage: passalong"));
    sb.cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout("passalong 0.1.0\n");
}

#[test]
fn usage_errors_exit_with_code_two() {
    let sb = Sandbox::new();
    sb.cmd().arg("bogus").assert().code(2);
    sb.cmd().assert().code(2);
}

#[test]
fn missing_config_is_a_one_line_error() {
    let sb = Sandbox::new();
    sb.cmd()
        .arg("list")
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::starts_with("error: no config file found"));
    sb.cmd()
        .args(["list", "--config", "nope.toml"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("does not exist"));
}

#[test]
fn invalid_config_names_the_key() {
    let sb = Sandbox::new();
    let path = sb.path("cfg/bad.toml");
    std::fs::write(&path, "[server]\nkind = \"local\"\n").unwrap();
    sb.cmd()
        .arg("list")
        .arg("--config")
        .arg(&path)
        .assert()
        .code(1)
        .stderr(predicate::str::contains("server.local.path"));
}

#[test]
fn list_on_an_empty_store() {
    let sb = Sandbox::new();
    sb.with_config()
        .arg("list")
        .assert()
        .success()
        .stdout("no items\n");
}

#[tokio::test]
async fn list_shows_items_newest_first_as_table_or_json() {
    let sb = Sandbox::new();
    let metas = sb.seed(&["first", "second"]).await;
    let out = sb
        .with_config()
        .arg("list")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let out = String::from_utf8(out).unwrap();
    let ids: Vec<_> = out
        .lines()
        .skip(1)
        .map(|l| l.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(ids, [metas[1].id.as_str(), metas[0].id.as_str()]);

    let out = sb
        .with_config()
        .args(["list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let items = json.as_array().unwrap();
    assert_eq!(items.len(), 2);
    for key in ["id", "kind", "size", "sha256", "created_at", "device"] {
        assert!(items[0].get(key).is_some(), "missing {key}");
    }
}

#[test]
fn config_can_come_from_dotenv_in_the_working_directory() {
    let sb = Sandbox::new();
    let config = sb.config();
    std::fs::write(
        sb.path("work/.env"),
        format!("PASSALONG_CONFIG_FILE={}\n", config.display()),
    )
    .unwrap();
    sb.cmd().arg("list").assert().success().stdout("no items\n");
}

#[test]
fn verbose_logging_goes_to_stderr_with_operation_ids() {
    let sb = Sandbox::new();
    sb.with_config()
        .args(["list", "--log-level", "verbose"])
        .assert()
        .success()
        .stdout("no items\n")
        .stderr(predicate::str::contains(" notice ").and(predicate::str::contains(" op=")));
}

#[test]
fn invalid_log_level_in_the_environment_is_reported() {
    let sb = Sandbox::new();
    sb.with_config()
        .arg("list")
        .env("PASSALONG_LOG_LEVEL", "loud")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("PASSALONG_LOG_LEVEL"));
}

fn id_line() -> impl Predicate<str> {
    predicate::str::is_match(r"^[0-9a-f]{8}-[0-9a-f]{12}\n$").unwrap()
}

#[test]
fn clipboard_from_stdin_prints_the_new_id() {
    let sb = Sandbox::new();
    let out = sb
        .with_config()
        .args(["clipboard", "--stdin"])
        .write_stdin("hi")
        .assert()
        .success()
        .stdout(id_line());
    let id = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    sb.with_config()
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains(id.trim()));
}

#[test]
fn empty_stdin_is_refused() {
    let sb = Sandbox::new();
    sb.with_config()
        .args(["clipboard", "--stdin"])
        .write_stdin("")
        .assert()
        .code(1)
        .stderr(predicate::str::ends_with("error: clipboard is empty\n"));
}

#[test]
fn file_prints_the_new_id_and_is_listed_by_name() {
    let sb = Sandbox::new();
    let path = sb.path("work/report.pdf");
    std::fs::write(&path, b"%PDF-1.7").unwrap();
    sb.with_config()
        .arg("file")
        .arg(&path)
        .assert()
        .success()
        .stdout(id_line());
    let out = sb
        .with_config()
        .args(["list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json[0]["name"], "report.pdf");
    assert_eq!(json[0]["mime"], "application/pdf");
}

#[test]
fn file_errors_name_the_path() {
    let sb = Sandbox::new();
    sb.with_config()
        .args(["file", "missing.bin"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("missing.bin"));
}

#[test]
fn file_list_load_round_trip_is_byte_identical() {
    let sb = Sandbox::new();
    let source = sb.path("work/big.bin");
    let data: Vec<u8> = (0..5 * 1024 * 1024_u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 9) as u8)
        .collect();
    std::fs::write(&source, &data).unwrap();
    sb.with_config().arg("file").arg(&source).assert().success();
    let out = sb
        .with_config()
        .args(["list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let id = json[0]["id"].as_str().unwrap().to_owned();
    let dest = sb.path("home");
    sb.with_config()
        .args(["load", &id])
        .arg(&dest)
        .assert()
        .success()
        .stdout(format!("{}\n", dest.join("big.bin").display()));
    assert_eq!(std::fs::read(dest.join("big.bin")).unwrap(), data);
    sb.with_config()
        .args(["load", &id])
        .arg(&dest)
        .assert()
        .code(1)
        .stderr(predicate::str::contains("--force"));
    sb.with_config()
        .args(["load", &id, "--force"])
        .arg(&dest)
        .assert()
        .success();
}

#[test]
fn loading_an_unknown_id_fails() {
    let sb = Sandbox::new();
    sb.with_config()
        .args(["load", "abcd"])
        .arg(sb.path("home"))
        .assert()
        .code(1)
        .stderr(predicate::str::contains("no item matches `abcd`"));
}

/// Runs the real `serve` process, drops a file, and stops it with SIGTERM as
/// systemd would. On Linux the display variables are removed, so no
/// clipboard is reachable and `serve` watches only the drop folder. macOS
/// always provides the system pasteboard, so there `serve` may also send
/// whatever text is on it into this test's temporary store.
#[cfg(unix)]
#[test]
fn serve_sends_dropped_files_and_stops_cleanly_on_sigterm() {
    use std::io::Read;
    use std::process::{Command as Process, Stdio};
    use std::time::{Duration, Instant};

    let sb = Sandbox::new();
    let config = sb.config();
    let mut child = Process::new(env!("CARGO_BIN_EXE_passalong"))
        .current_dir(sb.path("work"))
        .env("HOME", sb.path("home"))
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("PASSALONG_CONFIG_FILE")
        .env_remove("PASSALONG_LOG_LEVEL")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .args(["serve", "--config"])
        .arg(&config)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::fs::write(sb.path("drop/hello.txt"), b"from the drop folder").unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !sb.path("drop/sent/hello.txt").exists() {
        assert!(Instant::now() < deadline, "the file was never sent");
        assert!(child.try_wait().unwrap().is_none(), "serve exited early");
        std::thread::sleep(Duration::from_millis(50));
    }
    let killed = Process::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "serve did not stop after SIGTERM"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(status.success(), "exit {status:?}\n{stderr}");
    assert!(stderr.contains("serve stopped"), "{stderr}");
    #[cfg(target_os = "linux")]
    assert!(stderr.contains("clipboard unavailable"), "{stderr}");
    let out = sb
        .with_config()
        .args(["list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let names: Vec<&str> = json
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["name"].as_str())
        .collect();
    assert_eq!(names, ["hello.txt"], "exactly the dropped file was stored");
}

#[tokio::test]
async fn delete_removes_named_items_and_rejects_unknown_ones() {
    let sb = Sandbox::new();
    let metas = sb.seed(&["first", "second"]).await;
    sb.with_config()
        .args(["delete", metas[0].id.as_str()])
        .assert()
        .success()
        .stdout(format!("{}\n", metas[0].id));
    let out = sb
        .with_config()
        .args(["list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json.as_array().unwrap().len(), 1);
    assert_eq!(json[0]["id"], metas[1].id.as_str());
    sb.with_config()
        .args(["delete", "ffff"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("no item matches `ffff`"));
}
