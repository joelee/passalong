//! A real passalong-server per test: its own `HOME`, a self-signed pair for
//! `127.0.0.1`, a workspace, keys, and `serve` on a free port.
//!
//! `just test-https` builds the server at the pinned commit and names the
//! binary in `PASSALONG_SERVER_BIN`. The server is AGPL-3.0-or-later; it is
//! only run here.

#![allow(dead_code)]

use std::io::Read as _;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use passalong_core::api_key::ApiKey;
use passalong_core::config::{self, Config, TlsPin};
use passalong_core::testing::MapEnv;
use tempfile::TempDir;

/// The workspace every test server has.
pub const WORKSPACE: &str = "home";

/// A running server. Dropping it stops the server.
pub struct TestServer {
    home: TempDir,
    bin: PathBuf,
    child: Child,
    /// `https://127.0.0.1:<port>`.
    pub url: String,
    /// The pin of its certificate.
    pub pin: TlsPin,
    /// A read-write key of [`WORKSPACE`] that never expires.
    pub key: ApiKey,
}

/// Settings of a test server's configuration file.
#[derive(Debug, Clone)]
pub struct Settings {
    /// `rewrite.lease_secs`.
    pub lease_secs: u64,
    /// `limits.max_item_bytes`.
    pub max_item_bytes: Option<u64>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lease_secs: 600,
            max_item_bytes: None,
        }
    }
}

impl TestServer {
    /// A server with default settings.
    pub fn start() -> Self {
        Self::start_with(Settings::default())
    }

    /// A server with `settings`.
    pub fn start_with(settings: Settings) -> Self {
        let bin = PathBuf::from(std::env::var("PASSALONG_SERVER_BIN").unwrap_or_else(|_| {
            panic!("PASSALONG_SERVER_BIN is not set: run these tests with `just test-https`")
        }));
        let home = TempDir::new().unwrap();
        run(&bin, home.path(), &["init"]);
        let tls = run(
            &bin,
            home.path(),
            &[
                "tls",
                "self-signed",
                "--host",
                "localhost",
                "--ip",
                "127.0.0.1",
            ],
        );
        let pin = tls
            .split(|c: char| c == '"' || c.is_whitespace())
            .find(|word| word.starts_with("sha256/"))
            .and_then(|word| TlsPin::parse(word).ok())
            .unwrap_or_else(|| panic!("no pin in: {tls}"));

        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let config = home.path().join(".config/passalong-server/config.toml");
        let text = std::fs::read_to_string(&config).unwrap();
        let text = text
            .lines()
            .map(|line| match line.split_once(" = ") {
                Some(("address", _)) => format!("address = \"127.0.0.1:{port}\""),
                // Tests send wrong keys on purpose.
                Some(("auth_failures_per_minute", _)) => "auth_failures_per_minute = 0".to_owned(),
                Some(("lease_secs", _)) => format!("lease_secs = {}", settings.lease_secs),
                Some(("max_item_bytes", _)) => match settings.max_item_bytes {
                    Some(bytes) => format!("max_item_bytes = \"{bytes}\""),
                    None => line.to_owned(),
                },
                _ => line.to_owned(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&config, text).unwrap();

        run(&bin, home.path(), &["workspace", "create", WORKSPACE]);
        let log = std::fs::File::create(home.path().join("serve.log")).unwrap();
        let child = command(&bin, home.path())
            .arg("serve")
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .unwrap();
        let mut server = Self {
            key: ApiKey::parse("pal_0_0").unwrap(),
            home,
            bin,
            child,
            url: format!("https://127.0.0.1:{port}"),
            pin,
        };
        server.wait_listening(port);
        server.key = server.create_key(&["--never"]);
        server
    }

    fn wait_listening(&mut self, port: u16) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            if let Some(status) = self.child.try_wait().unwrap() {
                panic!("the server stopped ({status}):\n{}", self.log());
            }
            assert!(
                Instant::now() < deadline,
                "the server never listened:\n{}",
                self.log()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Runs a server command, such as `key revoke <id>`, and returns what
    /// it printed.
    pub fn run(&self, args: &[&str]) -> String {
        run(&self.bin, self.home.path(), args)
    }

    /// A new key of [`WORKSPACE`], made with `extra` options such as
    /// `--read-only` or `--expires 7d`.
    pub fn create_key(&self, extra: &[&str]) -> ApiKey {
        let mut args = vec![
            "key",
            "create",
            "--workspace",
            WORKSPACE,
            "--label",
            "test",
            "--json",
        ];
        args.extend_from_slice(extra);
        let out = self.run(&args);
        let json: serde_json::Value = serde_json::from_str(&out).unwrap();
        ApiKey::parse(json["key"].as_str().unwrap()).unwrap()
    }

    /// The client configuration of a device in `dir` that reaches this
    /// server with `pin`, keeping its keys there.
    pub fn config(&self, dir: &Path, pin: Option<&str>) -> Config {
        let pin = pin.map_or_else(String::new, |pin| format!("tls_pin = \"{pin}\"\n"));
        let text = format!(
            "[client]\ndevice_name = \"it\"\nkey_file = '{}'\n[server]\nkind = \"https\"\n[server.https]\nurl = \"{}\"\n{pin}api_key_file = '{}'\n",
            dir.join("store.key").display(),
            self.url,
            dir.join("api.key").display(),
        );
        config::parse(
            &text,
            &dir.join("config.toml"),
            &MapEnv::new().with("HOME", dir.to_str().unwrap()),
        )
        .unwrap()
    }

    /// The configuration with this server's own pin.
    pub fn pinned(&self, dir: &Path) -> Config {
        self.config(dir, Some(&self.pin.to_string()))
    }

    /// What the server logged so far.
    pub fn log(&self) -> String {
        let mut text = String::new();
        if let Ok(mut file) = std::fs::File::open(self.home.path().join("serve.log")) {
            let _ = file.read_to_string(&mut text);
        }
        text
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if std::thread::panicking() {
            eprintln!("--- server log ---\n{}", self.log());
        }
    }
}

fn command(bin: &Path, home: &Path) -> Command {
    let mut command = Command::new(bin);
    command
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("PASSALONG_SERVER_CONFIG_FILE")
        .env_remove("PASSALONG_SERVER_LOG_LEVEL")
        .stdin(Stdio::null());
    command
}

fn run(bin: &Path, home: &Path, args: &[&str]) -> String {
    let out = command(bin, home).args(args).output().unwrap();
    assert!(
        out.status.success(),
        "passalong-server {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}
