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
