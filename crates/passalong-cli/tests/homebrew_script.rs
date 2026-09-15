//! Tests for `scripts/update-homebrew-formula.sh`, which points the Homebrew
//! tap's formula at a new release. They pass `--sha256`, so crates.io is
//! never asked. The script is a bash script for the macOS and Linux
//! releases, so the tests run on Unix only.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

/// The tap's formula as it was first published, at 0.1.5.
const FORMULA: &str = r##"class Passalong < Formula
  desc "Lightweight cross-platform clipboard and file sharing over SSH"
  homepage "https://github.com/joelee/passalong"
  url "https://static.crates.io/crates/passalong/passalong-0.1.5.crate"
  sha256 "53eb16a803db8a6ae0884fd5f1b8c5e317f3fd79a48ee303c30dc2dc4f89710f"
  license "Apache-2.0"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/passalong --version")
    (testpath/"config.toml").write <<~TOML
      [server.local]
      path = "#{testpath}/store"
    TOML
  end
end
"##;

const NEW_SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/update-homebrew-formula.sh")
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

/// A tap folder holding the formula.
struct Tap {
    dir: TempDir,
}

impl Tap {
    fn new() -> Self {
        let tap = Self {
            dir: TempDir::new().unwrap(),
        };
        std::fs::create_dir_all(tap.dir.path().join("Formula")).unwrap();
        std::fs::write(tap.formula(), FORMULA).unwrap();
        tap
    }

    fn root(&self) -> &str {
        self.dir.path().to_str().unwrap()
    }

    fn formula(&self) -> PathBuf {
        self.dir.path().join("Formula/passalong.rb")
    }

    fn text(&self) -> String {
        std::fs::read_to_string(self.formula()).unwrap()
    }

    fn update(&self, version: &str, sha: &str) -> (i32, String) {
        run(&["--sha256", sha, version, self.root()])
    }
}

#[test]
fn only_the_url_and_the_checksum_change() {
    let tap = Tap::new();
    let (code, out) = tap.update("v0.1.6", NEW_SHA);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("passalong 0.1.6"), "{out}");
    let expected = FORMULA
        .replace("passalong-0.1.5.crate", "passalong-0.1.6.crate")
        .replace(
            "53eb16a803db8a6ae0884fd5f1b8c5e317f3fd79a48ee303c30dc2dc4f89710f",
            NEW_SHA,
        );
    assert_eq!(tap.text(), expected);
}

#[cfg(unix)]
#[test]
fn the_formula_keeps_its_file_mode() {
    use std::os::unix::fs::PermissionsExt;
    let tap = Tap::new();
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    std::fs::set_permissions(tap.formula(), std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(tap.update("v0.1.6", NEW_SHA).0, 0);
    assert_eq!(mode(&tap.formula()), 0o644);
}

#[test]
fn malformed_versions_and_checksums_are_rejected() {
    let tap = Tap::new();
    for version in ["0.1.6", "v0.1", "v0.1.6-rc.1", "latest"] {
        let (code, out) = tap.update(version, NEW_SHA);
        assert_eq!(code, 1, "{version}: {out}");
        assert!(
            out.contains("is not of the form vMAJOR.MINOR.PATCH"),
            "{out}"
        );
    }
    for sha in ["abc", &NEW_SHA.to_uppercase(), &format!("{NEW_SHA}0")] {
        let (code, out) = tap.update("v0.1.6", sha);
        assert_eq!(code, 1, "{sha}: {out}");
        assert!(out.contains("is not a SHA-256 checksum"), "{out}");
    }
    assert_eq!(tap.text(), FORMULA, "nothing was changed");
}

#[test]
fn a_missing_tap_or_formula_is_rejected() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("no-tap");
    let (code, out) = run(&["--sha256", NEW_SHA, "v0.1.6", missing.to_str().unwrap()]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("tap directory"), "{out}");

    let (code, out) = run(&["--sha256", NEW_SHA, "v0.1.6", dir.path().to_str().unwrap()]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("Formula/passalong.rb does not exist"), "{out}");
}

#[test]
fn a_formula_without_exactly_one_url_and_checksum_is_rejected() {
    let tap = Tap::new();
    let doubled = FORMULA.replace(
        "  license",
        "  url \"https://example.invalid/other.crate\"\n  license",
    );
    std::fs::write(tap.formula(), &doubled).unwrap();
    let (code, out) = tap.update("v0.1.6", NEW_SHA);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("exactly one top-level url line"), "{out}");
    assert_eq!(tap.text(), doubled);
}

#[test]
fn missing_arguments_are_a_usage_error() {
    for args in [&[][..], &["--sha256"][..], &["--sha256", NEW_SHA][..]] {
        let (code, out) = run(args);
        assert_eq!(code, 2, "{args:?}: {out}");
        assert!(out.contains("usage:"), "{out}");
    }
}
