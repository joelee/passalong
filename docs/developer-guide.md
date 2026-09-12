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
| `just test-integration` | Starts the Docker OpenSSH server and runs the ignored tests |
| `just test-deploy` | Starts `deploy/ssh-server` from a temporary directory and checks `init`, a round trip, host-owned storage, and a stable host key (Linux, Docker) |
| `just coverage` | `cargo llvm-cov --workspace --all-features --fail-under-lines 80 --summary-only` |
| `just coverage-full` | Coverage including the Docker-backed tests |
| `just build` | `cargo build --workspace --all-features --locked` |
| `just check` | `fmt-check`, `lint`, `test`, `coverage`, `build` |
| `just audit` | `cargo deny check`: advisories, licences, duplicate crates, and sources per `deny.toml` |
| `just lint-workflows` | `actionlint` on the GitHub Actions workflows, or its Docker image when not installed |
| `just ci` | `check`, `audit`, `publish-dry-run`, `lint-workflows`, `test-integration`, `test-deploy`, `coverage-full` |
| `just docker-build` | Builds the `passalong:dev` image |
| `just run <args>` | `cargo run -p passalong -- <args>` |
| `just publish-dry-run` | `cargo publish --workspace --dry-run --locked`: packages and verifies every crate without uploading |
| `just setup` | Installs coverage tooling, `cargo-deny`, and `actionlint` when Go is available |

CI runs `just ci` on Linux and `just check` on macOS, because GitHub's macOS
runners have no Docker. A third job runs the desktop clipboard tests under a
virtual X server (Xvfb).

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
runs the ignored tests, and always removes the container.

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

## Publishing

Three crates are published to crates.io, in dependency order:
`passalong-core`, `passalong-ssh`, and `passalong`, the CLI package in
`crates/passalong-cli/`. `just publish-dry-run` packages and verifies all
three exactly as crates.io would, without uploading. The published CLI
installs with `cargo install passalong`.

## Releasing

Releases are cut by pushing a version tag; `.github/workflows/release.yml`
does the rest.

1. On a branch, set the new version in `[workspace.package]` and the
   internal `version = "=X.Y.Z"` requirements in `Cargo.toml`, move the
   `CHANGELOG.md` `Unreleased` entries under `## vX.Y.Z - <UTC time>`, and
   write `docs/release/vX.Y.Z.md`. Merge it to `main`.
2. Tag the merge commit and push the tag:

   ```sh
   git tag -a vX.Y.Z -m "passalong vX.Y.Z"
   git push origin vX.Y.Z
   ```

3. The workflow checks the tag against `Cargo.toml`
   (`scripts/check-release-tag.sh`), runs `cargo publish --dry-run`, builds
   Linux x86_64 and macOS arm64 binaries with SHA-256 files, creates the
   GitHub release from `docs/release/vX.Y.Z.md`, attaches the binaries, and
   publishes the three crates to crates.io.

Publishing needs a crates.io API token with the `publish-new` and
`publish-update` scopes, stored as the repository secret
`CARGO_REGISTRY_TOKEN` (Settings, Secrets and variables, Actions). A
published version cannot be replaced, only yanked, which is why the dry run
and the binary builds must pass first.

`just publish-dry-run` first removes earlier builds of the three workspace
crates. The dry run compiles the packaged crates as if downloaded from a
registry, and cargo never rebuilds a registry crate with an unchanged version,
so without the clean it can verify against stale code and fail.
