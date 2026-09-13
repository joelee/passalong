# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## Unreleased

## v0.1.3 - 2026-09-13T16:39:56Z

### Added

- `Store::get_meta` and `Store::list_ids` in `passalong-core`, both with
  default implementations, so existing `Store` implementations keep
  compiling (PLAN-00004 STEP-02 and STEP-04).

### Deprecated

- `Store::newest_id`, which pull mode no longer uses; use `list_ids`
  (PLAN-00004 STEP-04).
- `download::free_target`, whose name could be taken by another writer
  before it was written; use `download_into` (PLAN-00004 STEP-03).

### Fixed

- Copying an image in a browser sends the image, not the link the browser
  puts beside it: text that is only a link or an `<img>` tag no longer hides
  the image, and UTF-16 text read by mistake (`text/x-moz-url`) is never
  stored as text (PLAN-00004, post-gate fix).
- Pull mode applies every new item from another device once, even when that
  device's clock runs behind: it remembers the item ids it has handled
  instead of comparing creation times (PLAN-00004 STEP-04).
- Choosing between items that match an ambiguous id reads only the metadata
  of the items shown, not the whole store, which on an SSH server meant one
  request per stored item (Code Review 00001, REV-00001-MED-01; PLAN-00004
  STEP-02).
- Downloads by `load` and pull mode choose their file name only once the
  content is complete and verified, and link it into place without ever
  replacing a file, so two downloads of the same name at the same moment
  keep both files (Code Review 00001, REV-00001-LOW-01; PLAN-00004
  STEP-03).
- The release notes link to their delivery plans with absolute URLs pinned
  to the release tag, because GitHub release pages cannot resolve relative
  links. `scripts/check-release-tag.sh` now rejects relative links in
  release notes.

## v0.1.2 - 2026-09-13T09:04:23Z

### Added

- `passalong cat <ID>` prints an item to standard output exactly as stored,
  verifying its SHA-256; binary items are refused on a terminal unless
  `--force` is given (PLAN-00003 STEP-03).
- When an id prefix matches several items, `load`, `cat`, and `delete` list
  the matches on a terminal and ask which one you mean; scripts still get an
  error listing them (PLAN-00003 STEP-05).
- Clipboard images: `passalong clipboard` sends the image when the clipboard
  has no text, `load` puts it back on the clipboard (or writes the PNG with
  a destination), `cat` prints the PNG, and `list` shows kind `image`.
  Images are stored as PNG files, so older clients see ordinary files
  (PLAN-00003 STEP-06 and STEP-07).
- `serve` also sends clipboard images, reading one only when the clipboard
  holds no text and sending it once while it is unchanged; set
  `serve.clipboard_images = false` to turn this off (PLAN-00003 STEP-08).
- Pull mode: with `serve.pull = true`, `serve` applies items sent by other
  devices, the newest text or image to the clipboard and files into an
  existing `client.download_dir`, without ever sending them back
  (PLAN-00003 STEP-10).

### Changed

- RSA keys are now opt-in: RSA identity files and `ssh-rsa` host keys need
  a build with the `rsa` feature (`cargo install passalong --features rsa`),
  because the `rsa` crate has an unfixed timing side channel
  (RUSTSEC-2023-0071). Ed25519 and ECDSA keys work in every build
  (PLAN-00003 STEP-01).
- `passalong load <ID>` without a destination downloads file items into
  `client.download_dir` (`~/Downloads` by default) instead of refusing them;
  an existing name is kept and the download is numbered, as in
  `report (1).pdf`, unless `--force` is given (PLAN-00003 STEP-04).
- `scripts/check-release-tag.sh`, which the release workflow runs before
  building or publishing, also fails when `CHANGELOG.md` has no section for
  the version, the release notes are still a draft, or the README still
  describes the release as being prepared. `AGENTS.md` describes the
  release workflow step by step, and the README no longer states the release
  status.
- The release workflow publishes to crates.io only after a maintainer
  approves the `release` environment, which holds the crates.io token, and
  creates the GitHub release only after publishing succeeds. `CODEOWNERS`,
  `SECURITY.md`, and `CONTRIBUTING.md` prepare the project for other
  contributors.

## v0.1.1 - 2026-09-12T20:00:12Z

### Added

- `passalong delete <ID>...` removes items; every id is resolved first, so a
  typo deletes nothing (PLAN-00002 STEP-04).
- `passalong prune --older-than <AGE> --keep <N>` deletes old items after
  confirmation, supports `--dry-run` and `--yes`, and clears stale upload
  leftovers on the server (PLAN-00002 STEP-05).
- `passalong init` writes a config file for an SSH server, pinning its host
  key only after you confirm the fingerprint, and tests the connection
  (PLAN-00002 STEP-07).
- `passalong serve --daemon` runs `serve` in the background with a log file;
  `serve --status` and `serve --stop` manage it, and a second `serve` is
  refused (PLAN-00002 STEP-08).
- A guide and a tested Docker Compose example for running the passalong SSH
  server with the storage on the host: `docs/docker-ssh-server-setup.md`
  and `deploy/ssh-server/` (PLAN-00002 STEP-11).
- Tag-driven releases: pushing `vX.Y.Z` checks the tag, builds Linux x86_64
  and macOS arm64 binaries, creates the GitHub release, and publishes the
  crates to crates.io; CI also audits dependencies with `cargo deny` and
  runs the desktop clipboard tests under Xvfb (PLAN-00002 STEP-12).

### Changed

- The CLI package is now named `passalong` (the binary name is unchanged),
  and all crates carry crates.io metadata; third-party dependency
  requirements are caret requirements (PLAN-00002 STEP-02).

### Fixed

- Crate metadata now points at the correct repository,
  `https://github.com/joelee/passalong` (PLAN-00002 STEP-01).
- On Linux, text that `load` copies to the clipboard stays available after
  the command exits, without needing a clipboard manager (PLAN-00002
  STEP-09).
- A config location that exists but cannot be read is reported as such
  instead of "no config file found" (PLAN-00002 STEP-10).
- `serve` sends a dropped file it could not read once the file changes,
  instead of skipping it until restart (PLAN-00002 STEP-10).

## v0.1.0 - 2026-09-12T14:24:52Z

### Added

- Cargo workspace with `passalong-core`, `passalong-ssh`, and the `passalong`
  CLI; `just` recipes, pre-commit hook, GitHub Actions CI, Dockerfile, Docker
  OpenSSH test server, and the Apache-2.0 license (PLAN-00001 STEP-01).
- Syslog-compatible logging with five levels (`error`, `warning`, `info`,
  `verbose`, `debug`), per-operation correlation ids, and a field allow-list
  that keeps clipboard text, file contents, and secrets out of logs
  (PLAN-00001 STEP-02).
- Configuration from `config.toml` with the six-position lookup order,
  documented defaults, validation errors that name the key, `~` expansion,
  and the SSH key passphrase read only from the environment or `./.env`
  (PLAN-00001 STEP-03).
- Item model: time-sortable ids (`<hex seconds>-<content key>`), SHA-256
  content hashing, and the versioned `meta.json` schema (PLAN-00001 STEP-04).
- Storage layer: atomic publish through a staging directory, deduplication
  by content key, newest-first listing, and id prefix resolution, with a
  `local` backend for directories such as mounted shares (PLAN-00001
  STEP-05, STEP-06).
- Clipboard access for macOS and Linux (X11 and Wayland) behind a trait,
  so front-ends can supply their own (PLAN-00001 STEP-07).
- `passalong list` as a table or `--json`; global `--config` and
  `--log-level` options; one-line `error:` messages with exit codes 0, 1,
  and 2 (PLAN-00001 STEP-08).
- `passalong clipboard` (with `--stdin`) and `passalong file` send text and
  files and print the new item's id (PLAN-00001 STEP-09).
- `passalong load` copies an item to a file, a directory, or the clipboard,
  verifying its SHA-256 before anything is replaced (PLAN-00001 STEP-10).
- SSH/SFTP storage backend with strict host key pinning, public-key
  authentication, and a connect timeout (PLAN-00001 STEP-11).
- `server.kind = "ssh"` selects the SSH backend; backends are registered by
  kind, so new ones plug in without changing commands (PLAN-00001 STEP-12).
- `passalong serve` sends new clipboard text and dropped files, retries
  with reconnection when the server is unreachable, and stops cleanly on
  Ctrl-C or SIGTERM; systemd and launchd examples in `docs/service/`
  (PLAN-00001 STEP-13).
- Documentation: quick start, container use, command and configuration
  references, architecture and security model, developer guide, and
  backlog (PLAN-00001 STEP-14).
