//! `passalong service-install` and `passalong service-remove`: install
//! `serve` as a service that starts at login, and remove it again.

use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use passalong_core::config::EnvProvider;

use crate::cli::ServiceInstallArgs;
use crate::daemon::{Os, StatePaths, Status};
use crate::service::{self, LAUNCHD_LABEL, SYSTEMD_UNIT, ServiceManager};

/// The error on platforms without a supported service manager.
pub const UNSUPPORTED: &str =
    "service-install and service-remove support Linux (systemd) and macOS (launchd) only";

/// The service manager to install for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// A systemd user unit, on Linux.
    Systemd,
    /// A launchd agent, on macOS, in the GUI session of user `uid`.
    Launchd {
        /// The user's numeric id.
        uid: u32,
    },
}

/// The platform this binary was built for. For launchd, the user is the
/// owner of `home`.
pub fn current_platform(home: &Path) -> anyhow::Result<Platform> {
    if cfg!(target_os = "linux") {
        Ok(Platform::Systemd)
    } else if cfg!(target_os = "macos") {
        Ok(Platform::Launchd {
            uid: owner_uid(home)?,
        })
    } else {
        anyhow::bail!(UNSUPPORTED)
    }
}

#[cfg(unix)]
fn owner_uid(path: &Path) -> anyhow::Result<u32> {
    use std::os::unix::fs::MetadataExt as _;
    let meta =
        std::fs::metadata(path).with_context(|| format!("cannot read {}", path.display()))?;
    Ok(meta.uid())
}

#[cfg(not(unix))]
fn owner_uid(_path: &Path) -> anyhow::Result<u32> {
    anyhow::bail!(UNSUPPORTED)
}

/// What `service-install` works with besides its options.
pub struct Install<'a> {
    /// Which service manager to use.
    pub platform: Platform,
    /// Environment: `HOME`, and `XDG_CONFIG_HOME` for systemd.
    pub env: &'a dyn EnvProvider,
    /// The absolute path of the binary the unit runs.
    pub exe: &'a Path,
    /// The absolute config path to pass with `--config`, if one was given.
    pub config: Option<&'a Path>,
    /// Whether a `serve` holds the pid lock now.
    pub serve: Status,
    /// Runs `systemctl` or `launchctl`.
    pub manager: &'a mut dyn ServiceManager,
}

/// Writes the unit, then enables and starts it unless `--no-start`.
pub fn run(
    args: &ServiceInstallArgs,
    install: Install<'_>,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let Install {
        platform,
        env,
        exe,
        config,
        serve,
        manager,
    } = install;
    let home = home_dir(env)?;
    let path = unit_path(platform, env, &home);
    let mut argv = vec![exe.display().to_string()];
    if let Some(config) = config {
        argv.push("--config".to_owned());
        argv.push(config.display().to_string());
    }
    argv.push("serve".to_owned());
    let log = StatePaths::resolve(env, Os::Mac)
        .context("cannot find the home directory: set HOME")?
        .log;
    let text = match platform {
        Platform::Systemd => service::systemd_unit(&service::systemd_exec_start(exe, config)),
        Platform::Launchd { .. } => service::launchd_plist(
            &argv,
            &home.display().to_string(),
            &log.display().to_string(),
        ),
    };
    let existing = match std::fs::read_to_string(&path) {
        Ok(existing) => Some(existing),
        Err(err) if err.kind() == ErrorKind::NotFound => None,
        Err(err) => return Err(err).with_context(|| format!("cannot read {}", path.display())),
    };
    if !args.force {
        match existing.as_deref() {
            Some(existing) if existing == text => {
                writeln!(out, "{} is already installed and unchanged", path.display())?;
                return Ok(());
            }
            Some(_) => anyhow::bail!(
                "{} exists and differs; use --force to replace it",
                path.display()
            ),
            None => {}
        }
    }
    if !args.no_start
        && let Status::Running { pid, .. } = serve
    {
        let pid = pid.map_or_else(String::new, |pid| format!(" (pid {pid})"));
        anyhow::bail!(
            "serve is already running{pid}; stop it first with `passalong serve --stop`, or with `passalong service-remove` if a service runs it"
        );
    }
    write_atomically(&path, &text)?;
    writeln!(out, "wrote {}", path.display())?;
    tracing::info!(path = %path.display(), "service unit written");
    match platform {
        Platform::Systemd if args.no_start => {
            writeln!(
                out,
                "start it with: systemctl --user enable --now {SYSTEMD_UNIT}"
            )?;
        }
        Platform::Systemd => {
            call(manager, "systemctl", &["--user", "daemon-reload"])
                .and_then(|()| {
                    call(
                        manager,
                        "systemctl",
                        &["--user", "enable", "--now", SYSTEMD_UNIT],
                    )
                })
                .with_context(|| left_in_place(&path))?;
            writeln!(out, "enabled and started {SYSTEMD_UNIT}")?;
        }
        Platform::Launchd { uid } => {
            // launchd does not create the log file's folder.
            if let Some(dir) = log.parent() {
                std::fs::create_dir_all(dir)
                    .with_context(|| format!("cannot create {}", dir.display()))?;
            }
            let bootstrap = [
                "bootstrap".to_owned(),
                format!("gui/{uid}"),
                path.display().to_string(),
            ];
            if args.no_start {
                writeln!(out, "start it with: launchctl {}", bootstrap.join(" "))?;
                return Ok(());
            }
            if existing.is_some()
                && let Err(err) = call(manager, "launchctl", &["bootout", &agent(uid)])
            {
                tracing::debug!(error = %format!("{err:#}"), "the old agent was not loaded");
            }
            let bootstrap: Vec<&str> = bootstrap.iter().map(String::as_str).collect();
            call(manager, "launchctl", &bootstrap).with_context(|| left_in_place(&path))?;
            writeln!(out, "loaded {LAUNCHD_LABEL}")?;
        }
    }
    Ok(())
}

/// Stops, disables, and removes the installed unit, or says it is not
/// installed.
pub fn remove(
    platform: Platform,
    env: &dyn EnvProvider,
    manager: &mut dyn ServiceManager,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let path = unit_path(platform, env, &home_dir(env)?);
    remove_unit(platform, &path, manager, out)
}

fn home_dir(env: &dyn EnvProvider) -> anyhow::Result<PathBuf> {
    env.var("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .context("cannot find the home directory: set HOME")
}

fn remove_unit(
    platform: Platform,
    path: &Path,
    manager: &mut dyn ServiceManager,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    if !path.exists() {
        writeln!(out, "not installed: no {}", path.display())?;
        return Ok(());
    }
    match platform {
        Platform::Systemd => {
            call(
                manager,
                "systemctl",
                &["--user", "disable", "--now", SYSTEMD_UNIT],
            )
            .with_context(|| format!("{} is left in place", path.display()))?;
            remove_file(path)?;
            call(manager, "systemctl", &["--user", "daemon-reload"])?;
        }
        Platform::Launchd { uid } => {
            // An agent that is not loaded cannot be booted out, and that is
            // fine when removing it.
            if let Err(err) = call(manager, "launchctl", &["bootout", &agent(uid)]) {
                tracing::warn!(error = %format!("{err:#}"), "the agent was not loaded");
            }
            remove_file(path)?;
        }
    }
    writeln!(out, "removed {}", path.display())?;
    Ok(())
}

/// Where the unit goes: the systemd user unit folder under
/// `${XDG_CONFIG_HOME:-~/.config}`, or `~/Library/LaunchAgents`.
fn unit_path(platform: Platform, env: &dyn EnvProvider, home: &Path) -> PathBuf {
    match platform {
        Platform::Systemd => env
            .var("XDG_CONFIG_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| home.join(".config"))
            .join("systemd/user")
            .join(SYSTEMD_UNIT),
        Platform::Launchd { .. } => home
            .join("Library/LaunchAgents")
            .join(format!("{LAUNCHD_LABEL}.plist")),
    }
}

/// The launchd service target of user `uid`'s agent.
fn agent(uid: u32) -> String {
    format!("gui/{uid}/{LAUNCHD_LABEL}")
}

fn call(manager: &mut dyn ServiceManager, program: &str, args: &[&str]) -> anyhow::Result<()> {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    manager
        .run(program, &args)
        .with_context(|| format!("`{program} {}` failed", args.join(" ")))
}

fn left_in_place(path: &Path) -> String {
    format!(
        "wrote {}, but starting the service failed; the unit is left in place",
        path.display()
    )
}

fn remove_file(path: &Path) -> anyhow::Result<()> {
    std::fs::remove_file(path).with_context(|| format!("cannot remove {}", path.display()))
}

/// Writes through a temporary file and a rename, so a failure never leaves
/// half a unit.
fn write_atomically(path: &Path, text: &str) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, text).with_context(|| format!("cannot write {}", temp.display()))?;
    std::fs::rename(&temp, path).with_context(|| format!("cannot write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use passalong_core::testing::MapEnv;
    use tempfile::TempDir;

    /// Records each service-manager command; the `fail`th call (from 1)
    /// fails.
    #[derive(Default)]
    struct Recorder {
        calls: Vec<String>,
        fail: Option<usize>,
    }

    impl ServiceManager for Recorder {
        fn run(&mut self, program: &str, args: &[String]) -> anyhow::Result<()> {
            self.calls.push(format!("{program} {}", args.join(" ")));
            if self.fail == Some(self.calls.len()) {
                anyhow::bail!("exit status 1");
            }
            Ok(())
        }
    }

    struct Rig {
        dir: TempDir,
        env: MapEnv,
        manager: Recorder,
    }

    impl Rig {
        fn new() -> Self {
            let dir = TempDir::new().unwrap();
            let home = dir.path().join("home");
            std::fs::create_dir_all(&home).unwrap();
            let env = MapEnv::new().with("HOME", home.to_str().unwrap());
            Self {
                dir,
                env,
                manager: Recorder::default(),
            }
        }

        fn home(&self) -> PathBuf {
            self.dir.path().join("home")
        }

        fn unit(&self) -> PathBuf {
            self.home()
                .join(".config/systemd/user/passalong-serve.service")
        }

        #[cfg(unix)]
        fn plist(&self) -> PathBuf {
            self.home()
                .join("Library/LaunchAgents/com.passalong.serve.plist")
        }

        fn run(
            &mut self,
            platform: Platform,
            args: ServiceInstallArgs,
            serve: Status,
        ) -> (anyhow::Result<()>, String) {
            let mut out = Vec::new();
            let install = Install {
                platform,
                env: &self.env,
                exe: Path::new("/opt/bin/passalong"),
                config: Some(Path::new("/etc/passalong/config.toml")),
                serve,
                manager: &mut self.manager,
            };
            let result = run(&args, install, &mut out);
            (result, String::from_utf8(out).unwrap())
        }

        #[cfg(unix)]
        fn remove(&mut self, platform: Platform) -> (anyhow::Result<()>, String) {
            let mut out = Vec::new();
            let result = remove(platform, &self.env, &mut self.manager, &mut out);
            (result, String::from_utf8(out).unwrap())
        }
    }

    fn args() -> ServiceInstallArgs {
        ServiceInstallArgs::default()
    }

    #[cfg(unix)]
    const LAUNCHD: Platform = Platform::Launchd { uid: 501 };

    #[cfg(unix)]
    #[test]
    fn systemd_install_writes_the_unit_then_enables_and_starts_it() {
        let mut rig = Rig::new();
        let (result, out) = rig.run(Platform::Systemd, args(), Status::NotRunning);
        result.unwrap();
        let unit = std::fs::read_to_string(rig.unit()).unwrap();
        assert_eq!(
            unit,
            crate::service::systemd_unit(
                "/opt/bin/passalong --config /etc/passalong/config.toml serve"
            )
        );
        assert_eq!(
            rig.manager.calls,
            [
                "systemctl --user daemon-reload",
                "systemctl --user enable --now passalong-serve.service"
            ]
        );
        assert_eq!(
            out,
            format!(
                "wrote {}\nenabled and started passalong-serve.service\n",
                rig.unit().display()
            )
        );
    }

    #[test]
    fn xdg_config_home_decides_where_the_unit_goes() {
        let mut rig = Rig::new();
        let xdg = rig.dir.path().join("xdg");
        rig.env = MapEnv::new()
            .with("HOME", rig.home().to_str().unwrap())
            .with("XDG_CONFIG_HOME", xdg.to_str().unwrap());
        rig.run(Platform::Systemd, args(), Status::NotRunning)
            .0
            .unwrap();
        assert!(xdg.join("systemd/user/passalong-serve.service").is_file());
        assert!(!rig.unit().exists());
    }

    #[test]
    fn no_start_only_writes_the_unit_and_says_how_to_start_it() {
        let mut rig = Rig::new();
        let no_start = ServiceInstallArgs {
            no_start: true,
            ..args()
        };
        let (result, out) = rig.run(
            Platform::Systemd,
            no_start,
            Status::Running {
                pid: Some(42),
                ready: true,
            },
        );
        result.unwrap();
        assert!(rig.unit().is_file());
        assert!(rig.manager.calls.is_empty(), "{:?}", rig.manager.calls);
        assert!(
            out.ends_with("start it with: systemctl --user enable --now passalong-serve.service\n"),
            "{out}"
        );
    }

    // systemd and launchd paths are Unix paths; Windows has its own tests.
    #[cfg(unix)]
    #[test]
    fn an_identical_unit_is_left_alone_and_a_different_one_needs_force() {
        let mut rig = Rig::new();
        rig.run(Platform::Systemd, args(), Status::NotRunning)
            .0
            .unwrap();
        rig.manager.calls.clear();
        let (result, out) = rig.run(Platform::Systemd, args(), Status::NotRunning);
        result.unwrap();
        assert_eq!(
            out,
            format!(
                "{} is already installed and unchanged\n",
                rig.unit().display()
            )
        );
        assert!(rig.manager.calls.is_empty());

        std::fs::write(rig.unit(), "[Unit]\nDescription=edited\n").unwrap();
        let (result, _) = rig.run(Platform::Systemd, args(), Status::NotRunning);
        let err = result.unwrap_err().to_string();
        assert!(err.contains("--force"), "{err}");
        assert_eq!(
            std::fs::read_to_string(rig.unit()).unwrap(),
            "[Unit]\nDescription=edited\n"
        );
        let force = ServiceInstallArgs {
            force: true,
            ..args()
        };
        rig.run(Platform::Systemd, force, Status::NotRunning)
            .0
            .unwrap();
        assert!(
            std::fs::read_to_string(rig.unit())
                .unwrap()
                .contains("ExecStart=/opt/bin/passalong")
        );
    }

    #[test]
    fn install_is_refused_while_serve_runs() {
        let mut rig = Rig::new();
        let (result, _) = rig.run(
            Platform::Systemd,
            args(),
            Status::Running {
                pid: Some(42),
                ready: true,
            },
        );
        let err = result.unwrap_err().to_string();
        assert!(err.contains("already running (pid 42)"), "{err}");
        assert!(err.contains("passalong serve --stop"), "{err}");
        assert!(!rig.unit().exists());
        assert!(rig.manager.calls.is_empty());
    }

    #[test]
    fn a_failed_start_leaves_the_unit_in_place_and_says_so() {
        let mut rig = Rig::new();
        rig.manager.fail = Some(2);
        let (result, _) = rig.run(Platform::Systemd, args(), Status::NotRunning);
        let err = format!("{:#}", result.unwrap_err());
        assert!(
            err.contains("systemctl --user enable --now passalong-serve.service"),
            "{err}"
        );
        assert!(err.contains("left in place"), "{err}");
        assert!(rig.unit().is_file());
    }

    #[cfg(unix)]
    #[test]
    fn systemd_remove_disables_removes_and_reloads() {
        let mut rig = Rig::new();
        rig.run(Platform::Systemd, args(), Status::NotRunning)
            .0
            .unwrap();
        rig.manager.calls.clear();
        let (result, out) = rig.remove(Platform::Systemd);
        result.unwrap();
        assert!(!rig.unit().exists());
        assert_eq!(
            rig.manager.calls,
            [
                "systemctl --user disable --now passalong-serve.service",
                "systemctl --user daemon-reload"
            ]
        );
        assert_eq!(out, format!("removed {}\n", rig.unit().display()));
        rig.manager.calls.clear();
        let (result, out) = rig.remove(Platform::Systemd);
        result.unwrap();
        assert_eq!(out, format!("not installed: no {}\n", rig.unit().display()));
        assert!(rig.manager.calls.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn launchd_install_writes_the_agent_and_bootstraps_it() {
        let mut rig = Rig::new();
        let (result, out) = rig.run(LAUNCHD, args(), Status::NotRunning);
        result.unwrap();
        let home = rig.home().display().to_string();
        let plist = std::fs::read_to_string(rig.plist()).unwrap();
        assert_eq!(
            plist,
            crate::service::launchd_plist(
                &[
                    "/opt/bin/passalong".to_owned(),
                    "--config".to_owned(),
                    "/etc/passalong/config.toml".to_owned(),
                    "serve".to_owned()
                ],
                &home,
                &format!("{home}/Library/Logs/passalong/serve.log")
            )
        );
        assert!(rig.home().join("Library/Logs/passalong").is_dir());
        assert_eq!(
            rig.manager.calls,
            [format!(
                "launchctl bootstrap gui/501 {}",
                rig.plist().display()
            )]
        );
        assert!(out.ends_with("loaded com.passalong.serve\n"), "{out}");
    }

    #[cfg(unix)]
    #[test]
    fn launchd_replacement_boots_the_old_agent_out_first() {
        let mut rig = Rig::new();
        std::fs::create_dir_all(rig.plist().parent().unwrap()).unwrap();
        std::fs::write(rig.plist(), "old").unwrap();
        let force = ServiceInstallArgs {
            force: true,
            ..args()
        };
        // Booting out an agent that is not loaded fails, which is fine.
        rig.manager.fail = Some(1);
        rig.run(LAUNCHD, force, Status::NotRunning).0.unwrap();
        assert_eq!(
            rig.manager.calls,
            [
                "launchctl bootout gui/501/com.passalong.serve".to_owned(),
                format!("launchctl bootstrap gui/501 {}", rig.plist().display())
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn launchd_remove_boots_out_and_removes_even_when_not_loaded() {
        let mut rig = Rig::new();
        rig.run(LAUNCHD, args(), Status::NotRunning).0.unwrap();
        rig.manager.calls.clear();
        rig.manager.fail = Some(1);
        let (result, out) = rig.remove(LAUNCHD);
        result.unwrap();
        assert_eq!(
            rig.manager.calls,
            ["launchctl bootout gui/501/com.passalong.serve"]
        );
        assert!(!rig.plist().exists());
        assert_eq!(out, format!("removed {}\n", rig.plist().display()));
    }

    #[test]
    fn the_platform_follows_the_build_target() {
        let dir = TempDir::new().unwrap();
        let platform = current_platform(dir.path());
        if cfg!(target_os = "linux") {
            assert_eq!(platform.unwrap(), Platform::Systemd);
        } else if cfg!(target_os = "macos") {
            assert!(matches!(platform.unwrap(), Platform::Launchd { .. }));
        } else {
            assert_eq!(platform.unwrap_err().to_string(), UNSUPPORTED);
        }
        assert_eq!(
            UNSUPPORTED,
            "service-install and service-remove support Linux (systemd) and macOS (launchd) only"
        );
    }
}
