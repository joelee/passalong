//! Running the `passalong` binary's `serve` in a test of the `passalong`
//! package: shared by its tests against a server of their own
//! (`cli_https_backend.rs`) and against a deployed one
//! (`cli_deployed_server.rs`). Unix only: `serve` is stopped by killing it.

#![allow(dead_code)]

use std::path::Path;

/// `[serve]` settings that act within a test's patience.
#[cfg(unix)]
pub fn quick_serve(dir: &Path, pull: bool) -> String {
    std::fs::create_dir_all(dir.join("drop")).unwrap();
    std::fs::create_dir_all(dir.join("downloads")).unwrap();
    format!(
        "\n[serve]\ndrop_folder = '{}'\nfile_stable_wait_ms = 100\npull = {pull}\npull_interval_ms = 1000\n",
        dir.join("drop").display()
    )
}

/// `passalong serve` running in the background, stopped when dropped.
#[cfg(unix)]
pub struct Serving {
    child: std::process::Child,
    stderr: std::path::PathBuf,
}

#[cfg(unix)]
impl Serving {
    pub fn start(dir: &Path, config: &Path) -> Self {
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

    pub fn log(&self) -> String {
        std::fs::read_to_string(&self.stderr).unwrap_or_default()
    }

    /// Waits up to 20 s for `text` in the log, while serve runs.
    pub fn wait_for(&mut self, text: &str) {
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
    pub fn exit(&mut self) -> std::process::ExitStatus {
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

    pub fn running(&mut self) -> bool {
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
pub fn wait_for_file(path: &Path) {
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
