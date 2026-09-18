//! Compatibility: a store this version changed stays readable and writable
//! by the released v0.2.0 binary, which knows the same store format.
//!
//! This version encrypts a store, changes its words, rotates its key, and
//! stores items; v0.2.0 then lists and prints every item and sends one,
//! which this version reads back. The test is ignored by default and needs
//! the old binary in `PASSALONG_COMPAT_V020_BIN`; `just test-compat`
//! downloads the v0.2.0 release for this platform and runs it.

use std::path::PathBuf;
use std::process::Output;
use std::sync::Arc;

use assert_cmd::Command;
use passalong_core::clock::SystemClock;
use passalong_core::crypto::{DataKey, KDF_SALT_LEN, KdfParams, Words};
use passalong_core::encryption::{
    SystemGit, change_words, open_with_key, rotate, save_key_file, set_up,
};
use passalong_core::fs::LocalFs;
use passalong_core::model::NewItem;
use passalong_core::random::StdRandom;
use passalong_core::store::Store;
use tempfile::TempDir;
use tokio::io::AsyncReadExt;

struct Sandbox {
    dir: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        for sub in ["home", "work", "cfg", "store", "keys"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        Self { dir }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    fn key_file(&self) -> PathBuf {
        self.path("keys/store.key")
    }

    fn config(&self) -> PathBuf {
        let path = self.path("cfg/config.toml");
        let text = format!(
            "[client]\ndevice_name = \"compat-old\"\nkey_file = \"{}\"\n\n[server]\nkind = \"local\"\n\n[server.local]\npath = \"{}\"\n",
            self.key_file().display(),
            self.path("store").display()
        );
        std::fs::write(&path, text).unwrap();
        path
    }

    /// The old binary with a clean environment and the sandbox's config.
    fn old(&self) -> Command {
        let bin = std::env::var_os("PASSALONG_COMPAT_V020_BIN").expect(
            "PASSALONG_COMPAT_V020_BIN must name the v0.2.0 binary; run `just test-compat`",
        );
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

fn quick() -> KdfParams {
    KdfParams {
        m_kib: 64,
        t: 1,
        p: 1,
        salt: [8; KDF_SALT_LEN],
    }
}

/// Standard output of a run that must succeed.
fn succeed(what: &str, out: Output) -> Vec<u8> {
    assert!(
        out.status.success(),
        "{what}: exit {:?}\nstdout: {}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

async fn open(sb: &Sandbox, key: &DataKey) -> Box<dyn Store> {
    open_with_key(
        LocalFs::new(sb.path("store")),
        Some(key.clone()),
        Arc::new(SystemClock),
        Box::new(StdRandom::new()),
    )
    .await
    .unwrap()
}

async fn put(sb: &Sandbox, key: &DataKey, text: &str) {
    open(sb, key)
        .await
        .put(
            NewItem::text("compat-new"),
            Box::new(std::io::Cursor::new(text.as_bytes().to_vec())),
        )
        .await
        .unwrap();
}

#[test]
#[ignore = "needs the v0.2.0 binary in PASSALONG_COMPAT_V020_BIN; run `just test-compat`"]
fn compat_v020_reads_and_writes_a_store_this_version_changed() {
    let sb = Sandbox::new();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let texts = ["stored before new words", "stored after the rotation"];
    let key = runtime.block_on(async {
        let fs = LocalFs::new(sb.path("store"));
        let words: Vec<Words> = (0..3).map(|_| Words::generate().unwrap()).collect();
        let first = set_up(&fs, &words[0], quick()).await.unwrap();
        put(&sb, &first, texts[0]).await;
        change_words(&fs, &words[0], &words[1], quick())
            .await
            .unwrap();
        let key = rotate(&fs, &first, &words[2], quick(), Arc::new(SystemClock))
            .await
            .unwrap();
        put(&sb, &key, texts[1]).await;
        key
    });
    save_key_file(&sb.key_file(), &key, &SystemGit::new()).unwrap();

    // v0.2.0 lists and prints every item.
    let listed: serde_json::Value = serde_json::from_slice(&succeed(
        "list --json",
        sb.old().args(["list", "--json"]).output().unwrap(),
    ))
    .unwrap();
    let ids: Vec<String> = listed
        .as_array()
        .expect("a JSON array")
        .iter()
        .map(|item| item["id"].as_str().expect("an id").to_owned())
        .collect();
    assert_eq!(ids.len(), texts.len(), "{listed}");
    let mut printed: Vec<String> = ids
        .iter()
        .map(|id| {
            let out = succeed("cat", sb.old().args(["cat", id]).output().unwrap());
            String::from_utf8(out).unwrap().trim_end().to_owned()
        })
        .collect();
    printed.sort();
    let mut want: Vec<String> = texts.iter().map(|text| (*text).to_owned()).collect();
    want.sort();
    assert_eq!(printed, want);

    // v0.2.0 sends an item; this version reads it back.
    let sent = "sent by v0.2.0";
    succeed(
        "clipboard --stdin",
        sb.old()
            .args(["clipboard", "--stdin"])
            .write_stdin(sent)
            .output()
            .unwrap(),
    );
    let found = runtime.block_on(async {
        let store = open(&sb, &key).await;
        let mut found = Vec::new();
        for meta in store.list().await.unwrap() {
            let (_, mut content) = store.get(&meta.id).await.unwrap();
            let mut bytes = Vec::new();
            content.read_to_end(&mut bytes).await.unwrap();
            found.push(String::from_utf8(bytes).unwrap());
        }
        found
    });
    assert_eq!(found.len(), texts.len() + 1, "{found:?}");
    assert!(found.iter().any(|text| text == sent), "{found:?}");
}
