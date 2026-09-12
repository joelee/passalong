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

## Recipes

| Recipe | Runs |
|---|---|
| `just fmt` | `cargo fmt --all` |
| `just fmt-check` | `cargo fmt --all -- --check` |
| `just lint` | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| `just test` | `cargo test --workspace --all-targets --all-features` |
| `just test-integration` | Starts the Docker OpenSSH server and runs the ignored tests |
| `just coverage` | `cargo llvm-cov --workspace --all-features --fail-under-lines 80 --summary-only` |
| `just coverage-full` | Coverage including the Docker-backed tests |
| `just build` | `cargo build --workspace --all-features --locked` |
| `just check` | `fmt-check`, `lint`, `test`, `coverage`, `build` |
| `just ci` | `check`, `test-integration`, `coverage-full` |
| `just docker-build` | Builds the `passalong:dev` image |
| `just run <args>` | `cargo run -p passalong-cli -- <args>` |
| `just setup` | Installs coverage tooling |

CI runs `just ci` on Linux and `just check` on macOS, because GitHub's macOS
runners have no Docker.

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
a throwaway key pair in `tests/docker/keys/` (git-ignored), starts
`tests/docker/docker-compose.yml`, exports the variables, runs the ignored
tests, and always removes the container.
