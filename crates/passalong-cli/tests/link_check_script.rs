//! Tests for `scripts/check-links.sh`, which `just check` runs. It fails
//! when a Markdown link points at a missing file or heading, when a link to
//! this repository's `main` branch names a missing path, and when a crate's
//! README, which crates.io shows, has a relative link. The script is a bash
//! script run by the Linux and macOS jobs, so the tests run on Unix only.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/check-links.sh")
}

fn run(root: &Path) -> (i32, String) {
    let out = Command::new("bash")
        .arg(script())
        .arg(root)
        .output()
        .expect("bash runs");
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), text)
}

/// A small repository whose links are all valid: a crate whose README is
/// the root README, and a docs folder.
struct Tree {
    dir: TempDir,
}

impl Tree {
    fn valid() -> Self {
        let tree = Self {
            dir: TempDir::new().unwrap(),
        };
        tree.write(
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/cli\"]\n\n[workspace.package]\nversion = \"1.0.0\"\nrepository = \"https://github.com/o/r\"\n",
        );
        tree.write(
            "crates/cli/Cargo.toml",
            "[package]\nname = \"cli\"\nreadme = \"../../README.md\"\n",
        );
        tree.write(
            "README.md",
            "# Demo\n\n## Quick start\n\nSee [usage](https://github.com/o/r/blob/main/docs/usage.md), [the service files](https://github.com/o/r/tree/main/docs/service), [below](#quick-start), and [elsewhere](https://example.com/x).\n",
        );
        tree.write(
            "docs/usage.md",
            "# Usage\n\n## `demo load <ID> [DEST]`\n\nBack to the [README](../README.md#quick-start), on to [notes](notes.md), see [loading](#demo-load-id-dest).\n\n```text\n[not a link](missing.md)\n```\n\nInline `[code](missing.md)` is not a link.\n",
        );
        tree.write("docs/notes.md", "# Notes\n\n![diagram](img/d.png)\n");
        tree.write("docs/img/d.png", "png");
        tree.write("docs/service/unit.service", "[Unit]\n");
        tree
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.path(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn check(&self) -> (i32, String) {
        run(self.dir.path())
    }
}

#[test]
fn valid_links_pass() {
    let (code, out) = Tree::valid().check();
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("links ok"), "{out}");
}

#[test]
fn a_relative_link_to_a_missing_file_fails_naming_it() {
    let tree = Tree::valid();
    tree.write("docs/notes.md", "# Notes\n\nSee [gone](gone.md).\n");
    let (code, out) = tree.check();
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("docs/notes.md:3"), "{out}");
    assert!(out.contains("gone.md"), "{out}");
}

#[test]
fn a_link_to_a_missing_heading_fails() {
    let tree = Tree::valid();
    tree.write(
        "docs/notes.md",
        "# Notes\n\nSee [setup](../README.md#server-setup) and [here](#nowhere).\n",
    );
    let (code, out) = tree.check();
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("server-setup"), "{out}");
    assert!(out.contains("nowhere"), "{out}");
}

#[test]
fn a_relative_link_in_a_crate_readme_fails_even_when_the_file_exists() {
    // crates.io resolves relative links against the crate's own folder in
    // the repository, where these files do not exist.
    let tree = Tree::valid();
    tree.write(
        "README.md",
        "# Demo\n\n## Quick start\n\nSee [usage](docs/usage.md) and [below](#quick-start).\n",
    );
    let (code, out) = tree.check();
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("README.md:5"), "{out}");
    assert!(out.contains("docs/usage.md"), "{out}");
    assert!(out.contains("crates.io"), "{out}");
    assert!(!out.contains("#quick-start"), "anchors are fine: {out}");
}

#[test]
fn a_link_to_a_missing_path_on_main_fails() {
    let tree = Tree::valid();
    tree.write(
        "README.md",
        "# Demo\n\nSee [usage](https://github.com/o/r/blob/main/docs/gone.md), [a tag](https://github.com/o/r/blob/v0.9.0/docs/old.md), and [another repo](https://github.com/x/y/blob/main/nope.md).\n",
    );
    let (code, out) = tree.check();
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("docs/gone.md"), "{out}");
    assert!(
        !out.contains("old.md"),
        "tag-pinned links are not checked: {out}"
    );
    assert!(
        !out.contains("nope.md"),
        "other repositories are not checked: {out}"
    );
}

#[test]
fn every_broken_link_is_reported_at_once() {
    let tree = Tree::valid();
    tree.write("docs/notes.md", "# Notes\n\n[a](a.md) [b](b.md)\n");
    let (code, out) = tree.check();
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("a.md") && out.contains("b.md"), "{out}");
}

#[test]
fn the_repository_passes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let (code, out) = run(&root);
    assert_eq!(code, 0, "{out}");
}
