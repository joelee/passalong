//! Tests for `scripts/check-release-tag.sh`, which the release workflow
//! runs before building or publishing anything.

use std::path::PathBuf;
use std::process::Command;

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

fn manifest(version: &str) -> tempfile::NamedTempFile {
    let file = tempfile::NamedTempFile::new().unwrap();
    let text = format!(
        "[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"{version}\"\nedition = \"2024\"\n\n[workspace.dependencies]\nversion_like = {{ version = \"9.9.9\" }}\n"
    );
    std::fs::write(file.path(), text).unwrap();
    file
}

#[test]
fn a_tag_matching_the_workspace_version_passes() {
    let m = manifest("1.2.3");
    let (code, out) = run(&["v1.2.3", m.path().to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("matches"), "{out}");
}

#[test]
fn mismatched_or_malformed_tags_fail() {
    let m = manifest("1.2.3");
    let path = m.path().to_str().unwrap();
    for tag in ["v1.2.4", "1.2.3", "v1.2", "release-1.2.3", "v1.2.3-rc.1"] {
        let (code, out) = run(&[tag, path]);
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
fn the_repository_version_matches_its_own_tag() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let tag = format!("v{}", env!("CARGO_PKG_VERSION"));
    let (code, out) = run(&[&tag, root.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
}
