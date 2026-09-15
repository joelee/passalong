//! Compatibility: the released v0.1.6 binary must never write item data
//! into a store laid out for encryption.
//!
//! An encrypted store's root holds a regular file named `items`, where
//! v0.1.6 expects its item directory, so every v0.1.6 command that touches
//! items fails instead of storing plaintext beside encrypted items.
//!
//! The tests are ignored by default and need the old binary in
//! `PASSALONG_COMPAT_BIN`; `just test-compat` downloads the v0.1.6 release
//! for this platform and runs them. `serve` is not run, because it would
//! read the real clipboard; it stores through the same `FsStore::put` as
//! `clipboard`.

use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::Arc;

use assert_cmd::Command;
use passalong_core::clock::SystemClock;
use passalong_core::crypto::{DataKey, KdfParams, Sealer, Words, wrap};
use passalong_core::encryption::{STOP_TEXT, StoreHeader, create_header, write_stop_file};
use passalong_core::fs::LocalFs;
use passalong_core::model::NewItem;
use passalong_core::random::StdRandom;
use passalong_core::store::{FsStore, Store};
use tempfile::TempDir;
const CLIPBOARD_MARKER: &str = "compat-marker-clipboard-7f3a19";
const FILE_MARKER: &str = "compat-marker-file-91c2e4";

struct Sandbox {
    dir: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        for sub in ["home", "work", "cfg", "store", "drop", "downloads"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        Self { dir }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    fn config(&self) -> PathBuf {
        let path = self.path("cfg/config.toml");
        let text = format!(
            "[client]\ndevice_name = \"compat-box\"\ndownload_dir = \"{}\"\n\n[server]\nkind = \"local\"\n\n[server.local]\npath = \"{}\"\n\n[serve]\ndrop_folder = \"{}\"\n",
            self.path("downloads").display(),
            self.path("store").display(),
            self.path("drop").display()
        );
        std::fs::write(&path, text).unwrap();
        path
    }

    /// The old binary with a clean environment and the sandbox's config.
    fn old(&self) -> Command {
        let bin = std::env::var_os("PASSALONG_COMPAT_BIN")
            .expect("PASSALONG_COMPAT_BIN must name the v0.1.6 binary; run `just test-compat`");
        let mut cmd = Command::new(bin);
        cmd.current_dir(self.path("work"))
            .env("HOME", self.path("home"))
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_STATE_HOME")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .env_remove("PASSALONG_CONFIG_FILE")
            .env_remove("PASSALONG_LOG_LEVEL")
            .env_remove("PASSALONG_SSH_KEY_PASSPHRASE")
            .arg("--config")
            .arg(self.config());
        cmd
    }
}

/// Every file below `root`, with its path relative to `root`.
fn files_below(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let bytes = std::fs::read(&path).unwrap();
                found.push((path.strip_prefix(root).unwrap().to_path_buf(), bytes));
            }
        }
    }
    found
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

fn describe(what: &str, out: &Output) -> String {
    format!(
        "{what}: exit {:?}\nstdout: {}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Runs every v0.1.6 command that reads or writes items against the store
/// in `sb`, and asserts that each one fails and that no marker text reaches
/// the store.
fn assert_old_client_is_refused(sb: &Sandbox) {
    let secret_file = sb.path("work/secret.txt");
    std::fs::write(&secret_file, FILE_MARKER).unwrap();

    let runs: Vec<(&str, Output)> = vec![
        (
            "clipboard --stdin",
            sb.old()
                .args(["clipboard", "--stdin"])
                .write_stdin(CLIPBOARD_MARKER)
                .output()
                .unwrap(),
        ),
        (
            "file",
            sb.old().arg("file").arg(&secret_file).output().unwrap(),
        ),
        ("list", sb.old().arg("list").output().unwrap()),
        (
            "list --json",
            sb.old().args(["list", "--json"]).output().unwrap(),
        ),
        ("load", sb.old().args(["load", "abcd"]).output().unwrap()),
        (
            "delete",
            sb.old().args(["delete", "abcd"]).output().unwrap(),
        ),
        (
            "prune",
            sb.old()
                .args(["prune", "--older-than", "1m", "--force"])
                .output()
                .unwrap(),
        ),
        ("check", sb.old().arg("check").output().unwrap()),
    ];
    for (what, out) in &runs {
        assert!(!out.status.success(), "{}", describe(what, out));
    }

    for (path, bytes) in files_below(&sb.path("store")) {
        for marker in [CLIPBOARD_MARKER, FILE_MARKER] {
            assert!(
                !contains(&bytes, marker),
                "{} holds `{marker}` written by v0.1.6",
                path.display()
            );
        }
    }
    assert_eq!(
        std::fs::read_to_string(sb.path("store/items")).unwrap(),
        STOP_TEXT,
        "the stop file must be left as it was"
    );
}

#[test]
#[ignore = "needs the v0.1.6 binary in PASSALONG_COMPAT_BIN; run `just test-compat`"]
fn compat_v016_cannot_write_into_a_store_whose_items_is_a_file() {
    let sb = Sandbox::new();
    std::fs::write(sb.path("store/items"), STOP_TEXT).unwrap();

    assert_old_client_is_refused(&sb);

    // Nothing but the stop file and, at most, the empty staging folder that
    // `check`'s write probe creates and cleans up.
    let mut names: Vec<String> = std::fs::read_dir(sb.path("store"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert!(
        names == ["items"] || names == ["items", "tmp"],
        "unexpected entries in the store root: {names:?}"
    );
    if names.len() == 2 {
        assert!(
            files_below(&sb.path("store/tmp")).is_empty(),
            "v0.1.6 left files in tmp/"
        );
    }
}

/// Encrypts a new store at `root` with the library, as `passalong encrypt`
/// does, and stores one item holding `text`.
fn encrypted_store(root: &Path, text: &str) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let fs = LocalFs::new(root);
        let key = DataKey::generate().unwrap();
        let words = Words::generate().unwrap();
        let wrapped = wrap(&key, &words, KdfParams::generate().unwrap()).unwrap();
        create_header(&fs, &StoreHeader::new(wrapped))
            .await
            .unwrap();
        write_stop_file(&fs).await.unwrap();
        let store = FsStore::sealed(
            fs,
            Arc::new(SystemClock),
            Box::new(StdRandom::new()),
            Sealer::new(key),
        );
        store
            .put(
                NewItem::text("compat"),
                Box::new(std::io::Cursor::new(text.as_bytes().to_vec())),
            )
            .await
            .unwrap();
    });
}

#[test]
#[ignore = "needs the v0.1.6 binary in PASSALONG_COMPAT_BIN; run `just test-compat`"]
fn compat_v016_cannot_write_into_an_encrypted_store() {
    let sb = Sandbox::new();
    let existing = "compat-existing-item-5d21";
    encrypted_store(&sb.path("store"), existing);

    assert_old_client_is_refused(&sb);

    for (path, bytes) in files_below(&sb.path("store")) {
        assert!(
            !contains(&bytes, existing),
            "{} holds an existing item's plaintext",
            path.display()
        );
    }
    let names: Vec<String> = std::fs::read_dir(sb.path("store"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert!(
        names
            .iter()
            .all(|name| ["encryption", "items", "tmp", "v2"].contains(&name.as_str())),
        "unexpected entries in the store root: {names:?}"
    );
}
