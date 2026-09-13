//! Tests for `scripts/check-release-tag.sh`, which the release workflow
//! runs before building or publishing anything. It checks that the tag
//! matches the workspace version and that the release records are final.

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/check-release-tag.sh")
}

fn run(args: &[&str]) -> (i32, String) {
    let out = Command::new("bash")
        .arg(script())
        .args(args)
        .output()
        .expect("bash runs");
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), text)
}

/// A repository tree whose release records are final for `version`.
struct Tree {
    dir: TempDir,
}

impl Tree {
    fn finalised(version: &str) -> Self {
        let tree = Self {
            dir: TempDir::new().unwrap(),
        };
        tree.write(
            "Cargo.toml",
            &format!(
                "[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"{version}\"\nedition = \"2024\"\n\n[workspace.dependencies]\nversion_like = {{ version = \"9.9.9\" }}\n"
            ),
        );
        tree.write(
            "CHANGELOG.md",
            &format!("# Changelog\n\n## Unreleased\n\n## v{version} - 2026-09-13T08:00:00Z\n\n### Added\n\n- Something.\n"),
        );
        tree.write(
            &format!("docs/release/v{version}.md"),
            &format!("# passalong v{version}\n\nReleased 2026-09-13.\n\n## Summary\n"),
        );
        tree.write(
            "README.md",
            "# passalong\n\n> **Status:** see the releases page.\n",
        );
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

    fn check(&self, tag: &str) -> (i32, String) {
        run(&[tag, self.path("Cargo.toml").to_str().unwrap()])
    }
}

#[test]
fn a_matching_tag_with_final_records_passes() {
    let (code, out) = Tree::finalised("1.2.3").check("v1.2.3");
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("matches the workspace version"), "{out}");
    assert!(out.contains("release records are final"), "{out}");
}

#[test]
fn mismatched_or_malformed_tags_fail() {
    let tree = Tree::finalised("1.2.3");
    for tag in ["v1.2.4", "1.2.3", "v1.2", "release-1.2.3", "v1.2.3-rc.1"] {
        let (code, out) = tree.check(tag);
        assert_eq!(code, 1, "{tag}: {out}");
        assert!(out.contains("error:"), "{tag}: {out}");
    }
}

#[test]
fn a_missing_tag_is_a_usage_error() {
    let (code, out) = run(&[]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("usage:"), "{out}");
}

#[test]
fn a_changelog_without_the_version_section_fails() {
    let tree = Tree::finalised("1.2.3");
    tree.write(
        "CHANGELOG.md",
        "# Changelog\n\n## Unreleased\n\n### Added\n\n- Something.\n\n## v1.2.2 - 2026-09-01T00:00:00Z\n",
    );
    let (code, out) = tree.check("v1.2.3");
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("CHANGELOG.md has no"), "{out}");
}

#[test]
fn a_missing_or_draft_release_document_fails() {
    let tree = Tree::finalised("1.2.3");
    std::fs::remove_file(tree.path("docs/release/v1.2.3.md")).unwrap();
    let (code, out) = tree.check("v1.2.3");
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("docs/release/v1.2.3.md is missing"), "{out}");

    tree.write(
        "docs/release/v1.2.3.md",
        "# passalong v1.2.3\n\nDraft for the `v1.2.3` tag.\n\n## Summary\n",
    );
    let (code, out) = tree.check("v1.2.3");
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("is still marked as a draft"), "{out}");
}

#[test]
fn pre_release_wording_in_the_readme_fails() {
    for phrase in ["v1.2.3 is being prepared.", "v1.2.3 is Not Yet Released."] {
        let tree = Tree::finalised("1.2.3");
        tree.write(
            "README.md",
            &format!("# passalong\n\n> **Status:** {phrase}\n"),
        );
        let (code, out) = tree.check("v1.2.3");
        assert_eq!(code, 1, "{phrase}: {out}");
        assert!(out.contains("README.md still says"), "{phrase}: {out}");
    }
}

#[test]
fn every_problem_is_reported_at_once() {
    let tree = Tree::finalised("1.2.3");
    tree.write("CHANGELOG.md", "# Changelog\n\n## Unreleased\n");
    tree.write(
        "docs/release/v1.2.3.md",
        "# passalong v1.2.3\n\nDraft for the `v1.2.3` tag.\n",
    );
    tree.write("README.md", "v1.2.3 is being prepared.\n");
    let (code, out) = tree.check("v1.2.3");
    assert_eq!(code, 1, "{out}");
    for expected in [
        "CHANGELOG.md has no",
        "still marked as a draft",
        "README.md still says",
    ] {
        assert!(out.contains(expected), "{expected}: {out}");
    }
}

#[test]
fn the_repository_manifest_is_understood() {
    let version = env!("CARGO_PKG_VERSION");
    let tree = Tree::finalised(version);
    let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    std::fs::copy(real, tree.path("Cargo.toml")).unwrap();
    let (code, out) = tree.check(&format!("v{version}"));
    assert_eq!(code, 0, "{out}");
}
