# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## Unreleased

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
