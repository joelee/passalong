//! `passalong service-install` and `passalong service-remove`: install
//! `serve` as a service that starts at login, and remove it again.

use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use passalong_core::config::EnvProvider;

use crate::cli::ServiceInstallArgs;
use crate::daemon::{Os, StatePaths, Status};
use crate::service::{
    self, LAUNCHD_LABEL, RUN_KEY, RUN_VALUE, SYSTEMD_UNIT, ServiceManager, TASK_NAME,
};

/// The error on platforms without a supported service manager.
pub const UNSUPPORTED: &str = "service-install and service-remove support Linux (systemd), macOS (launchd), and Windows (the Run key or Task Scheduler) only";

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
    /// The per-user Run key, on Windows: `serve --daemon` at log-in.
    RunKey,
    /// A Task Scheduler log-on task, on Windows, which restarts `serve`
    /// when it fails; creating it needs an administrator.
    Scheduler,
}

/// The platform this binary was built for; on Windows the Task Scheduler
/// when `scheduler`, the Run key otherwise. For launchd, the user is the
/// owner of `home`.
pub fn current_platform(home: &Path, scheduler: bool) -> anyhow::Result<Platform> {
    if cfg!(windows) {
        return Ok(if scheduler {
            Platform::Scheduler
        } else {
            Platform::RunKey
        });
    }
    anyhow::ensure!(!scheduler, "--scheduler is for Windows only");
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
    match platform {
        Platform::RunKey => return install_run_key(args, exe, config, serve, manager, out),
        Platform::Scheduler => {
            return install_task(args, env, exe, config, serve, manager, out);
        }
        Platform::Systemd | Platform::Launchd { .. } => {}
    }
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
        Platform::RunKey | Platform::Scheduler => unreachable!("installed above"),
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
    refuse_if_running(args, serve)?;
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
        Platform::RunKey | Platform::Scheduler => unreachable!("installed above"),
    }
    Ok(())
}

/// Refuses to start the service while a `serve` runs, unless it is not to
/// be started now.
fn refuse_if_running(args: &ServiceInstallArgs, serve: Status) -> anyhow::Result<()> {
    if !args.no_start
        && let Status::Running { pid, .. } = serve
    {
        let pid = pid.map_or_else(String::new, |pid| format!(" (pid {pid})"));
        anyhow::bail!(
            "serve is already running{pid}; stop it first with `passalong serve --stop`, or with `passalong service-remove` if a service runs it"
        );
    }
    Ok(())
}

/// Adds `passalong-serve` to the per-user Run key, which starts
/// `serve --daemon` at every log-in, and starts it now unless `--no-start`.
fn install_run_key(
    args: &ServiceInstallArgs,
    exe: &Path,
    config: Option<&Path>,
    serve: Status,
    manager: &mut dyn ServiceManager,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let command = service::run_key_command(exe, config);
    let length = command.chars().count();
    anyhow::ensure!(
        length <= service::RUN_KEY_MAX,
        "the command that starts serve has {length} characters, over the {} a Run key command may have; use `passalong service-install --scheduler` from an administrator prompt, or a shorter config path",
        service::RUN_KEY_MAX
    );
    let existing = query(manager, "reg", &["query", RUN_KEY, "/v", RUN_VALUE])?
        .map(|found| service::parse_reg_value(&found).unwrap_or_default());
    if !args.force {
        match existing.as_deref() {
            Some(existing) if existing == command => {
                writeln!(out, "{RUN_VALUE} is already in {RUN_KEY}, unchanged")?;
                return Ok(());
            }
            Some(_) => anyhow::bail!(
                "{RUN_VALUE} is already in {RUN_KEY} and differs; use --force to replace it"
            ),
            None => {}
        }
    }
    refuse_if_running(args, serve)?;
    call(
        manager,
        "reg",
        &[
            "add", RUN_KEY, "/v", RUN_VALUE, "/t", "REG_SZ", "/d", &command, "/f",
        ],
    )?;
    writeln!(
        out,
        "added {RUN_VALUE} to {RUN_KEY}: serve starts when you log in"
    )?;
    tracing::info!(value = RUN_VALUE, "Run key value written");
    if args.no_start {
        writeln!(out, "start it now with: passalong serve --daemon")?;
        return Ok(());
    }
    let start = service::serve_args(config, true);
    let start: Vec<&str> = start.iter().map(String::as_str).collect();
    call(manager, &exe.display().to_string(), &start)
        .context("the Run key value is in place, but starting serve now failed")?;
    writeln!(out, "started serve")?;
    Ok(())
}

/// Registers the Task Scheduler task `passalong-serve`, which runs `serve`
/// when this user logs in and restarts it when it fails, and runs it now
/// unless `--no-start`. Windows lets only an administrator create a
/// log-on task.
fn install_task(
    args: &ServiceInstallArgs,
    env: &dyn EnvProvider,
    exe: &Path,
    config: Option<&Path>,
    serve: Status,
    manager: &mut dyn ServiceManager,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let user = windows_user(env)?;
    let exists = query(manager, "schtasks", &["/Query", "/TN", TASK_NAME])?.is_some();
    anyhow::ensure!(
        !exists || args.force,
        "the scheduled task {TASK_NAME} exists; use --force to replace it"
    );
    refuse_if_running(args, serve)?;
    let xml = service::task_xml(
        &user,
        &exe.display().to_string(),
        &service::serve_args(config, false),
    );
    let file =
        std::env::temp_dir().join(format!("passalong-serve-task-{}.xml", std::process::id()));
    std::fs::write(&file, service::utf16_with_bom(&xml))
        .with_context(|| format!("cannot write {}", file.display()))?;
    let created = call(
        manager,
        "schtasks",
        &[
            "/Create",
            "/TN",
            TASK_NAME,
            "/XML",
            &file.display().to_string(),
            "/F",
        ],
    );
    let _ = std::fs::remove_file(&file);
    created.context(
        "Windows lets only an administrator create a log-on task: run this from an administrator prompt, or leave out --scheduler to use the Run key",
    )?;
    writeln!(
        out,
        "registered the scheduled task {TASK_NAME}: serve starts when you log in and restarts if it fails"
    )?;
    tracing::info!(task = TASK_NAME, "scheduled task registered");
    if args.no_start {
        writeln!(out, "start it now with: schtasks /Run /TN {TASK_NAME}")?;
        return Ok(());
    }
    call(manager, "schtasks", &["/Run", "/TN", TASK_NAME])?;
    writeln!(out, "started the task")?;
    Ok(())
}

/// `DOMAIN\name` of the user a scheduled task runs as.
fn windows_user(env: &dyn EnvProvider) -> anyhow::Result<String> {
    let var = |key: &str| env.var(key).filter(|value| !value.is_empty());
    let name = var("USERNAME").context("cannot tell who you are: USERNAME is not set")?;
    Ok(match var("USERDOMAIN") {
        Some(domain) => format!("{domain}\\{name}"),
        None => name,
    })
}

/// Removes the Run key value and the scheduled task, whichever exist.
fn remove_windows(manager: &mut dyn ServiceManager, out: &mut dyn Write) -> anyhow::Result<()> {
    let mut removed = false;
    if query(manager, "reg", &["query", RUN_KEY, "/v", RUN_VALUE])?.is_some() {
        call(manager, "reg", &["delete", RUN_KEY, "/v", RUN_VALUE, "/f"])?;
        writeln!(out, "removed {RUN_VALUE} from {RUN_KEY}")?;
        removed = true;
    }
    if query(manager, "schtasks", &["/Query", "/TN", TASK_NAME])?.is_some() {
        if let Err(err) = call(manager, "schtasks", &["/End", "/TN", TASK_NAME]) {
            tracing::debug!(error = %format!("{err:#}"), "the task was not running");
        }
        call(manager, "schtasks", &["/Delete", "/TN", TASK_NAME, "/F"])?;
        writeln!(out, "removed the scheduled task {TASK_NAME}")?;
        removed = true;
    }
    if removed {
        writeln!(
            out,
            "a serve started at log-in keeps running until `passalong serve --stop`"
        )?;
    } else {
        writeln!(
            out,
            "not installed: no {RUN_VALUE} in {RUN_KEY} and no scheduled task {TASK_NAME}"
        )?;
    }
    Ok(())
}

fn query(
    manager: &mut dyn ServiceManager,
    program: &str,
    args: &[&str],
) -> anyhow::Result<Option<String>> {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    manager
        .query(program, &args)
        .with_context(|| format!("`{program} {}` failed", args.join(" ")))
}

/// Stops, disables, and removes the installed unit, or says it is not
/// installed.
pub fn remove(
    platform: Platform,
    env: &dyn EnvProvider,
    manager: &mut dyn ServiceManager,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    if matches!(platform, Platform::RunKey | Platform::Scheduler) {
        return remove_windows(manager, out);
    }
    let path = unit_path(platform, env, &home_dir(env)?);
    remove_unit(platform, &path, manager, out)
}

/// The home folder: `HOME`, or on Windows `USERPROFILE`.
///
/// # Errors
///
/// When neither is set.
pub fn home_dir(env: &dyn EnvProvider) -> anyhow::Result<PathBuf> {
    let var = |key: &str| env.var(key).filter(|value| !value.is_empty());
    var("HOME")
        .or_else(|| {
            if cfg!(windows) {
                var("USERPROFILE")
            } else {
                None
            }
        })
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
        Platform::RunKey | Platform::Scheduler => unreachable!("removed by remove_windows"),
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
        Platform::RunKey | Platform::Scheduler => unreachable!("no unit file on Windows"),
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
        /// What queries find, by command line; others find nothing.
        found: std::collections::HashMap<String, String>,
    }

    impl ServiceManager for Recorder {
        fn run(&mut self, program: &str, args: &[String]) -> anyhow::Result<()> {
            self.calls.push(format!("{program} {}", args.join(" ")));
            if self.fail == Some(self.calls.len()) {
                anyhow::bail!("exit status 1");
            }
            Ok(())
        }

        fn query(&mut self, program: &str, args: &[String]) -> anyhow::Result<Option<String>> {
            let call = format!("{program} {}", args.join(" "));
            self.calls.push(call.clone());
            Ok(self.found.get(&call).cloned())
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
            let env = MapEnv::new()
                .with("HOME", home.to_str().unwrap())
                .with("USERNAME", "me")
                .with("USERDOMAIN", "DESKTOP");
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

    const START: &str = "/opt/bin/passalong --config /etc/passalong/config.toml serve --daemon";

    fn run_key_query() -> String {
        format!("reg query {RUN_KEY} /v {RUN_VALUE}")
    }

    fn task_query() -> String {
        format!("schtasks /Query /TN {TASK_NAME}")
    }

    #[test]
    fn a_run_key_install_adds_the_value_and_starts_serve() {
        let mut rig = Rig::new();
        let (result, out) = rig.run(Platform::RunKey, args(), Status::NotRunning);
        result.unwrap();
        assert_eq!(
            rig.manager.calls,
            [
                run_key_query(),
                format!(
                    "reg add {RUN_KEY} /v {RUN_VALUE} /t REG_SZ /d \"/opt/bin/passalong\" --config /etc/passalong/config.toml serve --daemon /f"
                ),
                START.to_owned(),
            ]
        );
        assert!(out.contains("serve starts when you log in"), "{out}");
        assert!(out.ends_with("started serve\n"), "{out}");
    }

    #[test]
    fn an_unchanged_run_key_value_is_left_alone_and_a_different_one_needs_force() {
        let mut rig = Rig::new();
        let command = service::run_key_command(
            Path::new("/opt/bin/passalong"),
            Some(Path::new("/etc/passalong/config.toml")),
        );
        rig.manager.found.insert(
            run_key_query(),
            format!("\r\n{RUN_KEY}\r\n    {RUN_VALUE}    REG_SZ    {command}\r\n\r\n"),
        );
        let (result, out) = rig.run(Platform::RunKey, args(), Status::NotRunning);
        result.unwrap();
        assert!(out.contains("unchanged"), "{out}");
        assert_eq!(rig.manager.calls, [run_key_query()]);

        rig.manager.found.insert(
            run_key_query(),
            format!("    {RUN_VALUE}    REG_SZ    \"C:\\old\\passalong.exe\" serve --daemon\r\n"),
        );
        let (result, _) = rig.run(Platform::RunKey, args(), Status::NotRunning);
        assert!(result.unwrap_err().to_string().contains("--force"));
        let forced = ServiceInstallArgs {
            force: true,
            no_start: true,
            ..args()
        };
        let (result, out) = rig.run(Platform::RunKey, forced, Status::NotRunning);
        result.unwrap();
        assert!(
            out.ends_with("start it now with: passalong serve --daemon\n"),
            "{out}"
        );
    }

    #[test]
    fn a_run_key_command_over_260_characters_is_refused() {
        let mut rig = Rig::new();
        let long = format!("/{}/config.toml", "x".repeat(260));
        let mut out = Vec::new();
        let result = run(
            &args(),
            Install {
                platform: Platform::RunKey,
                env: &rig.env,
                exe: Path::new("/opt/bin/passalong"),
                config: Some(Path::new(&long)),
                serve: Status::NotRunning,
                manager: &mut rig.manager,
            },
            &mut out,
        );
        let err = result.unwrap_err().to_string();
        assert!(err.contains("260") && err.contains("--scheduler"), "{err}");
        assert!(rig.manager.calls.is_empty());
    }

    #[test]
    fn a_scheduler_install_registers_the_task_and_runs_it() {
        let mut rig = Rig::new();
        let (result, out) = rig.run(Platform::Scheduler, args(), Status::NotRunning);
        result.unwrap();
        let calls = &rig.manager.calls;
        assert_eq!(calls[0], task_query());
        assert!(
            calls[1].starts_with(&format!("schtasks /Create /TN {TASK_NAME} /XML "))
                && calls[1].ends_with(" /F"),
            "{calls:?}"
        );
        assert_eq!(calls[2], format!("schtasks /Run /TN {TASK_NAME}"));
        assert_eq!(calls.len(), 3);
        assert!(out.contains("restarts if it fails"), "{out}");

        rig.manager.found.insert(task_query(), "Folder: \\".into());
        let (result, _) = rig.run(Platform::Scheduler, args(), Status::NotRunning);
        assert!(result.unwrap_err().to_string().contains("--force"));
    }

    #[test]
    fn the_task_runs_serve_at_this_user_s_log_in_and_restarts_it() {
        let xml = service::task_xml(
            r"DESKTOP\me",
            r"C:\Program Files\passalong.exe",
            &service::serve_args(Some(Path::new(r"C:\cfg & more\config.toml")), false),
        );
        for part in [
            r"<UserId>DESKTOP\me</UserId>",
            "<LogonTrigger>",
            r"<Command>C:\Program Files\passalong.exe</Command>",
            r"<Arguments>--config &quot;C:\cfg &amp; more\config.toml&quot; serve</Arguments>",
            "<RestartOnFailure>",
            "<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>",
        ] {
            assert!(xml.contains(part), "{part}: {xml}");
        }
        assert_eq!(service::utf16_with_bom("A"), [0xFF, 0xFE, b'A', 0]);
    }

    #[test]
    fn service_remove_on_windows_removes_the_value_and_the_task() {
        let mut rig = Rig::new();
        let (result, out) = rig.remove(Platform::RunKey);
        result.unwrap();
        assert!(out.starts_with("not installed"), "{out}");

        rig.manager.found.insert(run_key_query(), "found".into());
        rig.manager.found.insert(task_query(), "found".into());
        rig.manager.calls.clear();
        let (result, out) = rig.remove(Platform::Scheduler);
        result.unwrap();
        assert_eq!(
            rig.manager.calls,
            [
                run_key_query(),
                format!("reg delete {RUN_KEY} /v {RUN_VALUE} /f"),
                task_query(),
                format!("schtasks /End /TN {TASK_NAME}"),
                format!("schtasks /Delete /TN {TASK_NAME} /F"),
            ]
        );
        assert!(
            out.contains("keeps running until `passalong serve --stop`"),
            "{out}"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn the_scheduler_is_for_windows_only() {
        let err = current_platform(Path::new("/home/u"), true).unwrap_err();
        assert!(err.to_string().contains("Windows"), "{err}");
    }

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
        let platform = current_platform(dir.path(), false);
        if cfg!(windows) {
            assert_eq!(platform.unwrap(), Platform::RunKey);
            assert_eq!(
                current_platform(dir.path(), true).unwrap(),
                Platform::Scheduler
            );
        } else if cfg!(target_os = "linux") {
            assert_eq!(platform.unwrap(), Platform::Systemd);
        } else if cfg!(target_os = "macos") {
            assert!(matches!(platform.unwrap(), Platform::Launchd { .. }));
        } else {
            assert_eq!(platform.unwrap_err().to_string(), UNSUPPORTED);
        }
        assert_eq!(
            UNSUPPORTED,
            "service-install and service-remove support Linux (systemd), macOS (launchd), and Windows (the Run key or Task Scheduler) only"
        );
    }
}
