# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## Unreleased

### Added

- Windows x86_64. The workspace builds, lints, and passes its tests on
  Windows in CI. The config is in `%APPDATA%\passalong`, the system config
  in `%ProgramData%\passalong`, `serve`'s files in
  `%LOCALAPPDATA%\passalong`, and `~` means `USERPROFILE` (PLAN-00009
  STEP-08).
- On Windows the key file, the folders created for it, and the list cache
  are open to their owner alone, set with `icacls`. A key file others may
  open is refused, with the `icacls` command that fixes it (PLAN-00009
  STEP-09).
- `serve --daemon`, `--status`, and `--stop` on Windows. `service-install`
  adds `serve` to the per-user Run key, or with `--scheduler` registers a
  Task Scheduler log-on task that restarts it after a failure;
  `service-remove` removes either (PLAN-00009 STEP-10).
- Release binaries for Windows x86_64:
  `passalong-<version>-x86_64-pc-windows-msvc.zip` and its `.sha256`
  (PLAN-00009 STEP-11).
- `encrypt --recover` also finishes or undoes an interrupted set-up, fresh
  start, or change of words. It restores a store whose header an
  interrupted v0.2.0 change left aside, once the words unlock it. It refuses
  to run while another recovery runs, and offers to take over one that
  stopped more than 10 minutes ago (PLAN-00009 STEP-01, STEP-02).
- `check` and `list` report what passalong 0.2.0 left unencrypted in `tmp/`
  when it encrypted a store, and unused journals; `prune --plain` removes
  them (PLAN-00009 STEP-04).
- Library, all additive:
  - in `passalong_core::encryption`: `Journal`, `HeaderChange`,
    `HeaderChangeKind`, `read_journal`, `RecoveryMarker`,
    `RECOVERY_STALE_SECS`, `recovery_in_progress`, `release_recovery`,
    `restore_header`, `Leftovers`, `leftovers`, and `remove_leftovers`;
  - `ListCache::identity_for`;
  - `RemoteFs` for `Arc<T>`.

### Changed

- The version is 0.2.1. `cargo semver-checks` finds no breaking change in
  `passalong-core` or `passalong-ssh` against 0.2.0, and the store layout
  is unchanged (PLAN-00009 STEP-11).
- File names from the store are made safe for Windows on every platform:
  `< > : " | ? *` become `_`, trailing dots and spaces are dropped, and
  reserved names such as `CON` or `aux.txt` get a leading `_` (PLAN-00009
  STEP-08).

### Fixed

Findings of Code Review 00002 (v0.2.0):

- A cut-short start of a re-encryption can no longer leave a lock with an
  empty or partial journal: the journal is written whole elsewhere and
  renamed into place (MED-01, PLAN-00009 STEP-01).
- Setting up encryption, a fresh start, and changing the words run under
  the same lock and journal as a re-encryption, so they never run at once
  and an interruption can be finished or undone (MAJ-04, PLAN-00009
  STEP-02).
- A store holding only part of an encrypted layout, as a synced folder can
  deliver it, is refused as broken and never opened as plaintext; an open
  plaintext store stops writing once encryption starts (MAJ-02, PLAN-00009
  STEP-03).
- Encrypting removes the plaintext staging folder, where uploads cut short
  could leave unencrypted content (MAJ-03, PLAN-00009 STEP-04).
- A send racing a key rotation can no longer store an item under the old
  key: the key id is read from the header before each write and again after
  publishing, and the item is taken back if the key changed; `serve` keeps
  the file and sends it again (MAJ-01, MED-02, PLAN-00009 STEP-05).
- `serve`'s list cache follows the store to a new key, and pull mode skips
  an encrypted item that does not open, instead of stopping at it; a recent
  one is tried again later (MED-03, MED-04, PLAN-00009 STEP-06).
- The key file's git work-tree check follows symbolic links, and the
  compatibility test runs v0.1.6's real `prune --yes`; the v0.2.0 release
  notes no longer claim a synced-folder check (MED-05, LOW-01, LOW-02,
  PLAN-00009 STEP-07).

## v0.2.0 - 2026-09-15T16:46:38Z

### Added

- `just test-compat` runs the released v0.1.6 binary against the layout of
  an encrypted store and proves it fails without writing item data; `just
  ci` runs it (PLAN-00008 STEP-01).
- `passalong_core::crypto`: the key hierarchy for encrypted stores (a random
  data key wrapped with Argon2id, 64 MiB, t=3, p=4, and AES-256-GCM), the
  keyed content key, sealed metadata, a chunked AES-256-GCM content format
  that rejects truncated, reordered, or extended content, and six-word
  passphrases from the EFF large word list (CC BY 4.0; see `NOTICE`)
  (PLAN-00008 STEP-02).
- `client.key_file` (default `store.key` beside the default config file)
  and the key file behind it: written with mode 0600 through a temporary
  file, and refused when other users can read it or when it is inside a git
  work tree that does not ignore it (PLAN-00008 STEP-03).
- `FsStore::sealed` keeps a store's items under `v2/` with sealed metadata
  (only the schema and id are readable), sealed chunked content, and ids
  made of keyed content keys; deduplication and id prefixes work as
  before. A recent sealed item that does not open yet is reported as not
  complete rather than corrupt, for stores in synced folders. `RemoteFs`
  gains `create_dir` (exclusive) and `remove_file`, and `Store` gains
  `key_id` and `content_key` (PLAN-00008 STEP-04).
- Every store now opens as its header, `encryption/header.json`, says:
  plaintext as before, sealed with this device's key, or refused with the
  command that fixes it (`encrypt --join` without the key or with another
  one, `encrypt --recover` while items are re-encrypted or the header is
  missing, and a refusal for a key with a plaintext store). A store opened
  sealed stops writing when its key changes or a re-encryption starts.
  `BackendRegistry` gains `register_fs` and `open_fs`, and `local` and `ssh`
  register their filesystems; `just test-compat` now also proves v0.1.6
  cannot write into a real encrypted store (PLAN-00008 STEP-05).
- `passalong encrypt`: on a plaintext store it shows six new words, has
  them typed back, and encrypts the store (a store with items gets a fresh
  start: they stay unencrypted in `plain/` until `passalong prune
  --plain`); on an encrypted store it changes the words without
  re-encrypting anything; `--join` gives a device the store's key. `init`
  offers encryption for an empty store and joins an encrypted one; `check`
  gains an `encryption` line; `list` warns while unencrypted items remain
  (PLAN-00008 STEP-06).
- `encrypt` offers to migrate a store's items, re-encrypting each one, as
  well as a fresh start; `encrypt --rotate` replaces the store's key and
  words and re-encrypts every item, so a lost device is shut out; and
  `encrypt --recover` finishes or undoes an interrupted re-encryption. The
  re-encryption holds a lock (`.rewrite/`), keeps each item's time and
  metadata, checks every copy before the new key takes over, and resumes
  without uploading anything twice (PLAN-00008 STEP-07).
- `serve` works with encrypted stores: the uploader looks for text and
  images already stored by the store's own content key, pull mode starts
  afresh instead of applying every item again when the store's key
  changes, and the list cache's identity names this device's key, so a
  cache from before a migration, rotation, or join is never used
  (PLAN-00008 STEP-08).

### Changed

- The version is 0.2.0. Encrypted stores use a new layout, and the
  library's public API breaks code built against 0.1: `ClientConfig` gains
  `key_file`, `StoreError` gains `Encryption`, and `StoreError`, `FsError`,
  `ConfigError`, `ModelError`, `Config`, `ClientConfig`, `ServerConfig`,
  `SshConfig`, `LocalConfig`, and `ServeConfig` are now
  `#[non_exhaustive]`, so later additions stop being breaking changes.
  Windows and S3 support move to v0.2.1 (PLAN-00008 STEP-10).
- `init` now inspects the store after its connection test, and its last
  line says how to encrypt or join it.
- A list cache written by v0.1.6 is read once more from the server, since
  the cache now names this device's key.

## v0.1.6 - 2026-09-14T21:14:49Z

### Added

- Homebrew: the `joelee/oss/passalong` formula builds the crates.io
  release, and `scripts/update-homebrew-formula.sh vX.Y.Z [TAP_DIR]` points
  it at a new one; the release workflow ends with that step (PLAN-00007
  STEP-06, STEP-07).
- `passalong choose` opens from a fresh list cache and says how old it
  is; `r` reloads the cache, and the new `R` reads the server and rewrites
  the cache (PLAN-00007 STEP-05).
- `passalong list` prints a fresh list cache without connecting, with the
  same output as from the server; `--nocache` reads the server and
  rewrites the cache. `file`, `clipboard`, `delete`, and `prune` add their
  own changes to the cache, and `load`, `cat`, and `get` refresh it after
  their output (PLAN-00007 STEP-04).
- `serve` keeps the list cache current for the ssh backend: at start and
  every `serve.list_cache_check_secs`, over a connection of its own
  (`passalong_core::cache::refresh_loop`, PLAN-00007 STEP-03).
- `passalong_core::cache::ListCache`: a local copy of a store's item list,
  refreshed with one id listing plus new metadata, and the `serve.list_cache`
  and `serve.list_cache_check_secs` settings (PLAN-00007 STEP-02).
- `passalong check` times its write probe, now 128 bytes, and reports the
  time and the rate it makes (PLAN-00007 STEP-01).

## v0.1.5 - 2026-09-14T14:26:22Z

### Added

- `passalong choose` opens a full-screen list of the stored items to
  filter and pick one, then load or print it, show its metadata in a
  scrollable dialog, or delete it and see the list read again; `?` shows
  the passalong version and every key (PLAN-00006 STEP-05).
- `passalong service-install` installs `serve` as a systemd user unit
  (Linux) or a launchd agent (macOS), enables it, and starts it, with
  `--no-start` and `--force`; `passalong service-remove` stops and removes
  it (PLAN-00005 STEP-06, renamed in PLAN-00006 STEP-03).
- `passalong check` checks the config, the connection with its pinned host
  key, and that the store can be read and written, one line per check
  (PLAN-00005 STEP-05); a last line reports whether `serve` is running
  (PLAN-00006 STEP-04).
- `Store::probe_write` and `WriteProbe` in `passalong-core`; the default
  reports `NotSupported`, so existing implementations keep compiling
  (PLAN-00005 STEP-05).
- `passalong get <ID>` prints an item's metadata without reading its
  content; `--json` prints the same object as `list --json` (PLAN-00005
  STEP-04).
- `-q`/`--quiet`: print nothing but errors and prompts, with logging at
  `error` unless a level is set explicitly; `cat` still prints the item
  (PLAN-00005 STEP-03).

### Changed

- The units in `docs/service/` run from the home directory, so a `~/.env`
  is found; the launchd agent logs to `~/Library/Logs/passalong/serve.log`,
  like `serve --daemon`, and is loaded with `launchctl bootstrap`
  (PLAN-00005 STEP-06).
- The README's "How it works" diagram is a Mermaid diagram (PLAN-00005
  STEP-02).

### Fixed

- An idle `serve` no longer keeps more than a CPU core busy: the drop
  folder watcher ignored nothing, so the folder being opened by each scan
  triggered the next scan. It now reacts only to files being created,
  written, renamed, or removed (PLAN-00006 STEP-01).
- `passalong cat` prints only the item: its "item printed" record is now at
  `verbose` level, so it no longer follows the content on the terminal
  (PLAN-00005 STEP-03).
- The links in the README work on crates.io: they are absolute, because
  crates.io resolved the relative ones against `crates/passalong-cli/`.
  `just check` now runs `scripts/check-links.sh`, which also fails on a
  missing link target or heading (PLAN-00005 STEP-02).
- The Release workflow no longer reports errors from the build cache's
  cleanup of `target/package`, and its artifact actions run on Node.js 24
  (`upload-artifact@v7`, `download-artifact@v8`). Dependabot proposes
  action updates monthly (PLAN-00005 STEP-01).

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
