# passalong task runner. Every quality gate here wraps the exact commands
# mandated by AGENTS.md; pre-commit and CI call these recipes.

set shell := ["bash", "-euo", "pipefail", "-c"]

image := "passalong:dev"
compose := "tests/docker/docker-compose.yml"

# List available recipes
default:
    @just --list

# Install developer tooling: llvm-tools-preview and cargo-llvm-cov
setup:
    rustup component add llvm-tools-preview
    command -v cargo-llvm-cov >/dev/null || cargo install --locked cargo-llvm-cov

# Format all code in place
fmt:
    cargo fmt --all

# Check formatting
fmt-check:
    cargo fmt --all -- --check

# Lint with clippy, warnings are errors
lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Run unit and integration tests (Docker-backed tests are excluded)
test:
    cargo test --workspace --all-targets --all-features

# Run the Docker-backed SSH integration tests (ignored tests)
test-integration: (_with-sshd "cargo test --workspace --all-features -- --ignored --skip desktop_")

# Line coverage gate (>= 80%) without Docker-backed tests
coverage:
    cargo llvm-cov --workspace --all-features --fail-under-lines 80 --summary-only

# Line coverage gate including the Docker-backed tests
coverage-full: (_with-sshd "cargo llvm-cov --workspace --all-features --fail-under-lines 80 --summary-only -- --include-ignored --skip desktop_")

# Build the workspace from the lockfile
build:
    cargo build --workspace --all-features --locked

# All mandated checks: format, lint, tests, coverage, build
check: fmt-check lint test coverage build

# Full CI pipeline: all checks, then Docker-backed integration and coverage
ci: check test-integration coverage-full

# Build the container image
docker-build:
    docker build -t {{image}} .

# Run the CLI, e.g. `just run list`
run *ARGS:
    cargo run -p passalong-cli -- {{ARGS}}

# Start the throwaway OpenSSH server, export its connection details as
# PASSALONG_IT_SSH_* variables, run CMD, and always tear the server down.
[private]
_with-sshd CMD:
    #!/usr/bin/env bash
    set -euo pipefail
    keys=tests/docker/keys
    mkdir -p "$keys"
    if [ ! -f "$keys/id_ed25519" ]; then
        ssh-keygen -q -t ed25519 -N "" -C passalong-it -f "$keys/id_ed25519"
    fi
    chmod 644 "$keys/id_ed25519.pub"
    cleanup() { docker compose -f {{compose}} down -v --remove-orphans >/dev/null 2>&1 || true; }
    trap cleanup EXIT
    docker compose -f {{compose}} up -d --wait
    host_key="$(docker compose -f {{compose}} exec -T sshd cat /config/ssh_host_keys/ssh_host_ed25519_key.pub | awk '{print $1" "$2}')"
    export PASSALONG_IT_SSH_HOST=127.0.0.1
    export PASSALONG_IT_SSH_PORT=2222
    export PASSALONG_IT_SSH_USER=passalong
    export PASSALONG_IT_SSH_HOST_KEY="$host_key"
    export PASSALONG_IT_SSH_IDENTITY="$PWD/$keys/id_ed25519"
    export PASSALONG_IT_SSH_REMOTE_PATH=/config/passalong
    {{CMD}}
