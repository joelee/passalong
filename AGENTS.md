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
1. Use SemVer `vMAJOR.MINOR.PATCH`. If no version is given, increment PATCH. All workspace crates and their internal `=` requirements share the version; the plan's documentation step bumps it.
2. Agent completes the plan: all checks pass, coverage >= 80%, branch CI passes. Agent hands off with the evidence.
3. User reviews and approves the work.
4. Agent finalises the release in one commit, `release: vX.Y.Z - <top feature>`:
   - `CHANGELOG.md`: rename `Unreleased` to `vX.Y.Z - <UTC timestamp of this commit>` and add a fresh `Unreleased` above it.
   - `docs/release/vX.Y.Z.md`: remove the draft line; name the date, plan, and PR, not a commit hash.
   - `README.md` and other docs: remove pre-release wording such as "being prepared".
   - Run `scripts/check-release-tag.sh vX.Y.Z`, and suggest the PR title and description.
5. User verifies, pushes the branch, and opens a PR to `main`. `main` accepts only PRs whose Linux, macOS, and Xvfb checks pass.
6. Agent debugs PR CI failures on the branch. User gets the PR approved and merged.
7. User pulls `main`, tags the merge commit, and pushes the tag. The Release workflow checks the tag and release records and builds binaries. After the user approves the pending `release` deployment, it publishes to crates.io and then creates the GitHub release from `docs/release/vX.Y.Z.md`. Do not create the release by hand. A published crate version can only be yanked, never replaced.
8. User checks the release page and crates.io. Agent helps debug a failed release run.
