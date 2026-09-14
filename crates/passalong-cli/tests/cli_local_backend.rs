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
            .env_remove("XDG_STATE_HOME")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
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
        .stdout("passalong 0.1.5\n");
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

#[tokio::test]
async fn cat_prints_text_and_file_items_exactly() {
    let sb = Sandbox::new();
    let metas = sb.seed(&["first line\nsecond line"]).await;
    sb.with_config()
        .args(["cat", metas[0].id.as_str()])
        .assert()
        .success()
        .stdout("first line\nsecond line");
    let source = sb.path("work/data.bin");
    let data: Vec<u8> = (0..300_000_u32).map(|i| (i % 253) as u8).collect();
    std::fs::write(&source, &data).unwrap();
    let out = sb.with_config().arg("file").arg(&source).assert().success();
    let id = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let printed = sb
        .with_config()
        .args(["cat", id.trim()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(printed, data);
}

#[tokio::test]
async fn cat_adds_nothing_to_stderr_unless_verbose() {
    let sb = Sandbox::new();
    let metas = sb.seed(&["hello there"]).await;
    sb.with_config()
        .args(["cat", metas[0].id.as_str()])
        .assert()
        .success()
        .stdout("hello there")
        .stderr("");
    sb.with_config()
        .args(["cat", metas[0].id.as_str(), "--log-level", "verbose"])
        .assert()
        .success()
        .stdout("hello there")
        .stderr(predicate::str::contains("item printed"));
}

#[tokio::test]
async fn get_prints_an_items_metadata_as_fields_or_json() {
    let sb = Sandbox::new();
    let metas = sb.seed(&["hello"]).await;
    let id = metas[0].id.as_str();
    sb.with_config()
        .args(["get", &id[9..15]])
        .assert()
        .success()
        .stdout(
            predicate::str::starts_with(format!("id:      {id}\n"))
                .and(predicate::str::contains("preview: hello\n")),
        )
        .stderr("");
    let out = sb
        .with_config()
        .args(["get", id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["id"], id);
    sb.with_config()
        .args(["--quiet", "get", id])
        .assert()
        .success()
        .stdout("")
        .stderr("");
    sb.with_config()
        .args(["get", "ffff"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("no item matches `ffff`"));
}

#[tokio::test]
async fn check_reports_each_step_and_fails_at_the_first_problem() {
    let sb = Sandbox::new();
    sb.seed(&["one"]).await;
    sb.with_config().arg("check").assert().success().stdout(
        predicate::str::contains("config         ok    ")
            .and(predicate::str::contains("server         ok    local "))
            .and(predicate::str::contains("storage read   ok    1 item\n"))
            .and(predicate::str::contains(
                "storage write  ok    wrote and removed a 128-byte probe in ",
            ))
            .and(predicate::str::ends_with(
                "serve          off   not running\n",
            )),
    );
    sb.with_config()
        .args(["--quiet", "check"])
        .assert()
        .success()
        .stdout("")
        .stderr("");
    sb.cmd()
        .arg("check")
        .assert()
        .code(1)
        .stdout(
            predicate::str::contains("config         FAIL  ")
                .and(predicate::str::contains("storage write  skip\n"))
                .and(predicate::str::ends_with(
                    "serve          off   not running\n",
                )),
        )
        .stderr(predicate::str::contains("error: check failed: config"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let tmp = sb.path("store/tmp");
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o555)).unwrap();
        // Root ignores permissions, and then there is nothing to test.
        let enforced = std::fs::create_dir(tmp.join("root-test")).is_err();
        if enforced {
            sb.with_config()
                .arg("check")
                .assert()
                .code(1)
                .stdout(predicate::str::contains("storage write  FAIL  "))
                .stderr(predicate::str::contains(
                    "error: check failed: storage write",
                ));
        }
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            std::fs::read_dir(&tmp)
                .unwrap()
                .filter(|entry| entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("probe-"))
                .count(),
            0,
            "no probe is left behind"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn service_install_writes_a_systemd_unit_and_service_remove_removes_it() {
    use std::os::unix::fs::PermissionsExt;
    let sb = Sandbox::new();
    let bin = sb.path("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let fake = bin.join("systemctl");
    std::fs::write(&fake, "#!/bin/sh\necho \"$*\" >> \"$FAKE_SYSTEMCTL_LOG\"\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let log = sb.path("systemctl.log");
    let config = sb.config();
    let install = |args: &[&str]| {
        let mut cmd = sb.cmd();
        cmd.env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .env("XDG_CONFIG_HOME", sb.path("xdg"))
        .env("FAKE_SYSTEMCTL_LOG", &log)
        .arg("--config")
        .arg(&config)
        .args(args);
        cmd
    };
    let unit = sb.path("xdg/systemd/user/passalong-serve.service");
    install(&["service-install"])
        .assert()
        .success()
        .stdout(format!(
            "wrote {}\nenabled and started passalong-serve.service\n",
            unit.display()
        ));
    let text = std::fs::read_to_string(&unit).unwrap();
    let exe = std::fs::canonicalize(env!("CARGO_BIN_EXE_passalong")).unwrap();
    assert!(
        text.contains(&format!(
            "ExecStart={} --config {} serve\n",
            exe.display(),
            config.display()
        )),
        "{text}"
    );
    assert!(text.contains("WorkingDirectory=%h\n"), "{text}");
    assert_eq!(
        std::fs::read_to_string(&log).unwrap(),
        "--user daemon-reload\n--user enable --now passalong-serve.service\n"
    );
    install(&["service-install"])
        .assert()
        .success()
        .stdout(predicate::str::ends_with(
            "is already installed and unchanged\n",
        ));
    install(&["--quiet", "service-remove"])
        .assert()
        .success()
        .stdout("")
        .stderr("");
    assert!(!unit.exists());
    assert_eq!(
        std::fs::read_to_string(&log).unwrap(),
        "--user daemon-reload\n--user enable --now passalong-serve.service\n--user disable --now passalong-serve.service\n--user daemon-reload\n"
    );
}

#[tokio::test]
async fn choose_needs_a_terminal() {
    let sb = Sandbox::new();
    sb.seed(&["one"]).await;
    sb.with_config()
        .arg("choose")
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::ends_with(
            "error: choose needs a terminal\n",
        ));
}

#[tokio::test]
async fn quiet_prints_nothing_but_errors_and_cat_output() {
    let sb = Sandbox::new();
    let metas = sb.seed(&["one", "two", "three"]).await;
    let quiet = |args: &[&str]| {
        let mut cmd = sb.with_config();
        cmd.arg("--quiet").args(args);
        cmd
    };
    quiet(&["list"]).assert().success().stdout("").stderr("");
    quiet(&["clipboard", "--stdin"])
        .write_stdin("hi")
        .assert()
        .success()
        .stdout("")
        .stderr("");
    let source = sb.path("work/a.txt");
    std::fs::write(&source, "file body").unwrap();
    quiet(&["file", source.to_str().unwrap()])
        .assert()
        .success()
        .stdout("")
        .stderr("");
    let dest = sb.path("work/out.txt");
    quiet(&["load", metas[0].id.as_str(), dest.to_str().unwrap()])
        .assert()
        .success()
        .stdout("")
        .stderr("");
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "one");
    quiet(&["cat", metas[1].id.as_str()])
        .assert()
        .success()
        .stdout("two")
        .stderr("");
    quiet(&["delete", metas[0].id.as_str()])
        .assert()
        .success()
        .stdout("")
        .stderr("");
    quiet(&["prune", "--keep", "1", "--yes"])
        .assert()
        .success()
        .stdout("")
        .stderr("");
    quiet(&["serve", "--status"])
        .assert()
        .code(3)
        .stdout("")
        .stderr("");
    quiet(&["delete", "ffff"])
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::ends_with("error: no item matches `ffff`\n"));
    // An explicit level wins over --quiet, from the flag or the environment.
    quiet(&["--log-level", "info", "clipboard", "--stdin"])
        .write_stdin("flag")
        .assert()
        .success()
        .stdout("")
        .stderr(predicate::str::contains("item stored"));
    quiet(&["clipboard", "--stdin"])
        .env("PASSALONG_LOG_LEVEL", "info")
        .write_stdin("env")
        .assert()
        .success()
        .stdout("")
        .stderr(predicate::str::contains("item stored"));
}

#[test]
fn load_without_a_destination_downloads_files_into_downloads() {
    let sb = Sandbox::new();
    let source = sb.path("work/notes.pdf");
    std::fs::write(&source, b"%PDF-1.7").unwrap();
    let out = sb.with_config().arg("file").arg(&source).assert().success();
    let id = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let target = sb.path("home/Downloads/notes.pdf");
    sb.with_config()
        .args(["load", id.trim()])
        .assert()
        .success()
        .stdout(format!("{}\n", target.display()));
    assert_eq!(std::fs::read(&target).unwrap(), b"%PDF-1.7");
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
        .env_remove("XDG_STATE_HOME")
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

#[tokio::test]
async fn prune_lists_confirms_and_deletes() {
    let sb = Sandbox::new();
    let metas = sb.seed(&["one", "two", "three"]).await;
    let count = |sb: &Sandbox| {
        let out = sb
            .with_config()
            .args(["list", "--json"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        serde_json::from_slice::<serde_json::Value>(&out)
            .unwrap()
            .as_array()
            .unwrap()
            .len()
    };
    sb.with_config()
        .args(["prune", "--keep", "1", "--dry-run"])
        .assert()
        .success()
        .stdout(
            predicate::str::starts_with("2 items to delete:")
                .and(predicate::str::ends_with("dry run: nothing deleted\n")),
        );
    assert_eq!(count(&sb), 3);
    sb.with_config()
        .args(["prune", "--keep", "1"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("--yes"));
    assert_eq!(count(&sb), 3);
    sb.with_config()
        .args(["prune", "--keep", "1", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::ends_with("deleted 2 items\n"));
    let out = sb
        .with_config()
        .args(["list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json[0]["id"], metas[2].id.as_str());
    sb.with_config()
        .args(["prune"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("--older-than"));
}

#[test]
fn init_writes_a_config_offline_and_refuses_to_overwrite_it() {
    let sb = Sandbox::new();
    let target = sb.path("cfg/new.toml");
    let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIF2M9DqIpW9GMebpvjNg+bobwAbQKRBqPVMatyvyI4gq";
    let init = |sb: &Sandbox| {
        let mut cmd = sb.cmd();
        cmd.arg("--config").arg(&target).args([
            "init",
            "--host",
            "127.0.0.1",
            "--host-key",
            key,
            "--yes",
            "--no-test",
        ]);
        cmd
    };
    init(&sb)
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "wrote {}",
            target.display()
        )));
    let written = std::fs::read_to_string(&target).unwrap();
    assert!(
        written.contains(key) && written.contains("host = \"127.0.0.1\""),
        "{written}"
    );
    init(&sb)
        .assert()
        .code(1)
        .stderr(predicate::str::contains("--force"));
    sb.cmd()
        .args(["init", "--host", "127.0.0.1", "--yes"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("--host-key or --fingerprint"));
}

/// Stops a background `serve` left running by a failed test.
#[cfg(unix)]
struct DaemonGuard(Option<u32>);

#[cfg(unix)]
impl Drop for DaemonGuard {
    fn drop(&mut self) {
        if let Some(pid) = self.0 {
            let _ = std::process::Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .status();
        }
    }
}

#[cfg(unix)]
#[test]
fn serve_daemon_starts_reports_refuses_a_second_copy_and_stops() {
    use std::time::{Duration, Instant};
    let sb = Sandbox::new();
    let started = sb
        .with_config()
        .args(["serve", "--daemon"])
        .timeout(Duration::from_secs(20))
        .assert()
        .success();
    let out = String::from_utf8(started.get_output().stdout.clone()).unwrap();
    assert!(out.starts_with("serve started (pid "), "{out}");
    let pid: u32 = out["serve started (pid ".len()..]
        .split(',')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let _guard = DaemonGuard(Some(pid));
    let log = if cfg!(target_os = "macos") {
        sb.path("home/Library/Logs/passalong/serve.log")
    } else {
        sb.path("home/.local/state/passalong/serve.log")
    };
    assert!(out.contains(&log.display().to_string()), "{out}");

    sb.with_config()
        .args(["serve", "--status"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with(format!("running (pid {pid}")));
    sb.with_config()
        .arg("check")
        .assert()
        .success()
        .stdout(predicate::str::ends_with(format!(
            "serve          ok    running (pid {pid})\n"
        )));
    let busy = format!("serve is already running (pid {pid})");
    sb.with_config()
        .args(["serve", "--daemon"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(busy.clone()));
    sb.with_config()
        .arg("serve")
        .assert()
        .code(1)
        .stderr(predicate::str::contains(busy));

    std::fs::write(sb.path("drop/daemon.txt"), b"sent by the daemon").unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !sb.path("drop/sent/daemon.txt").exists() {
        assert!(Instant::now() < deadline, "the daemon never sent the file");
        std::thread::sleep(Duration::from_millis(50));
    }

    sb.with_config()
        .args(["serve", "--stop"])
        .timeout(Duration::from_secs(20))
        .assert()
        .success()
        .stdout("stopped\n");
    sb.with_config()
        .args(["serve", "--status"])
        .assert()
        .code(3)
        .stdout("not running\n");
    sb.with_config()
        .args(["serve", "--stop"])
        .assert()
        .success()
        .stdout("not running\n");
    let logged = std::fs::read_to_string(&log).unwrap();
    assert!(logged.contains("serve stopped"), "{logged}");
    let pid_file = if cfg!(target_os = "macos") {
        sb.path("home/Library/Application Support/passalong/serve.pid")
    } else {
        sb.path("home/.local/state/passalong/serve.pid")
    };
    assert!(!pid_file.exists(), "--stop leaves no pid file");
}

#[cfg(unix)]
#[test]
fn serve_daemon_pulls_files_sent_by_another_device() {
    use std::time::{Duration, Instant};
    let sb = Sandbox::new();
    std::fs::create_dir_all(sb.path("home/dl")).unwrap();
    let config = sb.path("cfg/pull.toml");
    std::fs::write(
        &config,
        format!(
            "[client]\ndevice_name = \"laptop\"\ndownload_dir = \"{}\"\n\n[server]\nkind = \"local\"\n\n[server.local]\npath = \"{}\"\n\n[serve]\ndrop_folder = \"{}\"\npull = true\npull_interval_ms = 1000\n",
            sb.path("home/dl").display(),
            sb.path("store").display(),
            sb.path("drop").display()
        ),
    )
    .unwrap();
    let started = sb
        .cmd()
        .arg("--config")
        .arg(&config)
        .args(["serve", "--daemon"])
        .timeout(Duration::from_secs(20))
        .assert()
        .success();
    let out = String::from_utf8(started.get_output().stdout.clone()).unwrap();
    let pid: u32 = out["serve started (pid ".len()..]
        .split(',')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let _guard = DaemonGuard(Some(pid));

    // Another device sharing the store (the sandbox config is "test-box").
    let source = sb.path("work/from-phone.txt");
    std::fs::write(&source, b"sent by the phone").unwrap();
    sb.with_config().arg("file").arg(&source).assert().success();
    let target = sb.path("home/dl/from-phone.txt");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !target.exists() {
        assert!(
            Instant::now() < deadline,
            "the daemon never pulled the file"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(std::fs::read(&target).unwrap(), b"sent by the phone");
    sb.cmd()
        .arg("--config")
        .arg(&config)
        .args(["serve", "--stop"])
        .timeout(Duration::from_secs(20))
        .assert()
        .success();
}

#[cfg(unix)]
#[test]
fn serve_daemon_reports_start_up_failures() {
    let sb = Sandbox::new();
    let bad = sb.path("cfg/bad.toml");
    std::fs::write(&bad, "[server]\nkind = \"local\"\n").unwrap();
    sb.cmd()
        .arg("--config")
        .arg(&bad)
        .args(["serve", "--daemon"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("server.local.path"));

    let unreachable = sb.path("cfg/unreachable.toml");
    std::fs::write(
        &unreachable,
        "[server]\nkind = \"ssh\"\n[server.ssh]\nhost = \"127.0.0.1\"\nport = 1\nuser = \"u\"\nhost_key = \"ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIF2M9DqIpW9GMebpvjNg+bobwAbQKRBqPVMatyvyI4gq\"\nidentity_file = \"/nonexistent/key\"\nremote_path = \"/r\"\n",
    )
    .unwrap();
    sb.cmd()
        .arg("--config")
        .arg(&unreachable)
        .args(["serve", "--daemon"])
        .timeout(std::time::Duration::from_secs(20))
        .assert()
        .code(1)
        .stderr(
            predicate::str::contains("stopped during start-up")
                .and(predicate::str::contains("cannot load the SSH key")),
        );
    sb.cmd()
        .arg("--config")
        .arg(&unreachable)
        .args(["serve", "--status"])
        .assert()
        .code(3);
}

/// Linux: after `load` exits, its text is still on the clipboard. Needs a
/// desktop session; CI runs it under Xvfb. Never run it on a machine whose
/// clipboard you care about: it replaces the clipboard's content.
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "needs a desktop session with a clipboard"]
async fn desktop_loaded_text_survives_load_exiting() {
    use passalong_core::clipboard::{ArboardClipboard, Clipboard};
    let sb = Sandbox::new();
    let metas = sb.seed(&["held after load exits"]).await;
    let mut cmd = sb.with_config();
    for var in ["DISPLAY", "WAYLAND_DISPLAY", "XDG_RUNTIME_DIR"] {
        if let Ok(value) = std::env::var(var) {
            cmd.env(var, value);
        }
    }
    cmd.args(["load", metas[0].id.as_str()]).assert().success();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let mut clipboard = ArboardClipboard::new().unwrap();
    assert_eq!(
        clipboard.read_text().unwrap().as_deref(),
        Some("held after load exits")
    );
    clipboard.write_text("released").unwrap();
}

/// Like `desktop_loaded_text_survives_load_exiting`, for a clipboard image.
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "needs a desktop session with a clipboard"]
async fn desktop_loaded_image_survives_load_exiting() {
    use passalong_core::clipboard::{ArboardClipboard, Clipboard, RgbaImage, encode_png};
    let sb = Sandbox::new();
    let rgba: Vec<u8> = (0..4 * 4 * 4)
        .map(|i| {
            if i % 4 == 3 {
                255
            } else {
                (i * 23 % 256) as u8
            }
        })
        .collect();
    let image = RgbaImage::new(4, 4, rgba).unwrap();
    let store = FsStore::new(
        LocalFs::new(sb.path("store")),
        Arc::new(ManualClock::at("2026-09-12T09:53:11Z")),
        Box::new(StdRandom::new()),
    );
    let png = encode_png(&image).unwrap();
    let meta = store
        .put(
            NewItem::clipboard_image("seed", "2026-09-12T09:53:11Z".parse().unwrap()),
            Box::new(Cursor::new(png)),
        )
        .await
        .unwrap()
        .meta;
    let mut cmd = sb.with_config();
    for var in ["DISPLAY", "WAYLAND_DISPLAY", "XDG_RUNTIME_DIR"] {
        if let Ok(value) = std::env::var(var) {
            cmd.env(var, value);
        }
    }
    cmd.args(["load", meta.id.as_str()]).assert().success();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let mut clipboard = ArboardClipboard::new().unwrap();
    assert_eq!(clipboard.read_image().unwrap(), Some(image));
    clipboard.write_text("released").unwrap();
}
