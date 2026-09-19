# Developer guide

## Toolchain

- Rust 1.98.1, pinned in `rust-toolchain.toml` with `rustfmt`, `clippy`, and
  `llvm-tools-preview`. `rustup` installs it on first use.
- [`just`](https://github.com/casey/just) runs every task.
- Docker with the Compose plugin, for the SSH integration tests only.
- Optional: [`pre-commit`](https://pre-commit.com/).

One-time setup:

```sh
just setup            # llvm-tools-preview + cargo-llvm-cov
pre-commit install    # optional: run `just check` before every commit
```

## Repository layout

```text
crates/
├── passalong-core/   config, item model, storage (RemoteFs, FsStore, Store),
│                     clipboard trait, serve loop, telemetry, test doubles
├── passalong-ssh/    SftpFs over russh, host-key pinning, the `ssh` backend
├── passalong-https/  the `https` backend: a passalong-server workspace over
│                     its HTTPS API, with TLS pinning
└── passalong-cli/    package `passalong`, the binary: argument parsing, commands, output
docs/                 user and developer documentation; plans in docs/plans/
tests/docker/         OpenSSH server for the integration tests
```

## Recipes

| Recipe | Runs |
|---|---|
| `just fmt` | `cargo fmt --all` |
| `just fmt-check` | `cargo fmt --all -- --check` |
| `just lint` | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| `just test` | `cargo test --workspace --all-targets --all-features` |
| `just test-integration` | Starts the Docker OpenSSH server and runs the ignored tests, except `passalong-https`'s |
| `just server-build` | Builds passalong-server at the pinned commit into `target/passalong-server/` (see Server tests) |
| `just test-https` | `server-build`, then `passalong-https`'s ignored tests, each against a server of its own |
| `just test-deploy` | Starts `deploy/ssh-server` from a temporary directory and checks `init`, a round trip, host-owned storage, and a stable host key (Linux, Docker) |
| `just coverage` | `cargo llvm-cov --workspace --all-features --fail-under-lines 80 --summary-only` |
| `just coverage-full` | Coverage including the Docker-backed and server-backed tests |
| `just build` | `cargo build --workspace --all-features --locked` |
| `just links` | `scripts/check-links.sh`: relative links and heading anchors resolve, links to `main` name existing paths, and crate READMEs use only absolute links, because crates.io cannot resolve relative ones |
| `just check` | `fmt-check`, `lint`, `links`, `test`, `coverage`, `build` |
| `just audit` | `cargo deny check`: advisories, licences, duplicate crates, and sources per `deny.toml` |
| `just android-check` | `cargo check` of `passalong-core`, `passalong-ssh`, and `passalong-https` for `aarch64-linux-android` without default features; needs the Android NDK (see Android) |
| `just lint-workflows` | `actionlint` on the GitHub Actions workflows, or its Docker image when not installed |
| `just ci` | `check`, `audit`, `publish-dry-run`, `lint-workflows`, `test-integration`, `test-https`, `test-compat`, `test-deploy`, `coverage-full` |
| `just docker-build` | Builds the `passalong:dev` image |
| `just run <args>` | `cargo run -p passalong -- <args>` |
| `just publish-dry-run` | `cargo publish --workspace --dry-run --locked`: packages and verifies every crate without uploading |
| `just setup` | Installs coverage tooling, `cargo-deny`, and `actionlint` when Go is available |

CI runs `just ci` on Linux and `just check` on macOS, because GitHub's macOS
runners have no Docker. A third job runs the desktop clipboard tests under a
virtual X server (Xvfb), and a fourth runs `just android-check` with the
runner's preinstalled Android NDK.

## Android

There is no Android client yet, but `passalong-core` and `passalong-ssh`
must keep building for it: `just android-check` checks them for
`aarch64-linux-android` without default features, so without the desktop
clipboard (`arboard` has no Android support). CI runs it on every push;
running it locally needs:

1. The Rust target for the pinned toolchain:

   ```sh
   rustup target add aarch64-linux-android
   ```

2. The Android NDK, r26 or later. Install it with Android Studio's SDK
   Manager ("NDK (Side by side)") or with the command-line tools:

   ```sh
   sdkmanager "ndk;27.2.12479018"
   ```

3. `ANDROID_NDK_HOME` pointing at it, for example
   `~/Android/Sdk/ndk/27.2.12479018`. The recipe also accepts
   `ANDROID_NDK_LATEST_HOME`, which GitHub's runners set.

4. A Linux or macOS host with an x86_64 or Apple Silicon CPU, which the
   NDK's prebuilt `linux-x86_64` and `darwin-x86_64` toolchains cover.

The NDK is needed because `ring`, the crypto library under `russh`,
compiles C code for the target; the recipe points `cc` at the NDK's clang
for API level 24 (Android 7.0) and its `llvm-ar`. It only checks the code
(`cargo check`): nothing is linked or run on a device. `passalong-ssh`
uses `ring` rather than `aws-lc-rs` so that no CMake or extra toolchain is
needed.

## Windows

Windows x86_64 (`x86_64-pc-windows-msvc`) is built and tested by the
`windows` CI job, which runs what `just windows-check` runs: a build,
clippy with warnings as errors, and the tests. The SSH integration tests
need Docker, so they run on Linux only.

To build on Windows, install the toolchain above with the MSVC target and
the Microsoft C++ build tools, which ring needs for its C code. `just`
recipes run in Git Bash.

On Windows the config is looked for and written in `%APPDATA%\passalong\`,
the system config is under `%ProgramData%`, `serve`'s pid, log, and list
cache go to `%LOCALAPPDATA%\passalong\`, and `~` expands from
`%USERPROFILE%`. When one of those variables is unset, which a real Windows
system never does, the Unix rules apply. The crate-private `*_on(Platform)`
functions in `passalong-core/src/config.rs`, and
`StatePaths::resolve(env, Os::Windows)` in the CLI, let unit tests check
the Windows locations on every platform; the CLI tests point all of these
variables into their sandbox.

From Linux, only the core crate can be checked for Windows:
`rustup target add x86_64-pc-windows-msvc`, then
`cargo check --target x86_64-pc-windows-msvc -p passalong-core
--all-targets`. The SSH and CLI crates need the Windows SDK headers for
ring, so the CI job is their check.

## Test-driven workflow

1. Write a failing test for the next behaviour and run it; confirm it fails
   for the right reason.
2. Write the smallest code that passes it.
3. Refactor with the test green.
4. Run `just check` before committing.

Unit tests mock every external interface (clipboard, filesystem edges, remote
storage, clock, randomness, environment). Test doubles live in
`passalong_core::testing`, compiled for tests and behind the `testing`
feature for other crates' tests.

## SSH integration tests

Tests that need a real SSH server are `#[ignore]`d and read their connection
details from `PASSALONG_IT_SSH_*` variables. `just test-integration` generates
a throwaway key pair in `tests/docker/keys/` (git-ignored), pulls the
server image with up to 5 attempts because registries throttle shared CI
runners, starts `tests/docker/docker-compose.yml`, exports the variables,
runs the ignored tests, and always removes the container. At most 4
tests run at once, because the server drops new connections while too many
are still logging in.

## Server tests

The `https` backend is tested against a real
[passalong-server](https://github.com/joelee/passalong-server), not a mock:
its replays and its rewrite sessions are what a mock would get wrong. The
server is AGPL-3.0-or-later and this client Apache-2.0, so the server is
only ever *run*. Nothing of it is linked, copied, or depended on; the
client is written from the server's published API documents.

`just server-build` clones the server into `target/passalong-server/src`
once and builds the commit named by `server_commit` at the top of the
`justfile`, rebuilding only when that moves. `just test-https` then runs
the ignored tests of `crates/passalong-https/`. Each test starts its own
server (`tests/support/mod.rs`): a temporary `HOME`, `passalong-server
init`, a self-signed pair for `127.0.0.1` whose printed pin the client then
uses, a workspace and keys, and `serve` on a free port, stopped when the
test ends. `just ci` and `just coverage-full` include them, so the Linux CI
job runs them too.

To test against a newer server, move `server_commit` to that commit, run
`just test-https`, and read the server's changes to its `docs/api/` for
anything the client must follow.

## Compatibility test

Clients before v0.2.0 must never write into an encrypted store. An
encrypted store's root holds a regular file named `items` where older
clients expect their item directory, so their commands fail instead.
`crates/passalong-cli/tests/compat_v016.rs` proves this with the released
v0.1.6 binary: it runs every command that reads or writes items against
temporary stores and checks that each one fails at the store, not on its
arguments, that nothing it sent reaches the store, and that an encrypted
store's files are unchanged. A positive control runs the same `prune`
against a plaintext store, where it must prune.

The store format has not changed since v0.2.0, so the released v0.2.0
binary must keep working with stores this version changed.
`crates/passalong-cli/tests/compat_v020.rs` encrypts a store, changes its
words, rotates its key, and stores items with this version; v0.2.0 then
lists and prints every item and sends one that this version reads back.

The tests are ignored and need the old binaries in `PASSALONG_COMPAT_BIN`
and `PASSALONG_COMPAT_V020_BIN`; `just test-compat` downloads the v0.1.6
and v0.2.0 archives for Linux x86_64 or macOS arm64 once into
`target/compat/`, checks their SHA-256, and runs them. `just ci` includes
it. `serve` is not run, because it would read the real clipboard.

## Encryption tests

The `crypto` module is tested with round trips and with every kind of
tampering: truncated, reordered, repeated, or extended content, content
moved between items, and flipped bits. Tests use small Argon2 settings;
`cargo test --release -p passalong-core timing_ -- --ignored --nocapture`
measures the production settings. The rewrite engine's tests fail each
filesystem call of a whole migration and rotation in turn, then recover
both ways. The EFF word list in `crates/passalong-core/src/crypto/` must
stay unmodified: 7,776 lines of a dice code, a tab, and a word, and its
attribution in `NOTICE` and `crates/passalong-core/NOTICE`.

## Desktop clipboard test

The real clipboard adapter needs a desktop session, so its test is ignored
and excluded from the Docker-backed recipes. Run it by hand on a desktop:

```sh
cargo test -p passalong-core --all-features -- --ignored desktop_
```

`cargo build -p passalong-core --no-default-features` builds the core
without the desktop clipboard, as GUI and Android front-ends will.

## Coverage

`just coverage` measures line coverage without the Docker-backed tests and
must stay at 80 % or more. The SFTP code paths in `passalong-ssh` run only
against the real server, so `just coverage-full`, which `just ci` runs on
Linux, gives the complete figure. Both use the same 80 % gate.

## Container image

`just docker-build` builds `passalong:dev` from the `Dockerfile`: a Debian
trixie Rust builder and a slim trixie runtime running as the unprivileged
user `passalong`. `docker run --rm passalong:dev --help` is a quick smoke
test.

## Plans

Work is planned in numbered delivery plans under `docs/plans/`. Each plan's
Builder Work Log records per-step test evidence, verification results, and
deviations.

## RSA keys

RSA identity files and RSA host keys are behind the optional `rsa` feature
of `passalong-ssh`, which the `passalong` package forwards. The `rsa` crate
has a timing side channel with no fixed release (RUSTSEC-2023-0071), so
default builds, the release binaries, and `cargo install passalong` leave
it out, and `cargo deny` audits only the default features. In a default
build an RSA key fails with an error that names this feature.

```sh
cargo install --locked passalong --features rsa   # a build with RSA keys
```

`just lint` and `just test` also run the `passalong-ssh` checks without
the feature, and `just test-integration` logs in to the Docker server with
an RSA key under it.

## Duplicate dependencies

`cargo deny` rejects a crate that appears in two versions, so a new
duplicate is a decision instead of an accident. It checks the five
supported targets: Linux and macOS on x86_64 and aarch64, and Windows on
x86_64. Each duplicate
that cannot be avoided today has a `skip` entry in `deny.toml` naming the
older version and which dependency needs it.

When `just audit` reports a new duplicate:

1. Run `cargo tree -d` and `cargo tree -i <crate>@<version>` to see which
   dependency pulls in each version.
2. Try `cargo update` within the current requirements, or a newer release of
   the dependency that brings in the old version.
3. If neither helps, add a `skip` entry with `crate = "<name>@<version>"` and
   a `reason` naming the dependency path. Remove entries once upstream
   releases catch up; `cargo deny` warns about skips that no longer match.

ratatui, the terminal UI of `passalong choose`, accounts for the
`foldhash` and `hashbrown` skips: its layout solver `kasuari` needs
`hashbrown` 0.16, which no ratatui feature turns off.

## Publishing

Three crates are published to crates.io, in dependency order:
`passalong-core`, `passalong-ssh`, and `passalong`, the CLI package in
`crates/passalong-cli/`. `just publish-dry-run` packages and verifies all
three exactly as crates.io would, without uploading, and then removes the
unpacked packages in `target/package`. The published CLI installs with
`cargo install passalong`.

`.github/dependabot.yml` has Dependabot open a pull request each month
when an action used by the workflows has a new version. Cargo dependencies
are updated by hand and checked by `just audit`.

## Releasing

The steps, and who does each, are in the "Release workflow" section of
`AGENTS.md`. In short:

1. The plan's work bumps the version in `[workspace.package]` and the
   internal `version = "=X.Y.Z"` requirements, and drafts
   `docs/release/vX.Y.Z.md`.
2. After the work is approved, one `release: vX.Y.Z - <top feature>` commit
   moves the `CHANGELOG.md` `Unreleased` entries under
   `## vX.Y.Z - <UTC time>`, removes the draft line from the release notes,
   makes their links absolute URLs pinned to the tag, because the GitHub
   release page cannot resolve relative links, and removes pre-release
   wording from the README. This command must then
   pass:

   ```sh
   scripts/check-release-tag.sh vX.Y.Z
   ```

3. The branch is merged into `main` through a PR whose CI passes.
4. The merge commit on `main` is tagged, and the tag is pushed:

   ```sh
   git tag -a vX.Y.Z -m "passalong vX.Y.Z"
   git push origin vX.Y.Z
   ```

5. The workflow runs `scripts/check-release-tag.sh` again, which fails
   unless the tag matches the workspace version and the release records are
   final. It then runs `cargo publish --dry-run` and builds the binaries,
   each with a SHA-256 file: `.tar.gz` archives for Linux x86_64 and macOS
   arm64, and a `.zip` for Windows x86_64. Publishing waits until a maintainer
   approves the pending `release` deployment on the run's page. The workflow
   then publishes the three crates to crates.io, and only once that succeeds
   creates the GitHub release from `docs/release/vX.Y.Z.md` and attaches the
   binaries. Do not create the release by hand.
6. The Homebrew formula, `Formula/passalong.rb` in the
   [`joelee/homebrew-oss`](https://github.com/joelee/homebrew-oss) tap,
   builds the published crate. Once crates.io has the version, point it at
   the release from a clone of the tap next to this repository, then commit
   the change on a branch of the tap and push it:

   ```sh
   scripts/update-homebrew-formula.sh vX.Y.Z [TAP_DIR]
   ```

   The script sets only the formula's `url` and `sha256`, taking the
   checksum from crates.io, and fails when the version is not published
   yet. `TAP_DIR` defaults to `../homebrew-oss`. The tap's macOS workflow
   then builds, tests, and audits the formula. To check it on Linux, run
   the same `brew` commands in the `homebrew/brew` container image, after
   `brew update`. Homebrew 7 refuses formulae from a tap cloned from a
   local folder unless `HOMEBREW_NO_REQUIRE_TAP_TRUST=1` is set; users of
   the tap from GitHub run `brew trust joelee/oss` once instead.

Publishing needs a crates.io API token with the `publish-new` and
`publish-update` scopes, stored as the secret `CARGO_REGISTRY_TOKEN` of the
`release` environment (Settings, Environments, `release`). That environment
accepts only `v*` tags and requires a maintainer's approval, and the `v*`
tag ruleset limits who can create a release tag. A
published version cannot be replaced, only yanked, which is why the dry run
and the binary builds must pass first.

`just publish-dry-run` first removes earlier builds of the three workspace
crates, and their copies unpacked from cargo's temporary registries under
`~/.cargo/registry/src/-<hash>/`. The dry run compiles the packaged crates
as if downloaded from a registry, and cargo never rebuilds or re-unpacks a
registry crate with an unchanged version, so without this it can verify
against stale code and fail.
