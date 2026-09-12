# AGENTS.md - Rust/cargo/rustfmt

## Scope
Applies to this repo unless a deeper `AGENTS.md` overrides it. Follow explicit user instructions first.

## Stack
- Rust project managed by `cargo`; commit `Cargo.lock` for apps/bins.
- Format with `rustfmt`; lint with `clippy`; use `cargo` commands only unless docs specify stricter tools.
- Must run in containers: keep Docker/Colima-compatible build and runtime; avoid host-only paths.

## Non-negotiables
- TDD: write or update a failing unit test before production code; then implement; then refactor.
- Unit tests must mock external interfaces: HTTP, DB, queues, cloud APIs, filesystem edges, clocks, randomness, subprocesses.
- Every feature includes integration-test coverage planned up front and implemented before completion.
- Coverage gate: line coverage >= 80%; report coverage in completion notes.
- Add comments/rustdoc for public APIs, unsafe code, complex blocks, and non-obvious logic; avoid comments that restate code.
- Avoid `unsafe`; if unavoidable, justify with a safety comment and test coverage.
- No secrets in code, tests, docs, logs, or VCS.

## Commands
Use these unless project docs define stricter commands:
- Format check: `cargo fmt --all -- --check`
- Lint: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- Tests: `cargo test --workspace --all-targets --all-features`
- Coverage: `cargo llvm-cov --workspace --all-features --fail-under-lines 80 --summary-only`
- Build: `cargo build --workspace --all-features --locked`
- All checks: run format, lint, tests, coverage, build.
- Pre-commit must enforce the same checks. Keep `.pre-commit-config.yaml` or `.git/hooks/pre-commit` current; verify with `pre-commit run --all-files` when present.

## Config and secrets
- Secrets live only in `.env`; ensure `.env` is in `.gitignore`.
- Maintain `.env.sample` with supported variable names and safe example values.
- Non-secret config lives in `config.toml`.
- Config lookup order:
  1. `--config` argument
  2. `<APP_NAME>_CONFIG_FILE`
  3. `$XDG_CONFIG_HOME/<app-name>/config.toml`
  4. `$HOME/.config/<app-name>/config.toml`
  5. `/etc/<app-name>/config.toml`
  6. `./config.toml`
- Document all config in `docs/configuration.md` and keep defaults deterministic.

## Observability
- Logging is mandatory in core flows. Do not use `println!`/`eprintln!` for app logs except CLI output explicitly meant for users.
- Use syslog-compatible output and levels exposed as: Error, Warning, Info, Verbose, Debug.
- Map levels consistently: Error=err, Warning=warning, Info=info, Verbose=notice, Debug=debug.
- Logs must include timestamp, level, target/component, event/message, and correlation/request id when available.
- Never log secrets or full credentials.

## Docs to maintain
Update when behavior, commands, config, architecture, or user workflow changes:
- `README.md`
- `docs/architecture.md`
- `docs/configuration.md`
- `docs/usage.md`
- `docs/developer-guide.md`
- `docs/backlog.md`

## Backlog rules
- Track future work in `docs/backlog.md`.
- On completion, remove completed items from backlog.
- Add obvious follow-ups under `Agent suggested next steps`.

## New feature workflow
1. Run `git status --short`. If non-empty, stop and report dirty files; do not edit.
2. Create branch: `git switch -c feature/<feature_name>`.
3. Add a `CHANGELOG.md` entry under `Unreleased`.
4. Create plan: `docs/plans/<YYYYMMDDTHHMMSSZ>-<feature_name>.md`.
5. Plan must include scope, TDD unit tests, integration tests, config/secrets impact, observability, docs, coverage target, risks.
6. Write failing unit tests first; mock external interfaces.
7. Implement minimal code; add integration tests; update docs/config/backlog.
8. Run all checks and ensure coverage >= 80%.
9. Move plan to `docs/plans/done/<same-file>.md`; use `git mv` if tracked, else `mv`.
10. Report changed files, tests run, coverage result, docs updated, and backlog updates.

## Release workflow
1. Use SemVer `vMAJOR.MINOR.PATCH`.
2. If no version is provided, increment PATCH from `Cargo.toml`; if absent, start at `v0.1.0`.
3. Update `package.version` in `Cargo.toml` without the `v` prefix; in workspaces, update all released crates consistently.
4. Run all checks and ensure coverage >= 80%.
5. Rename `CHANGELOG.md` `Unreleased` section to `vX.Y.Z - <UTC timestamp>` and create a fresh `Unreleased` section above it.
6. Create `docs/release/vX.Y.Z.md` with summary, changes, tests, coverage, config changes, and upgrade notes.
7. Suggest commit message: `release: vX.Y.Z - <top feature>`.
