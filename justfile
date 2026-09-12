# passalong task runner. Every quality gate here wraps the exact commands
# mandated by AGENTS.md; pre-commit and CI call these recipes.

set shell := ["bash", "-euo", "pipefail", "-c"]

image := "passalong:dev"
compose := "tests/docker/docker-compose.yml"

# List available recipes
default:
    @just --list

# Install developer tooling: llvm-tools-preview, cargo-llvm-cov, cargo-deny, actionlint (needs Go)
setup:
    rustup component add llvm-tools-preview
    command -v cargo-llvm-cov >/dev/null || cargo install --locked cargo-llvm-cov
    command -v cargo-deny >/dev/null || cargo install --locked cargo-deny
    command -v actionlint >/dev/null || ! command -v go >/dev/null || GOBIN="$HOME/.local/bin" go install github.com/rhysd/actionlint/cmd/actionlint@v1.7.12

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

# Prove deploy/ssh-server works end to end: start it from a temporary
# directory, run `init` and a round trip, check host-owned storage and a
# stable host key across re-creation. Linux and Docker only.
test-deploy:
    #!/usr/bin/env bash
    set -euo pipefail
    root="$PWD"
    work="$(mktemp -d)"
    export PASSALONG_SSH_PORT=2223 PASSALONG_STORAGE="$work/storage" PUID="$(id -u)" PGID="$(id -g)"
    compose() { docker compose -f "$root/deploy/ssh-server/compose.yaml" --project-directory "$work" -p passalong-deploy-test "$@"; }
    cleanup() {
        img="$(compose config --images 2>/dev/null | head -1 || true)"
        compose down -v --remove-orphans >/dev/null 2>&1 || true
        # Some files under config/ may belong to the container's root user.
        if ! rm -rf "$work" 2>/dev/null && [ -n "$img" ]; then
            docker run --rm --entrypoint rm -v "$work:/w" "$img" -rf /w/config /w/storage /w/keys >/dev/null 2>&1 || true
            rm -rf "$work" || true
        fi
    }
    trap cleanup EXIT
    mkdir -p "$work/keys" "$work/storage"
    ssh-keygen -q -t ed25519 -N "" -C deploy-test -f "$work/client_key"
    cp "$work/client_key.pub" "$work/keys/deploy-test.pub"
    for attempt in 1 2 3 4 5; do
        if compose pull --quiet; then break; fi
        if [ "$attempt" -eq 5 ]; then echo "error: could not pull the SSH server image" >&2; exit 1; fi
        sleep $((attempt * 10))
    done
    compose up -d --wait
    # The same command the guide gives for reading the fingerprint.
    fingerprint() { compose exec -T passalong-sshd ssh-keygen -lf /config/ssh_host_keys/ssh_host_ed25519_key.pub | awk '{print $2}'; }
    fp="$(fingerprint)"
    cargo build -q --bin passalong
    pa() { env -u XDG_CONFIG_HOME -u XDG_STATE_HOME -u PASSALONG_CONFIG_FILE HOME="$work" "$root/target/debug/passalong" --config "$work/client.toml" "$@"; }
    pa init --host 127.0.0.1 --port 2223 --user passalong --identity-file "$work/client_key" --remote-path /data --device-name deploy-test --fingerprint "$fp" --yes
    id="$(echo "hello from the deploy test" | pa clipboard --stdin)"
    pa list | grep -q "$id"
    mkdir -p "$work/out"
    pa load "$id" "$work/out" >/dev/null
    grep -qx "hello from the deploy test" "$work/out/$id.txt"
    meta="$(find "$work/storage/items" -name meta.json | head -1)"
    [ -n "$meta" ] || { echo "error: no item in the host storage directory" >&2; exit 1; }
    [ "$(stat -c %u "$meta")" = "$(id -u)" ] || { echo "error: stored files are not owned by UID $(id -u)" >&2; exit 1; }
    compose down
    compose up -d --wait
    [ "$(fingerprint)" = "$fp" ] || { echo "error: the host key changed after re-creating the container" >&2; exit 1; }
    pa list | grep -q "$id"
    echo "deploy example OK: init, round trip, host-owned storage, host key $fp unchanged after re-creation"

# Supply-chain audit: advisories, licences, bans, and sources (deny.toml)
audit:
    cargo deny check

# Lint the GitHub Actions workflows, with a local actionlint or its image
lint-workflows:
    if command -v actionlint >/dev/null; then actionlint; else docker run --rm -v "$PWD:/repo" -w /repo rhysd/actionlint:1.7.12 -color; fi

# Build the workspace from the lockfile
build:
    cargo build --workspace --all-features --locked

# All mandated checks: format, lint, tests, coverage, build
check: fmt-check lint test coverage build

# Full CI pipeline: all checks, then Docker-backed integration and coverage
ci: check audit publish-dry-run lint-workflows test-integration test-deploy coverage-full

# Build the container image
docker-build:
    docker build -t {{image}} .

# Package and verify every crate for crates.io without uploading
publish-dry-run *ARGS:
    # Verification compiles the packaged crates as registry dependencies, and
    # cargo never rebuilds a registry crate whose version is unchanged, so
    # clear earlier builds of the workspace crates first.
    cargo clean -p passalong-core -p passalong-ssh -p passalong
    cargo publish --workspace --dry-run --locked {{ARGS}}

# Run the CLI, e.g. `just run list`
run *ARGS:
    cargo run -p passalong -- {{ARGS}}

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
    # Registries throttle shared CI runners, so retry the image pull before
    # starting the server.
    for attempt in 1 2 3 4 5; do
        if docker compose -f {{compose}} pull --quiet; then
            break
        fi
        if [ "$attempt" -eq 5 ]; then
            echo "error: could not pull the SSH test image after $attempt attempts" >&2
            exit 1
        fi
        echo "image pull failed (attempt $attempt of 5); retrying in $((attempt * 10)) s" >&2
        sleep $((attempt * 10))
    done
    docker compose -f {{compose}} up -d --wait
    host_key="$(docker compose -f {{compose}} exec -T sshd cat /config/ssh_host_keys/ssh_host_ed25519_key.pub | awk '{print $1" "$2}')"
    export PASSALONG_IT_SSH_HOST=127.0.0.1
    export PASSALONG_IT_SSH_PORT=2222
    export PASSALONG_IT_SSH_USER=passalong
    export PASSALONG_IT_SSH_HOST_KEY="$host_key"
    export PASSALONG_IT_SSH_IDENTITY="$PWD/$keys/id_ed25519"
    export PASSALONG_IT_SSH_REMOTE_PATH=/config/passalong
    {{CMD}}
