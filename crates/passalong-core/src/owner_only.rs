//! Files only their owner can open. Unix gets that from the mode a file is
//! created with; Windows needs an access list, which `icacls` sets: the
//! inherited entries removed, and full control granted to the current user
//! alone, named by the security identifier `whoami` reports.
//!
//! The parsing below runs on every platform, so its tests do too; the
//! commands run on Windows only.
#![cfg_attr(not(windows), allow(dead_code))]

/// The current user's account name and security identifier, from the
/// output of `whoami /user /fo csv /nh`: `"domain\name","S-1-5-..."`.
fn parse_whoami(output: &str) -> Option<(String, String)> {
    let line = output.lines().find(|line| !line.trim().is_empty())?;
    let (name, sid) = line.trim().split_once("\",\"")?;
    let name = name.trim_start_matches('"');
    let sid = sid.trim_end_matches('"');
    (!name.is_empty() && sid.starts_with("S-")).then(|| (name.to_owned(), sid.to_owned()))
}

/// The accounts `icacls <path>` lists for `path`: one entry per line, the
/// first after the path itself, up to the blank line before the summary,
/// whose wording depends on the system's language.
fn parse_icacls(output: &str, path: &str) -> Vec<String> {
    let mut accounts = Vec::new();
    for (i, line) in output.lines().enumerate() {
        let entry = if i == 0 {
            line.strip_prefix(path).unwrap_or(line)
        } else {
            line
        };
        let entry = entry.trim();
        if entry.is_empty() {
            break;
        }
        if let Some((account, _)) = entry.split_once(":(") {
            accounts.push(account.to_owned());
        }
    }
    accounts
}

/// Whether `accounts` is the account `user` alone.
fn only(accounts: &[String], user: &str) -> bool {
    matches!(accounts, [only] if only.eq_ignore_ascii_case(user))
}

#[cfg(windows)]
fn run(program: &str, args: &[&std::ffi::OsStr]) -> std::io::Result<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "`{program}` failed ({})",
            output.status
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(windows)]
fn current_user() -> std::io::Result<(String, String)> {
    use std::ffi::OsStr;
    let out = run(
        "whoami",
        &[
            OsStr::new("/user"),
            OsStr::new("/fo"),
            OsStr::new("csv"),
            OsStr::new("/nh"),
        ],
    )?;
    parse_whoami(&out).ok_or_else(|| std::io::Error::other("`whoami /user` named no account"))
}

/// Gives the current user alone full control of `path`; a folder passes
/// that on to what is created in it.
///
/// # Errors
///
/// When `whoami` or `icacls` cannot be run or fails.
#[cfg(windows)]
pub(crate) fn restrict(path: &std::path::Path, folder: bool) -> std::io::Result<()> {
    use std::ffi::OsStr;
    let (_, sid) = current_user()?;
    let grant = if folder {
        format!("*{sid}:(OI)(CI)F")
    } else {
        format!("*{sid}:F")
    };
    run(
        "icacls",
        &[
            path.as_os_str(),
            OsStr::new("/inheritance:r"),
            OsStr::new("/grant:r"),
            OsStr::new(&grant),
        ],
    )
    .map(drop)
}

/// Whether the current user alone may open `path`.
///
/// # Errors
///
/// When `whoami` or `icacls` cannot be run or fails.
#[cfg(windows)]
pub(crate) fn only_owner(path: &std::path::Path) -> std::io::Result<bool> {
    let (user, _) = current_user()?;
    let out = run("icacls", &[path.as_os_str()])?;
    Ok(only(&parse_icacls(&out, &path.to_string_lossy()), &user))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATH: &str = r"C:\Users\me\AppData\Roaming\passalong\store.key";
    const ME: &str = r"desktop-1\me";

    fn listing(entries: &[&str], summary: &str) -> String {
        let pad = " ".repeat(PATH.len() + 1);
        let mut out = format!("{PATH} {}\r\n", entries[0]);
        for entry in &entries[1..] {
            out.push_str(&format!("{pad}{entry}\r\n"));
        }
        out.push_str(&format!("\r\n{summary}\r\n"));
        out
    }

    const SUMMARY: &str = "Successfully processed 1 files; Failed processing 0 files";

    #[test]
    fn whoami_names_the_account_and_its_sid() {
        assert_eq!(
            parse_whoami("\"desktop-1\\me\",\"S-1-5-21-1-2-3-1001\"\r\n"),
            Some((ME.to_owned(), "S-1-5-21-1-2-3-1001".to_owned()))
        );
        for bad in ["", "\r\n", "garbage", "\"me\",\"not-a-sid\""] {
            assert_eq!(parse_whoami(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_file_only_its_owner_may_open_passes() {
        let out = listing(&[r"DESKTOP-1\me:(F)"], SUMMARY);
        assert_eq!(parse_icacls(&out, PATH), [r"DESKTOP-1\me"]);
        assert!(
            only(&parse_icacls(&out, PATH), ME),
            "names match in any case"
        );
    }

    #[test]
    fn any_other_entry_fails() {
        let cases = [
            listing(&[r"BUILTIN\Users:(R)", r"DESKTOP-1\me:(F)"], SUMMARY),
            listing(
                &[
                    r"DESKTOP-1\me:(I)(F)",
                    r"NT AUTHORITY\SYSTEM:(I)(F)",
                    r"BUILTIN\Administrators:(I)(F)",
                ],
                SUMMARY,
            ),
            // Account names and the summary follow the system's language.
            listing(
                &[r"VORDEFINIERT\Benutzer:(R)", r"DESKTOP-1\me:(F)"],
                "1 Dateien erfolgreich verarbeitet, bei 0 Dateien ist ein Verarbeitungsfehler aufgetreten.",
            ),
            listing(&[r"DESKTOP-1\someone-else:(F)"], SUMMARY),
            String::new(),
        ];
        for out in cases {
            assert!(!only(&parse_icacls(&out, PATH), ME), "{out}");
        }
    }

    #[test]
    fn a_listing_of_another_path_is_not_taken_for_this_one() {
        let out = listing(&[r"DESKTOP-1\me:(F)"], SUMMARY).replace(PATH, r"C:\other.key");
        assert!(!only(&parse_icacls(&out, PATH), ME));
    }
}
