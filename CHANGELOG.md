# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## Unreleased

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
