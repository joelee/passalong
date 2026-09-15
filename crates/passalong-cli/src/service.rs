//! Units that start `passalong serve` at login: a systemd user unit on
//! Linux and a launchd agent on macOS, and the service manager that loads
//! them. `docs/service/` holds the same units with placeholder paths.

use std::path::Path;
use std::process::Command;

use anyhow::Context as _;

/// The systemd user unit's file name.
pub const SYSTEMD_UNIT: &str = "passalong-serve.service";
/// The launchd agent's label, which is also its file name before `.plist`.
pub const LAUNCHD_LABEL: &str = "com.passalong.serve";
/// The first line of every plist.
pub const XML_DECLARATION: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n";
/// The per-user Run key: its programs start when the user logs in.
pub const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
/// The Run key value `service-install` writes.
pub const RUN_VALUE: &str = "passalong-serve";
/// The Task Scheduler task `service-install --scheduler` registers.
pub const TASK_NAME: &str = "passalong-serve";
/// The most characters a Run key command may have.
pub const RUN_KEY_MAX: usize = 260;

/// The arguments that start serve: `[--config CONFIG] serve [--daemon]`.
pub fn serve_args(config: Option<&Path>, daemon: bool) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(config) = config {
        args.push("--config".to_owned());
        args.push(config.display().to_string());
    }
    args.push("serve".to_owned());
    if daemon {
        args.push("--daemon".to_owned());
    }
    args
}

/// `arg` quoted for a Windows command line, when it needs quotes.
fn windows_quote(arg: &str) -> String {
    crate::daemon::windows_arg(arg)
}

/// The Run key command: `"EXE" [--config CONFIG] serve --daemon`, the
/// executable always quoted.
pub fn run_key_command(exe: &Path, config: Option<&Path>) -> String {
    let mut parts = vec![format!("\"{}\"", exe.display())];
    parts.extend(
        serve_args(config, true)
            .iter()
            .map(|arg| windows_quote(arg)),
    );
    parts.join(" ")
}

/// The command in `reg query` output: the text after `REG_SZ` on its line.
pub fn parse_reg_value(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        line.split_once("REG_SZ")
            .map(|(_, value)| value.trim().to_owned())
    })
}

/// The Task Scheduler task: `exe args`, run in `user`'s session when that
/// user logs in, restarted up to three times a minute apart when it fails,
/// with no time limit.
pub fn task_xml(user: &str, exe: &str, args: &[String]) -> String {
    let arguments = args
        .iter()
        .map(|arg| windows_quote(arg))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>passalong serve: send clipboard text and dropped files. Written by passalong service-install; remove it with passalong service-remove.</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>3</Count>
    </RestartOnFailure>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe}</Command>
      <Arguments>{arguments}</Arguments>
    </Exec>
  </Actions>
</Task>
"#,
        user = xml_escape(user),
        exe = xml_escape(exe),
        arguments = xml_escape(&arguments)
    )
}

/// `text` as UTF-16 with a byte-order mark, as `schtasks /XML` reads it.
pub fn utf16_with_bom(text: &str) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xFE];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

/// The `ExecStart=` value that runs `exe [--config CONFIG] serve`, with `%`
/// doubled and arguments holding spaces, quotes, or backslashes quoted, as
/// systemd requires.
pub fn systemd_exec_start(exe: &Path, config: Option<&Path>) -> String {
    let mut args = vec![exe.display().to_string()];
    if let Some(config) = config {
        args.push("--config".to_owned());
        args.push(config.display().to_string());
    }
    args.push("serve".to_owned());
    args.iter()
        .map(|arg| systemd_quote(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn systemd_quote(arg: &str) -> String {
    let escaped = arg.replace('%', "%%");
    if escaped
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\'))
    {
        format!("\"{}\"", escaped.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        escaped
    }
}

/// The systemd unit from `[Unit]` on, running `exec_start` from the home
/// directory.
pub fn systemd_unit_body(exec_start: &str) -> String {
    format!(
        "[Unit]\n\
Description=passalong: send clipboard text and dropped files\n\
PartOf=graphical-session.target\n\
After=graphical-session.target network-online.target\n\
\n\
[Service]\n\
Type=simple\n\
ExecStart={exec_start}\n\
WorkingDirectory=%h\n\
Restart=on-failure\n\
RestartSec=10\n\
# error, warning, info, verbose, or debug\n\
Environment=PASSALONG_LOG_LEVEL=info\n\
\n\
[Install]\n\
WantedBy=graphical-session.target\n\
"
    )
}

/// The systemd unit `service-install` writes.
pub fn systemd_unit(exec_start: &str) -> String {
    format!(
        "# Written by `passalong service-install`; remove it with\n\
         # `passalong service-remove`.\n\
         # Logs: journalctl --user -u passalong-serve.service -f\n\n{}",
        systemd_unit_body(exec_start)
    )
}

/// The launchd agent from `<!DOCTYPE` on: `args` run from `home`, with
/// standard error appended to `log`.
pub fn launchd_plist_body(args: &[String], home: &str, log: &str) -> String {
    let mut text = String::from(
        r#"<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.passalong.serve</string>
  <key>ProgramArguments</key>
  <array>
"#,
    );
    for arg in args {
        text.push_str(&format!("    <string>{}</string>\n", xml_escape(arg)));
    }
    text.push_str(&format!(
        r#"  </array>
  <key>WorkingDirectory</key>
  <string>{home}</string>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>StandardErrorPath</key>
  <string>{log}</string>
</dict>
</plist>
"#,
        home = xml_escape(home),
        log = xml_escape(log)
    ));
    text
}

/// The launchd agent `service-install` writes.
pub fn launchd_plist(args: &[String], home: &str, log: &str) -> String {
    // XML comments cannot contain two hyphens in a row; this text has none.
    format!(
        "{XML_DECLARATION}<!-- Written by passalong service-install; remove it with passalong service-remove. -->\n{}",
        launchd_plist_body(args, home, log)
    )
}

/// Escapes text for an XML element.
fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Runs service-manager commands, such as `systemctl --user daemon-reload`.
pub trait ServiceManager {
    /// Runs `program` with `args` and fails unless it succeeds.
    fn run(&mut self, program: &str, args: &[String]) -> anyhow::Result<()>;

    /// Runs a query: its output when it succeeds, `None` when it exits with
    /// status 1, which `reg query` and `schtasks /Query` give for what does
    /// not exist, and an error otherwise.
    fn query(&mut self, program: &str, args: &[String]) -> anyhow::Result<Option<String>>;
}

/// [`ServiceManager`] that runs the real programs. Their output is kept
/// and shown only when they fail, so `--quiet` stays quiet.
pub struct ProcessManager;

impl ServiceManager for ProcessManager {
    fn run(&mut self, program: &str, args: &[String]) -> anyhow::Result<()> {
        let output = Command::new(program)
            .args(args)
            .output()
            .with_context(|| format!("cannot run `{program}`"))?;
        if !output.status.success() {
            let said = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("{} ({})", said.trim(), output.status);
        }
        Ok(())
    }

    fn query(&mut self, program: &str, args: &[String]) -> anyhow::Result<Option<String>> {
        let output = Command::new(program)
            .args(args)
            .output()
            .with_context(|| format!("cannot run `{program}`"))?;
        match output.status.code() {
            Some(0) => Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned())),
            Some(1) => Ok(None),
            _ => {
                let said = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!("{} ({})", said.trim(), output.status)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn docs(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/service")
            .join(name);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("{}: {err}", path.display()))
            // A Windows checkout may turn line ends into CRLF.
            .replace("\r\n", "\n")
    }

    #[test]
    fn the_docs_task_is_the_task_service_install_registers() {
        let task = task_xml(
            r"DOMAIN\user",
            r"C:\Users\user\.cargo\bin\passalong.exe",
            &serve_args(
                Some(Path::new(
                    r"C:\Users\user\AppData\Roaming\passalong\config.toml",
                )),
                false,
            ),
        );
        let from_triggers = |text: &str| text[text.find("  <Triggers>").unwrap()..].to_owned();
        assert_eq!(
            from_triggers(&docs("passalong-serve-task.xml")),
            from_triggers(&task)
        );
    }

    #[test]
    fn run_key_commands_quote_the_exe_and_paths_with_spaces() {
        assert_eq!(
            run_key_command(Path::new(r"C:\bin\passalong.exe"), None),
            r#""C:\bin\passalong.exe" serve --daemon"#
        );
        assert_eq!(
            run_key_command(
                Path::new(r"C:\Program Files\passalong.exe"),
                Some(Path::new(r"C:\my files\config.toml"))
            ),
            r#""C:\Program Files\passalong.exe" --config "C:\my files\config.toml" serve --daemon"#
        );
        assert_eq!(
            parse_reg_value("    passalong-serve    REG_SZ    \"C:\\p.exe\" serve --daemon\r\n"),
            Some("\"C:\\p.exe\" serve --daemon".to_owned())
        );
        assert_eq!(parse_reg_value("nothing here"), None);
    }

    #[test]
    fn exec_start_quotes_spaces_and_escapes_percent_signs() {
        assert_eq!(
            systemd_exec_start(Path::new("/usr/bin/passalong"), None),
            "/usr/bin/passalong serve"
        );
        assert_eq!(
            systemd_exec_start(
                Path::new("/opt/pass along/passalong"),
                Some(Path::new("/home/u/100%/c.toml"))
            ),
            "\"/opt/pass along/passalong\" --config /home/u/100%%/c.toml serve"
        );
    }

    #[test]
    fn the_systemd_unit_runs_serve_from_the_home_directory() {
        let unit = systemd_unit("/usr/bin/passalong serve");
        assert!(
            unit.starts_with("# Written by `passalong service-install`"),
            "{unit}"
        );
        assert!(unit.contains("`passalong service-remove`"), "{unit}");
        for line in [
            "ExecStart=/usr/bin/passalong serve\n",
            "WorkingDirectory=%h\n",
            "Restart=on-failure\n",
            "WantedBy=graphical-session.target\n",
        ] {
            assert!(unit.contains(line), "{line}: {unit}");
        }
    }

    #[test]
    fn the_launchd_plist_lists_every_argument_escaped() {
        let args = [
            "/Apps/pass & go/passalong".to_owned(),
            "--config".to_owned(),
            "/c.toml".to_owned(),
            "serve".to_owned(),
        ];
        let plist = launchd_plist(
            &args,
            "/Users/me",
            "/Users/me/Library/Logs/passalong/serve.log",
        );
        assert!(plist.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- Written by passalong service-install"), "{plist}");
        let comment = &plist[plist.find("<!--").unwrap() + 4..plist.find("-->").unwrap()];
        assert!(
            !comment.contains("--"),
            "XML comments cannot hold `--`: {comment}"
        );
        for part in [
            "<string>com.passalong.serve</string>",
            "<string>/Apps/pass &amp; go/passalong</string>\n    <string>--config</string>\n    <string>/c.toml</string>\n    <string>serve</string>",
            "<key>WorkingDirectory</key>\n  <string>/Users/me</string>",
            "<key>StandardErrorPath</key>\n  <string>/Users/me/Library/Logs/passalong/serve.log</string>",
        ] {
            assert!(plist.contains(part), "{part}: {plist}");
        }
    }

    #[test]
    fn the_documented_units_are_the_templates_with_placeholders() {
        let unit = docs("passalong-serve.service");
        let body = systemd_unit_body("%h/.cargo/bin/passalong serve");
        let header = unit
            .strip_suffix(&body)
            .expect("the documented unit ends with the template body");
        assert!(
            header
                .lines()
                .all(|line| line.is_empty() || line.starts_with('#')),
            "only comments precede the body: {header}"
        );

        let plist = docs("com.passalong.serve.plist");
        let body = launchd_plist_body(
            &[
                "/Users/YOU/.cargo/bin/passalong".to_owned(),
                "serve".to_owned(),
            ],
            "/Users/YOU",
            "/Users/YOU/Library/Logs/passalong/serve.log",
        );
        let head = plist
            .strip_suffix(&body)
            .expect("the documented plist ends with the template body");
        let comment = head
            .strip_prefix(XML_DECLARATION)
            .expect("the documented plist starts with the XML declaration")
            .trim();
        assert!(
            comment.starts_with("<!--") && comment.ends_with("-->"),
            "only a comment precedes the body: {comment}"
        );
        assert!(!comment[4..comment.len() - 3].contains("--"), "{comment}");
    }

    #[test]
    fn xml_special_characters_are_escaped() {
        assert_eq!(
            xml_escape(r#"a&b<c>d"e'f"#),
            "a&amp;b&lt;c&gt;d&quot;e&apos;f"
        );
    }
}
