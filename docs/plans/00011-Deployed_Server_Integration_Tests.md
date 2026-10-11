---
title: "Delivery Plan 00011: Deployed Server Integration Tests"
aliases:
  - "Plan 00011"
tags:
  - delivery-plan
  - implementation
  - claude-code
type: delivery-plan
plan_id: "PLAN-00011"
plan_status: approved              # draft | approved | cancelled
plan_kind: initial                 # initial | superseding
created_at: "2026-10-11T12:24:44Z"
approved_at: "2026-10-11T13:23:08Z"
planner_agent: "Claude Code"
planner_model: "anthropic/claude-fable-5-1"
triggered_by: user                 # user | agent:<agent-name>
request_kind: direct               # idea | review | idea-and-review | direct | unplanned-query
repository: "joelee/passalong"
baseline_branch: "feature/deployed-server-integration-tests"
baseline_commit: "ec99f50d8e179b2e7eee7049d7f5acd40957bfb1"
source_ideas: []
source_reviews: []
previous_plan: null
requirements_count: 14
steps_count: 7
acceptance_criteria_count: 15
blocking_decisions: 0
build_ready: true
web_research_used: false
confidence: high                  # high | medium | low

# Builder-maintained front matter. Builder may update only these keys after
# explicit user approval; Delivery Planner initializes them.
implementation_status: in-progress # not-started | in-progress | blocked | completed | abandoned
builder_agent: "Claude Code"
builder_model: "anthropic/claude-fable-5-1"
execution_branch: "feature/deployed-server-integration-tests"
execution_started_at: "2026-10-11T13:23:08Z"
execution_updated_at: "2026-10-11T13:27:56Z"
execution_completed_at: null
current_step: "PLAN-00011-STEP-03"
---

# Delivery Plan 00011: Deployed Server Integration Tests

> [!abstract] Plan status: `approved`
> A suite of ignored integration tests runs the released client against the
> passalong-server deployed on phoenix (the "Passalong HTTPS Server" note),
> reading its connection details and secrets from `.env` as
> `INTEGRATION_TEST_*` variables, through `just test-deployed`. No user
> decision is open; the server, a dedicated `integration-test` workspace,
> and two never-expiring keys are in place and were used for a successful
> round trip on 2026-10-11.

## 1. Objective and outcome

The `https` backend is tested today only against a server each test starts
itself on `127.0.0.1` (`just test-https`). Nothing exercises a *deployed*
server: Docker, a published image, a self-signed certificate with several
names, a port forwarded by Docker, a Tailscale route, keys that expire or
never do, and a workspace quota set by an operator. This plan adds a suite
that does, against the real deployment on phoenix, so that an upgrade of the
image, the client, the firewall, or the certificate is caught by a command
rather than by a phone.

After the plan, `just test-deployed` loads `.env`, runs every
`deployed_*` test with one thread, and passes on a machine on the tailnet.
The `home` workspace and its three device keys are never touched: the suite
uses the `integration-test` workspace (256 MiB quota) created for it, and
throwaway workspaces it creates and deletes over SSH for the destructive
tests (encryption, revocation).

## 2. Source traceability

| Requirement | Source | Source location | Interpretation |
|---|---|---|---|
| PLAN-00011-REQ-01 | User | User, 2026-10-11: "plan for some integration tests to test the `passalong-server` set up as documented in … Passalong HTTPS Server.md" | Ignored tests against the deployed server, run by a `just` recipe |
| PLAN-00011-REQ-02 | User; repository | User: "Save the server connection details and secrets in `.env` with `INTEGRATION_TEST_` prefix"; `AGENTS.md` § Config and secrets | Variables in `.env`, names in `.env.sample`, loaded by the recipe |
| PLAN-00011-REQ-03 | Note; repository | Note § Verification performed (pin recomputed, SANs confirmed, `check` all `ok`); `crates/passalong-https/tests/server.rs` | TLS and identity tests against the deployment |
| PLAN-00011-REQ-04 | Note; repository | Note § Verification performed (`file` → `list` → `cat` → `diff` byte-identical, attribution, delete, 0 items); `crates/passalong-cli/tests/cli_https_backend.rs` | CLI round trips |
| PLAN-00011-REQ-05 | User answer | D-02: a read-only key for refusal tests | Read-only key behaviour |
| PLAN-00011-REQ-06 | User answer | D-03: "Quota and limits" | Quota refusal on the 256 MiB workspace |
| PLAN-00011-REQ-07 | User answer | D-03: "`serve` round trip" | `serve` against the deployment |
| PLAN-00011-REQ-08 | User answer | D-03: "Encryption cycle" | The encryption cycle on a throwaway workspace |
| PLAN-00011-REQ-09 | User answer | D-03: "Admin via SSH" | Admin commands over SSH during tests |
| PLAN-00011-REQ-10 | User answer | D-01: a dedicated workspace; `home` untouched | Isolation and clean-up |
| PLAN-00011-REQ-11 | Repository | `justfile` `test-integration`, `test-https`, `coverage-full` run ignored tests by name filters; CI runners are not on the tailnet | The suite is excluded from every existing recipe and from CI |
| PLAN-00011-REQ-12 | Repository | `AGENTS.md` § Docs to maintain, § Backlog rules, § New feature workflow step 3 | Documentation, CHANGELOG, backlog |
| PLAN-00011-REQ-13 | Repository | `AGENTS.md` § Non-negotiables ("No secrets in … logs"); PLAN-00010-AC-08 | No secret in output or logs |
| PLAN-00011-REQ-14 | Repository | `AGENTS.md` § Non-negotiables (coverage gate, TDD) | Gates unchanged and reported |

## 3. Repository baseline

| Field | Value |
|---|---|
| Repository | `joelee/passalong` |
| Branch | `feature/deployed-server-integration-tests` (from `main`) |
| HEAD | `ec99f50d8e179b2e7eee7049d7f5acd40957bfb1` |
| Working tree at publication | Clean, apart from this allocated plan file |
| Applicable instructions | `AGENTS.md` (root), `docs/plans/AGENTS.md`, `docs/developer-guide.md` § Server tests |

Verified at the baseline, 2026-10-11:

- The deployment answers `GET /readyz` with 204 on `https://phoenix.kiko-walleye.ts.net:8445`, `https://100.72.227.19:8445`, and `https://192.168.0.121:8445`; its pin is `sha256//MOMQGz8V/Pd+JZPCaRvUfQtU9wSMKsJ3IRpOjDCg0s=`; the image is `joeworks/passalong-server:0.1.0`, the same release as `server_commit` in the `justfile`.
- `ssh agent@phoenix.kiko-walleye.ts.net` works without a prompt, and `docker compose exec -T server passalong-server …` in `/home/agent/services/passalong-https` runs the admin commands.
- Workspace `integration-test` (id `ed69e8c8bb89e863`, quota 256 MiB) exists with keys `91009089d4f8` (`integration-test`, read-write, never expires) and `b496adf90e4b` (`integration-test-ro`, read-only, never expires). Their secrets are in `.env` only (mode 0600, git-ignored). Two earlier keys whose secrets were lost were revoked and deleted.
- With those values, `passalong init --yes`, `check` (every line `ok`), a text and a file round trip (`load` and `cat` byte-identical, device attributed), `get`, `list --json`, and `delete` succeeded from this machine with the v0.3.0 client built at `HEAD`. `check` also passed on both alternative URLs under the pin; a wrong pin failed the `server` line naming the presented pin; a 300 MiB sparse file was refused in about 4 s with `QUOTA_EXCEEDED`, the message naming the item's 314572800 bytes and the quota of 268435456 bytes, before any content was sent. The read-only key passed `check` with `storage write  n/a` and was refused a send with `FORBIDDEN_ROLE`; an unknown key failed the `server` line with `UNAUTHENTICATED`.

## 4. Scope

### In scope

- A `just test-deployed` recipe that loads `.env`, checks the variables, and runs the `deployed_*` tests of `passalong-https` and `passalong` with `--test-threads=1`.
- `.env.sample` entries for every `INTEGRATION_TEST_*` variable.
- A shared support module for the deployed server: variables, device configs, an SSH admin helper, throwaway workspaces.
- Tests: TLS and identity; `init --yes` and `check`; text and file round trips with every one-shot command; duplicate content; read-only key; wrong key; quota; `serve` upload and pull; revoked key; the encryption cycle.
- `--skip deployed_` in `test-integration`, `test-https`, and `coverage-full`.
- Developer guide, README, CHANGELOG, backlog.

### Out of scope

- Any change to the client's behaviour. A test that finds a client bug is recorded as a blocker; fixing it is another plan.
- Any change on phoenix beyond workspaces and keys made by the suite itself: no firewall, compose, or image change.
- Running the suite in GitHub Actions.
- The SSH deployment on port 2222 (`just test-integration` already covers an SSH server).
- Windows: the suite is Unix-only like `cli_https_backend.rs`'s `serve` tests; the rest is not gated but not promised.

## 5. Constraints and preserved decisions

- The server is AGPL-3.0-or-later and is only ever run; nothing of it is copied or linked (PLAN-00010 D-04). The suite runs it over the network and over SSH.
- Secrets live in `.env` only, which `.gitignore` already lists; `.env.sample` holds names and safe examples (`AGENTS.md`).
- `home` and its keys `b2c38ff363e8`, `6437a8b21c81`, `25dddb1677f7` are never used, listed, or changed by a test.
- Never `docker compose down --volumes`, `--user`, or `docker compose cp` on phoenix (note § Container hardening, § Persistence).
- Tests are `#[ignore]`d, named `deployed_*`, and fail with a message naming the missing variable and `just test-deployed`, like `PASSALONG_SERVER_BIN` does in `tests/support/mod.rs`.
- `encrypt` needs a terminal, so the encryption cycle is driven through `HttpEncryptionAdmin` and `EncryptionAdmin`, as `cli_https_backend.rs` does, with the CLI for sending, `check`, `list`, and `prune --plain`.
- Existing recipes and CI stay green without network access to phoenix.

## 6. Assumptions

None. Unresolved matters are recorded as decisions and block approval when
material.

## 7. Decisions and blockers

| ID | Decision or blocker | Resolution | Owner | Status |
|---|---|---|---|---|
| D-01 | Which workspace the tests use | A new `integration-test` workspace with a 256 MiB quota; `home` untouched. Created 2026-10-11 | User, 2026-10-11 | Resolved |
| D-02 | Which API keys | Two new never-expiring keys: `integration-test` (read-write) and `integration-test-ro` (read-only). Created 2026-10-11 | User, 2026-10-11 | Resolved |
| D-03 | Scope beyond round trips and TLS | All four: encryption cycle, quota, `serve`, admin over SSH | User, 2026-10-11 | Resolved |
| D-04 | Server URL in `.env` | `https://phoenix.kiko-walleye.ts.net:8445`; the two IPs in `INTEGRATION_TEST_SERVER_ALT_URLS` for the SAN check | User, 2026-10-11 | Resolved |
| D-05 | Where destructive tests run | An encrypted workspace cannot be returned to plaintext, and a device key kept between runs would be lost on another machine; so each run of an encryption or revocation test creates a throwaway workspace `it-<8 hex>` and its keys over SSH, and deletes them when done. These tests skip, passing with a message, when `INTEGRATION_TEST_SSH_HOST` is unset | Planner, 2026-10-11 | Resolved |
| D-06 | How the suite stays out of CI | Test names start with `deployed_`; `test-integration`, `test-https`, and `coverage-full` add `--skip deployed_`; `test-deployed` is not in `ci` | Planner, 2026-10-11 | Resolved |
| D-07 | Loading `.env` | The recipe sources `.env` with `set -a` in its own shell, like `_with-sshd` exports its variables; `dotenv-load` is not switched on for every recipe | Planner, 2026-10-11 | Resolved |

## 8. Affected architecture and components

Test code and tooling only; no crate source changes.

| Path | Change |
|---|---|
| `crates/passalong-https/tests/support/deployed.rs` | New. `Deployed::from_env()` (URL, alternative URLs, pin, workspace, keys, optional SSH host and compose dir), `config(dir, url, pin, key)`, `Admin` (runs `passalong-server` over SSH, redacting secrets from panics), `ThrowawayWorkspace` (create on `new`, delete on `Drop`, with a read-write key made with `--json`) |
| `crates/passalong-https/tests/deployed.rs` | New. Library-level tests: pin forms, wrong pin, SANs, wrong key, read-only viewer, workspace facts |
| `crates/passalong-cli/tests/cli_deployed_server.rs` | New. CLI tests: `init --yes`, `check`, round trips, quota, `serve`, revocation, the encryption cycle |
| `justfile` | `test-deployed`; `--skip deployed_` in `test-integration`, `test-https`, `coverage-full` |
| `.env.sample` | The `INTEGRATION_TEST_*` names with safe examples |
| `docs/developer-guide.md` | § Deployed server tests |
| `README.md`, `CHANGELOG.md`, `docs/backlog.md` | Pointers, Unreleased entry, backlog |

Variables, all read by `Deployed::from_env()`:

| Variable | Secret | Meaning |
|---|---|---|
| `INTEGRATION_TEST_SERVER_URL` | no | The server, `https://phoenix.kiko-walleye.ts.net:8445` |
| `INTEGRATION_TEST_SERVER_ALT_URLS` | no | Comma-separated other URLs the certificate names |
| `INTEGRATION_TEST_TLS_PIN` | no | `passalong-server tls fingerprint` on the server |
| `INTEGRATION_TEST_WORKSPACE` | no | `integration-test` |
| `INTEGRATION_TEST_API_KEY` | yes | Read-write key of that workspace |
| `INTEGRATION_TEST_API_KEY_READ_ONLY` | yes | Read-only key of that workspace |
| `INTEGRATION_TEST_SSH_HOST` | no | `user@host` for admin commands; unset skips them |
| `INTEGRATION_TEST_SSH_COMPOSE_DIR` | no | The compose project directory on that host |

## 9. Requirement catalogue

### PLAN-00011-REQ-01 — A suite against the deployed server

- **Requirement:** `just test-deployed` runs ignored tests named `deployed_*` in `passalong-https` and `passalong` against the server named by `INTEGRATION_TEST_SERVER_URL`, with `--test-threads=1`, and exits non-zero when any fails.
- **Rationale:** The deployment, not a local server, is what devices use.
- **Source:** User, 2026-10-11.
- **Acceptance evidence:** AC-01.

### PLAN-00011-REQ-02 — Connection details and secrets in `.env`

- **Requirement:** The recipe loads `.env` from the repository root; every variable is prefixed `INTEGRATION_TEST_`; `.env.sample` lists each with a safe example; a missing required variable fails the test with its name and the recipe to run.
- **Rationale:** User instruction; `AGENTS.md` § Config and secrets.
- **Source:** User; `AGENTS.md`.
- **Acceptance evidence:** AC-03, AC-04.

### PLAN-00011-REQ-03 — TLS and identity

- **Requirement:** Tests show the pinned server answers with the pin in `sha256/` and `sha256//` forms; a wrong pin fails before any request, naming the server's pin; every URL in `INTEGRATION_TEST_SERVER_ALT_URLS` presents the same pin; an unknown key makes one request and fails `UNAUTHENTICATED`; the viewer reports `integration-test` as read-write and never expiring.
- **Rationale:** The note's verification of pin and SANs, made repeatable.
- **Source:** Note § Verification performed; `tests/server.rs`.
- **Acceptance evidence:** AC-05, AC-06.

### PLAN-00011-REQ-04 — CLI round trips

- **Requirement:** From a temporary `HOME`, `init --url --tls-pin --api-key-file --yes` writes a config and an owner-only key file; `check` passes with the expected eight lines; `clipboard --stdin` and `file` round-trip through `list`, `list --json`, `get`, `cat`, and `load` byte-identically with the device name attributed; `delete` removes them and the workspace's item count returns to what it was; identical content sent twice is stored once.
- **Rationale:** The note's manual round trip, made repeatable.
- **Source:** Note § Verification performed, § Client usage; `cli_https_backend.rs`.
- **Acceptance evidence:** AC-07, AC-08.

### PLAN-00011-REQ-05 — Read-only key

- **Requirement:** With `INTEGRATION_TEST_API_KEY_READ_ONLY`, `check` shows the key as read-only with `n/a` for the write probe, `list` works, and a send is refused with `FORBIDDEN_ROLE` and stores nothing.
- **Rationale:** D-02.
- **Source:** User answer; `docs/usage.md` § `passalong check`.
- **Acceptance evidence:** AC-06.

### PLAN-00011-REQ-06 — Quota

- **Requirement:** A file larger than the workspace's remaining quota is refused with a message naming `QUOTA_EXCEEDED`, the item's size, and the quota in bytes, and the item count is unchanged. The server refuses at `beginUpload`, before content is sent (verified 2026-10-11), so a sparse file costs no bandwidth; the test asserts the refusal takes under 30 s.
- **Rationale:** D-03; an operator-set quota is what the deployment has that a test server does not.
- **Source:** User answer; PLAN-00010-AC-15.
- **Acceptance evidence:** AC-09.

### PLAN-00011-REQ-07 — `serve`

- **Requirement:** On Unix, `serve` with a drop folder sends a dropped file, which appears in `list` and moves to `sent/`; in pull mode it downloads an item sent under a second device config into the download directory; both stop cleanly.
- **Rationale:** D-03.
- **Source:** User answer; `cli_https_backend.rs` `Serving`.
- **Acceptance evidence:** AC-10.

### PLAN-00011-REQ-08 — The encryption cycle

- **Requirement:** On a throwaway workspace: a device sets up encryption and sends an item; a second device joins with the words and reads it; the server reports the workspace sealed; a rotation re-encrypts the item and the un-joined device is told to join; a rotation interrupted part-way is recovered by the device that began it; every item loads with its original SHA-256; the workspace is deleted afterwards.
- **Rationale:** D-03, D-05.
- **Source:** User answer; PLAN-00010-AC-10..12.
- **Acceptance evidence:** AC-12.

### PLAN-00011-REQ-09 — Admin over SSH

- **Requirement:** A helper runs `passalong-server` admin commands on the host as `ssh $INTEGRATION_TEST_SSH_HOST 'cd $INTEGRATION_TEST_SSH_COMPOSE_DIR && docker compose exec -T server passalong-server …'`; tests use it to create and delete throwaway workspaces and keys and to revoke a key mid-test; a revoked key is refused from the next request on; `serve` exits non-zero with a message after its key is revoked. Keys made at runtime are read from `--json` output and never written to disk outside the test's temporary directory.
- **Rationale:** D-03.
- **Source:** User answer; note § Admin.
- **Acceptance evidence:** AC-11.

### PLAN-00011-REQ-10 — Isolation and clean-up

- **Requirement:** No test names, lists, or changes the `home` workspace or its keys; every test deletes the items it created and every throwaway workspace and key, on failure too (`Drop`); tests tolerate items left by an aborted earlier run by comparing counts relative to the start, and the recipe prunes `integration-test` to zero items before the run.
- **Rationale:** D-01.
- **Source:** User answer.
- **Acceptance evidence:** AC-08, AC-11, AC-15.

### PLAN-00011-REQ-11 — Out of every other recipe and CI

- **Requirement:** `cargo test --workspace --all-features`, `just test-integration`, `just test-https`, and `just coverage-full` run no `deployed_*` test; `just ci` is unchanged; the GitHub workflows are unchanged.
- **Rationale:** Runners cannot reach the tailnet.
- **Source:** `justfile`; `.github/workflows`.
- **Acceptance evidence:** AC-02.

### PLAN-00011-REQ-12 — Documentation

- **Requirement:** `docs/developer-guide.md` gains "Deployed server tests" (what the suite needs, the variables, how to make a key and the workspace on the server, what it leaves behind, why it is not in CI); `README.md` points to it; `CHANGELOG.md` Unreleased records the suite; `docs/backlog.md` loses nothing it covers and gains follow-ups.
- **Rationale:** `AGENTS.md` § Docs to maintain.
- **Source:** `AGENTS.md`.
- **Acceptance evidence:** AC-14.

### PLAN-00011-REQ-13 — No secrets in output

- **Requirement:** The captured stdout and stderr of a full CLI session at `debug` log level, and every panic message of the support module, contain no `pal_<id>_<secret>`; the test asserts it. Assertions compare key ids, never secrets.
- **Rationale:** `AGENTS.md` § Non-negotiables.
- **Source:** `AGENTS.md`; PLAN-00010-AC-08.
- **Acceptance evidence:** AC-13.

### PLAN-00011-REQ-14 — Gates

- **Requirement:** `just check` passes; line coverage stays at or above 80 % and is reported in the hand-off; `just test-deployed` passes on the planner's machine and the result is recorded in the work log.
- **Rationale:** `AGENTS.md`.
- **Source:** `AGENTS.md` § Non-negotiables, § Commands.
- **Acceptance evidence:** AC-01, AC-14.

## 10. Delivery strategy

Tooling first, then tests in order of blast radius, each step red before
green.

1. **STEP-01** makes the suite runnable: the recipe, the variables, the
   support module, one smoke test, and the name filters that keep it out of
   every other recipe. From here every later test is one `just
   test-deployed` away.
2. **STEP-02..03** cover what the note verified by hand: TLS, identity, and
   the round trips, on the persistent `integration-test` workspace.
3. **STEP-04** adds the quota and `serve` tests on the same workspace.
4. **STEP-05** adds the SSH admin helper and throwaway workspaces, and the
   revocation tests that need them.
5. **STEP-06** adds the encryption cycle on a throwaway workspace.
6. **STEP-07** documents, runs every gate, checks `home` is untouched, and
   hands off.

Each test cleans up in `Drop`, so a failing step leaves at most a throwaway
workspace, which the next run's prune and the step's rollback remove.

## 11. Detailed implementation steps

### PLAN-00011-STEP-01 — Recipe, variables, support module

- **Objective:** `just test-deployed` runs one passing smoke test against phoenix, and no other recipe runs it.
- **Requirements:** `PLAN-00011-REQ-01`, `PLAN-00011-REQ-02`, `PLAN-00011-REQ-11`
- **Depends on:** None
- **Affected components:** `justfile`, `.env.sample`, `CHANGELOG.md`, `crates/passalong-https/tests/support/deployed.rs`, `crates/passalong-https/tests/deployed.rs`
- **Preconditions:** Plan approved; `.env` filled (it is).
- **Test or evidence first:** `deployed_the_server_answers_the_viewer_and_workspace`: with `Deployed::from_env()` and the read-write key, `Client::viewer()` names key id `91009089d4f8` as read-write with no expiry, and `workspace()` names `integration-test` with a 256 MiB quota. It fails first because the module does not exist; then because the variables are unset when run without the recipe, with a message naming the variable.
- **Implementation tasks:**
  1. `CHANGELOG.md` Unreleased: "Added: `just test-deployed`, a suite against a deployed passalong-server …".
  2. `.env.sample`: the eight variables with comments and safe examples (`https://box.example:8443`, `sha256/AAAA…`, `pal_000000000000_example`).
  3. `support/deployed.rs`: `Deployed::from_env()` reading the variables (a missing required one panics: "`INTEGRATION_TEST_SERVER_URL` is not set: run these tests with `just test-deployed`"); `config(dir, url, pin, key) -> Config` mirroring `config_for`; `Deployed::redact(text)` replacing any `pal_<id>_<secret>` with `pal_<id>_…`.
  4. `justfile`: `test-deployed` (bash shebang; `set -a; . ./.env; set +a`; check the required variables; build `passalong`; prune `integration-test` to zero items through a temporary config; run `cargo test -p passalong-https --all-features --test deployed -- --ignored --test-threads=1` and `cargo test -p passalong --all-features --test cli_deployed_server -- --ignored --test-threads=1`); `--skip deployed_` in `test-integration`, `test-https`, and `coverage-full`.
- **Documentation/configuration/operations:** `.env.sample`; developer-guide section drafted, committed in STEP-07.
- **Verification:** `just test-deployed` passes; `cargo test --workspace --all-features` lists the new test as ignored; `just test-https` passes unchanged.
- **Completion criteria:** The smoke test passes through the recipe and fails clearly without it.
- **Rollback or recovery:** Revert the commit.
- **Builder stop conditions:** The recipe cannot reach the server (readyz not 204) or the viewer fails: report, do not change `.env`.

### PLAN-00011-STEP-02 — TLS and identity

- **Objective:** The note's pin and SAN verification as tests.
- **Requirements:** `PLAN-00011-REQ-03`, `PLAN-00011-REQ-05`, `PLAN-00011-REQ-13`
- **Depends on:** STEP-01
- **Affected components:** `crates/passalong-https/tests/deployed.rs`
- **Preconditions:** STEP-01 committed.
- **Test or evidence first:** Each test below, written red first against a deliberately wrong assertion, then corrected:
  `deployed_the_pin_works_in_either_form`; `deployed_a_wrong_pin_fails_before_any_request_naming_the_right_one` (transport error, not retryable, reason contains the pin, under one second); `deployed_every_name_on_the_certificate_presents_the_same_pin` (viewer succeeds with the pin on each alternative URL); `deployed_an_unknown_key_is_refused_once` (`Code::Unauthenticated`); `deployed_the_read_only_key_sees_but_cannot_write` (viewer role `ReadOnly`, a write refused with the `FORBIDDEN_ROLE` code, item count unchanged).
- **Implementation tasks:** Write the tests with the support module; no production code.
- **Documentation/configuration/operations:** None.
- **Verification:** `just test-deployed`.
- **Completion criteria:** All pass; none mentions a secret in its panic (assert with `redact` on failure paths).
- **Rollback or recovery:** Revert the commit.
- **Builder stop conditions:** An alternative URL is unreachable from the builder's network (LAN IP from outside): record it and make the SAN test report which URL failed, then stop and ask whether to drop that URL from `.env`.

### PLAN-00011-STEP-03 — CLI round trips

- **Objective:** `init`, `check`, and every one-shot command against the deployment.
- **Requirements:** `PLAN-00011-REQ-04`, `PLAN-00011-REQ-05`, `PLAN-00011-REQ-10`, `PLAN-00011-REQ-13`
- **Depends on:** STEP-01
- **Affected components:** `crates/passalong-cli/tests/cli_deployed_server.rs`
- **Preconditions:** STEP-01 committed.
- **Test or evidence first:** `deployed_init_yes_then_check_passes` (config and 0600 key file written; the eight `check` lines: `server` names `passalong-server 0.1.0, API v1, TLS pinned`; `api key` is `integration-test (read-write), never expires`; `workspace` names `integration-test` and `256.0 MiB`; `encryption off`; `storage write ok`); `deployed_text_and_file_round_trip_byte_identically` (clipboard and a 1 MiB random file; `list --json` device name; `get` sha256 equals the local SHA-256; `cat` and `load` bytes equal; `delete`; count restored); `deployed_identical_content_is_stored_once`; `deployed_the_read_only_key_checks_lists_and_is_refused_to_send` (`check` write line `n/a`, send fails with `FORBIDDEN_ROLE`, count unchanged); `deployed_no_secret_reaches_the_output` (the full session above at `PASSALONG_LOG_LEVEL=debug`, output contains no secret).
- **Implementation tasks:** A `device(deployed, dir, key, name)` writer and a `passalong(dir, config)` runner as in `cli_https_backend.rs`, with `PASSALONG_LOG_LEVEL` controlled; items deleted in a guard that runs on panic.
- **Documentation/configuration/operations:** None.
- **Verification:** `just test-deployed`.
- **Completion criteria:** All pass; `integration-test` holds 0 items after the run (`workspace` line of a final `check`).
- **Rollback or recovery:** Revert the commit; delete leftover items with `prune --keep 0 --yes`.
- **Builder stop conditions:** A `check` line differs from the documented format: that is a client or docs bug, report it.

### PLAN-00011-STEP-04 — Quota and `serve`

- **Objective:** The operator-set quota and `serve` against the deployment.
- **Requirements:** `PLAN-00011-REQ-06`, `PLAN-00011-REQ-07`, `PLAN-00011-REQ-10`
- **Depends on:** STEP-03
- **Affected components:** `crates/passalong-cli/tests/cli_deployed_server.rs`
- **Preconditions:** STEP-03 committed.
- **Test or evidence first:** `deployed_a_file_over_the_quota_is_refused_naming_it` (a sparse file of quota + 1 MiB; `file` fails with a message containing `QUOTA_EXCEEDED` and `268435456`; count unchanged; under 30 s); `deployed_serve_sends_a_dropped_file` (Unix; `Serving` from `cli_https_backend.rs` reused through a shared module or copied with attribution; file appears in `list`, moved to `sent/`); `deployed_serve_pulls_an_item_from_another_device` (Unix; second device config under the same key with another device name sends; pull mode downloads it).
- **Implementation tasks:** Move `Serving`, `quick_serve`, and `wait_for_file` into `crates/passalong-https/tests/support/cli.rs` so both CLI test files share them, if that keeps `cli_https_backend.rs` unchanged in behaviour; otherwise copy.
- **Documentation/configuration/operations:** None.
- **Verification:** `just test-deployed`; `just test-https` still passes.
- **Completion criteria:** All pass; the quota test's duration is recorded in the work log.
- **Rollback or recovery:** Revert; `prune --keep 0 --yes` on `integration-test`; `prune --plain` if an aborted upload left `plain/` items.
- **Builder stop conditions:** The refusal takes over 30 s or the server log shows content received: report; the plan does not change the client.

### PLAN-00011-STEP-05 — Admin over SSH and revocation

- **Objective:** Throwaway workspaces and keys made and removed over SSH; a revoked key refused.
- **Requirements:** `PLAN-00011-REQ-09`, `PLAN-00011-REQ-10`, `PLAN-00011-REQ-13`
- **Depends on:** STEP-03
- **Affected components:** `crates/passalong-https/tests/support/deployed.rs` (`Admin`, `ThrowawayWorkspace`), `crates/passalong-cli/tests/cli_deployed_server.rs`
- **Preconditions:** STEP-03 committed; `INTEGRATION_TEST_SSH_HOST` set.
- **Test or evidence first:** `deployed_a_throwaway_workspace_is_created_and_deleted` (after `Drop`, `workspace list --json` over SSH no longer names it, and `key list --json` names none of its keys); `deployed_a_revoked_key_is_refused_from_the_next_request` (a key made at runtime works for `check`, is revoked over SSH, the next `check` fails on the `server` line); `deployed_serve_stops_after_its_key_is_revoked` (Unix; `serve` running, key revoked, exits non-zero within one retry with a message naming the key). Each returns early with `eprintln!("skipped: INTEGRATION_TEST_SSH_HOST is not set")` when SSH is not configured.
- **Implementation tasks:**
  1. `Admin::run(&[&str]) -> String`: `ssh -o BatchMode=yes <host> 'cd <dir> && docker compose exec -T server passalong-server <args>'`, with `--json` where listings are parsed; panics are passed through `redact`.
  2. `ThrowawayWorkspace::new(admin, quota)` names `it-<8 hex from the clock and pid>`, creates it with `--quota 64MiB`, creates a read-write `--never --json` key labelled `it`; `Drop` runs `workspace delete <name>` (which removes its keys), best effort.
  3. `Admin::home_snapshot()`: `workspace show home --json` item count and `key list --json` ids for `home`, used by STEP-07's evidence.
- **Documentation/configuration/operations:** Developer guide: the SSH requirement, how to point the suite at another host, what a crashed run may leave (`it-*` workspaces) and the one command that removes them.
- **Verification:** `just test-deployed`; after it, `workspace list` over SSH shows only `home` and `integration-test`.
- **Completion criteria:** All pass, including a run with `INTEGRATION_TEST_SSH_HOST` unset where the three tests report skipped and pass.
- **Rollback or recovery:** Revert; `workspace delete it-*` for leftovers.
- **Builder stop conditions:** `workspace delete` refuses an encrypted or non-empty workspace: report; do not escalate privileges or use another command on the host.

### PLAN-00011-STEP-06 — The encryption cycle

- **Objective:** Set-up, join, rotate, and recovery on a throwaway workspace of the deployment.
- **Requirements:** `PLAN-00011-REQ-08`, `PLAN-00011-REQ-10`, `PLAN-00011-REQ-13`
- **Depends on:** STEP-05
- **Affected components:** `crates/passalong-cli/tests/cli_deployed_server.rs`
- **Preconditions:** STEP-05 committed.
- **Test or evidence first:** `deployed_encryption_set_up_join_rotate_and_recover` (skipped without SSH), in order: device A sets up encryption through `HttpEncryptionAdmin` as `cli_https_backend.rs` does and sends a text and a file; `workspace show --json` over SSH reports the workspace sealed; device B without a key fails `check` naming `encrypt --join`; device B joins with the words and `cat`s both items; device A rotates; device B's `check` names `encrypt --join` again, and after joining reads both items; a rotation begun by device A through the admin's `Rewrite` and dropped after the first staged item is recovered by device A (finish), after which `list` shows the same ids and `load` of each matches its original SHA-256; the workspace is dropped. A second test, `deployed_a_fresh_start_leaves_plain_items_until_pruned`, covers the fresh start and `prune --plain`.
- **Implementation tasks:** Reuse the admin and words helpers of `cli_https_backend.rs` (shared module from STEP-04, or copies).
- **Documentation/configuration/operations:** Developer guide: that the encryption tests never run on `integration-test`.
- **Verification:** `just test-deployed`.
- **Completion criteria:** Both pass; no `it-*` workspace remains.
- **Rollback or recovery:** Revert; `workspace delete it-*`.
- **Builder stop conditions:** The deployed v0.1.0 server disagrees with the client on any rewrite step (the harness tests pass against the same commit, so this would mean the deployment differs): report with the server log from `docker compose logs`.

### PLAN-00011-STEP-07 — Documentation and gate

- **Objective:** Docs, backlog, every gate, evidence that `home` is untouched.
- **Requirements:** `PLAN-00011-REQ-12`, `PLAN-00011-REQ-14`, `PLAN-00011-REQ-10`
- **Depends on:** STEP-06
- **Affected components:** `docs/developer-guide.md`, `README.md`, `CHANGELOG.md`, `docs/backlog.md`
- **Preconditions:** STEP-06 committed.
- **Test or evidence first:** `just links` red if a new link is wrong; `just check`.
- **Implementation tasks:**
  1. `docs/developer-guide.md` § Deployed server tests: purpose, the variables (table from § 8), the server-side commands to create the workspace and keys (from the note's § Admin), `just test-deployed`, what it creates and removes, why it is not in CI, and that key expiry is `never` by D-02.
  2. `README.md`: one line under testing pointing at the section.
  3. `docs/backlog.md`: remove nothing (no item covers this); add follow-ups: a CI job on a self-hosted tailnet runner; an SSH-free variant of the destructive tests once the server has a workspace reset API; running the suite against the SSH deployment's successor.
  4. Record in the work log: `home` item count and key ids over SSH before STEP-01 and after this step, equal.
- **Documentation/configuration/operations:** As above.
- **Verification:** `just check`; `just test-deployed`; `just test-https`; `just test-integration`; `just coverage` (report the percentage); `cargo test --workspace --all-features 2>&1 | grep -c "deployed_.* ignored"` equals the number of tests.
- **Completion criteria:** All pass; evidence in the work log; branch pushed and CI green (Linux, macOS, Xvfb, Windows, Android, https).
- **Rollback or recovery:** Revert the commit.
- **Builder stop conditions:** Coverage under 80 %; CI red.

## 12. Cross-cutting concerns

| Area | Applicability | Planned action or reason not applicable | Step or requirement |
|---|---|---|---|
| Compatibility and APIs | Not applicable | No crate source changes; tests use public APIs of `passalong-core` and `passalong-https` already used by `cli_https_backend.rs` | — |
| Data and migration | Applicable | Only `integration-test` and `it-*` workspaces are written; `home` untouched and checked | REQ-10, STEP-07 |
| Security and privacy | Applicable | Secrets in `.env` (0600, git-ignored) only; redaction in panics; no secret asserted; keys made at runtime live in temp dirs; `BatchMode=yes` SSH so nothing prompts | REQ-02, REQ-13, STEP-05 |
| Performance and scale | Applicable | One thread; the quota test uses a sparse file and records bytes sent; a 1 MiB file is the largest ordinary transfer | STEP-04 |
| Reliability and failure handling | Applicable | Clean-up in `Drop`; counts relative to the start; prune before the run; skip without SSH | REQ-10, STEP-01, STEP-05 |
| Observability and operations | Applicable | Tests capture the client's logs; failures print the server's `docker compose logs --tail=100` over SSH when available | STEP-05 |
| Dependencies and supply chain | Not applicable | No new crates; `ssh` and `docker compose` are host tools the existing recipes already assume | — |
| Accessibility and UX | Not applicable | Tests only | — |
| Documentation and release | Applicable | Developer guide, README, CHANGELOG, backlog; no version bump (a release is not part of this plan) | REQ-12, STEP-07 |
| Deployment and rollback | Applicable | Nothing on phoenix changes except workspaces and keys the suite makes and removes; leftovers are listed and removable with one command | STEP-05 |

## 13. Verification strategy

| Level | Evidence or command | When | Required result |
|---|---|---|---|
| Suite | `just test-deployed` | Every step | Pass on the tailnet |
| Isolation | `workspace show home --json`, `key list --json` over SSH before and after | STEP-01, STEP-07 | Unchanged |
| Existing suites | `just test-https`, `just test-integration` | STEP-01, STEP-04, STEP-07 | Pass, no `deployed_` test run |
| Skip path | `INTEGRATION_TEST_SSH_HOST= just test-deployed` | STEP-05 | SSH tests skipped, suite passes |
| Lint and format | `just check` | Every step | Pass |
| Coverage | `just coverage` | STEP-07 | ≥ 80 % lines, reported |
| CI | Push after every step | Every step | Green |

## 14. Acceptance criteria

- [ ] `PLAN-00011-AC-01` `just test-deployed` passes on a tailnet machine with the `.env` of 2026-10-11; every test in `tests/deployed.rs` and `tests/cli_deployed_server.rs` is `#[ignore]` and named `deployed_*`.
- [ ] `PLAN-00011-AC-02` `cargo test --workspace --all-features`, `just test-integration`, `just test-https`, and `just coverage-full` run no `deployed_*` test (their output lists them as ignored or filtered), and `.github/workflows` is unchanged.
- [ ] `PLAN-00011-AC-03` With `INTEGRATION_TEST_SERVER_URL` unset, the smoke test fails with a message naming that variable and `just test-deployed`; with `INTEGRATION_TEST_SSH_HOST` unset, the SSH-dependent tests print a skip line and pass.
- [ ] `PLAN-00011-AC-04` `.env.sample` lists all eight `INTEGRATION_TEST_*` variables with safe examples; `git grep -E 'pal_[0-9a-f]{12}_[A-Za-z0-9_-]{20,}'` finds nothing; `.env` is ignored.
- [ ] `PLAN-00011-AC-05` The pin in `sha256/` and `sha256//` forms succeeds; a wrong pin fails before any request, within one second, naming the server's pin; each URL in `INTEGRATION_TEST_SERVER_ALT_URLS` answers the viewer under the pin.
- [ ] `PLAN-00011-AC-06` An unknown key fails with `UNAUTHENTICATED` after one request; the read-only key's viewer role is read-only, `check` shows `storage write  n/a`, and a send fails with `FORBIDDEN_ROLE` while the item count is unchanged.
- [ ] `PLAN-00011-AC-07` `init --url … --tls-pin … --api-key-file … --device-name … --yes` writes a config and a 0600 key file, and `check` prints `server … passalong-server 0.1.0, API v1, TLS pinned`, `api key ok integration-test (read-write), never expires`, `workspace ok integration-test: … of 256.0 MiB used, N items`, `encryption off`, and `storage write ok`.
- [ ] `PLAN-00011-AC-08` Text and a 1 MiB random file round-trip: `list --json` names the test device, `get` shows the local SHA-256, `cat` and `load` return equal bytes, `delete` removes both, and the item count returns to its starting value; identical content sent twice yields one id.
- [ ] `PLAN-00011-AC-09` A sparse file larger than the remaining quota is refused within 30 s with a message naming `QUOTA_EXCEEDED` and the quota of 268435456 bytes; the item count is unchanged.
- [ ] `PLAN-00011-AC-10` On Unix, `serve` sends a dropped file (listed, moved to `sent/`) and in pull mode downloads an item sent under a second device name; both runs end cleanly.
- [ ] `PLAN-00011-AC-11` A key created over SSH works, is refused from the first request after `key revoke`, and `serve` under it exits non-zero within one retry naming the key; after the tests `workspace list` over SSH shows no `it-*` workspace and `key list` no `it` key.
- [ ] `PLAN-00011-AC-12` On a throwaway workspace: set-up, send, `workspace show` sealed, join from a second device and read, rotate, the stale device told to join, re-join and read, an interrupted rotation recovered by its device, every item's `load` SHA-256 equal to the original, a fresh start's plain items listed until `prune --plain`; the workspace is deleted afterwards.
- [ ] `PLAN-00011-AC-13` The captured output of a full session at `debug` level, and every panic message of the suite's helpers, contain no `pal_<id>_<secret>`.
- [ ] `PLAN-00011-AC-14` `docs/developer-guide.md` § Deployed server tests, a README pointer, a CHANGELOG Unreleased entry, and backlog follow-ups exist; `just check` and `just links` pass; coverage is at or above 80 % and the percentage is in the hand-off.
- [ ] `PLAN-00011-AC-15` `home`'s item count and key ids over SSH are the same before STEP-01 and after STEP-07.

## 15. Risks and mitigations

| Risk | Likelihood | Impact | Mitigation or test | Owner/step |
|---|---|---|---|---|
| phoenix or the tailnet is unreachable when the suite runs | Medium | Low | The recipe probes `/readyz` first and says which URL failed; nothing else is attempted | STEP-01 |
| phoenix reboots: Docker is socket-activated and no container restarts (note § Persistence) | Low | Medium | The recipe's probe names the note's fix; the user's sudo, not the suite | User |
| A LAN URL fails from outside the LAN | Medium | Low | The SAN test names the failing URL; the user may drop it from `.env` (stop condition in STEP-02) | STEP-02 |
| Two machines run the suite on `integration-test` at once | Low | Low | Counts are relative; items are named by content so they do not collide; documented as unsupported | STEP-03 |
| An aborted run leaves `it-*` workspaces or plain items | Medium | Low | Prune before each run; `Drop` clean-up; one documented command removes leftovers | STEP-01, STEP-05 |
| The quota test uploads 256 MiB before refusal | Low | Low | Verified 2026-10-11 that the server refuses at `beginUpload`; the stop condition catches a change | STEP-04 |
| Secrets leak through a panic message or a captured log | Low | High | `redact` on every helper panic; the no-secret test; assertions on ids only | STEP-01, STEP-03 |
| The deployed server is upgraded past `server_commit` and the contract moves | Low | Medium | `check` records the server version; a mismatch with the pinned commit is a work-log note, not a failure | STEP-03 |
| `workspace delete` refuses a sealed workspace | Low | Medium | Stop condition in STEP-05; recovered by hand over SSH | STEP-05 |

## 16. Builder hand-off

- **Start condition:** User approval, a clean repository, `.env` present with the eight variables, and `curl -sk https://phoenix.kiko-walleye.ts.net:8445/readyz` returning 204.
- **First step:** PLAN-00011-STEP-01, after recording `home`'s item count and key ids over SSH in the work log.
- **Required sequence:** STEP-01 → 02 → 03 → 04 → 05 → 06 → 07; STEP-02 may run alongside STEP-03; commit and push after each step so CI runs.
- **Parallel-safe work:** The developer-guide section may be drafted from STEP-01 and committed in STEP-07.
- **Do not change:** approved scope, requirements, steps, acceptance criteria, or content outside Builder's permitted work-log area; any client source under `crates/*/src`; anything on phoenix other than `integration-test` items and `it-*` workspaces and their keys; `.env`.
- **Escalate when:**
  - any Builder stop condition triggers;
  - a test exposes a client bug (record, do not fix here);
  - the server refuses an admin command the plan relies on;
  - the suite would need a secret anywhere but `.env` and temporary directories.
- **Completion hand-off:** The work log is complete with the `home` before/after evidence, the coverage percentage, the quota test's bytes sent, and the `just test-deployed` output summary; the user reviews and approves; no release is part of this plan.

<!-- BUILDER_WORK_LOG_START -->
## 17. Builder Work Log

> [!warning] Builder-maintained section
> Delivery Planner creates this section. After approval, Builder may update only
> this delimited section and the Builder-maintained front-matter fields. Builder
> must preserve prior entries and use UTC timestamps.

### Step status

| Step | Status | Started (UTC) | Completed (UTC) | Evidence | Builder notes |
|---|---|---|---|---|---|
| PLAN-00011-STEP-01 | completed | 2026-10-11T13:23:08Z | 2026-10-11T13:26:08Z | Commit of this step; `just test-deployed` 2 passed | home before: 0 items, keys b2c38ff363e8 6437a8b21c81 25dddb1677f7 (over SSH, 13:23Z) |
| PLAN-00011-STEP-02 | completed | 2026-10-11T13:26:08Z | 2026-10-11T13:27:56Z | Commit of this step; `just test-deployed` 6 + 1 passed | The deployment's pin is `sha256//MOMQ…`: canonical `sha256/` and a base64 that itself begins with `/`, not curl's form; the helper derives both forms from the parsed pin |
| PLAN-00011-STEP-03 | not-started | — | — | — | — |
| PLAN-00011-STEP-04 | not-started | — | — | — | — |
| PLAN-00011-STEP-05 | not-started | — | — | — | — |
| PLAN-00011-STEP-06 | not-started | — | — | — | — |
| PLAN-00011-STEP-07 | not-started | — | — | — | — |

Allowed status values: `not-started`, `in-progress`, `blocked`, `completed`,
`skipped`. A skipped step requires explicit user approval recorded in Evidence.

### Execution log

| Timestamp (UTC) | Step | Event | Evidence or reference | Next action |
|---|---|---|---|---|
| 2026-10-11T13:26:08Z | PLAN-00011-STEP-01 | Red: the smoke test without the variables panicked with `INTEGRATION_TEST_SERVER_URL` is not set: run these tests with `just test-deployed` | cargo test -p passalong-https --test deployed -- --ignored | Write the recipe |
| 2026-10-11T13:26:08Z | PLAN-00011-STEP-01 | Green: recipe, support module, CLI smoke test | just test-deployed: 2 passed | STEP-02 |
| 2026-10-11T13:27:56Z | PLAN-00011-STEP-02 | Red: pin-form and SAN tests failed on a helper that stripped a slash from the base64 | the pin's base64 starts with `/` | Derive forms from `TlsPin::parse` |
| 2026-10-11T13:27:56Z | PLAN-00011-STEP-02 | Green: five TLS and identity tests | just test-deployed: 6 + 1 passed | STEP-03 |

### Deviations and blockers

| Timestamp (UTC) | Step | Deviation or blocker | Impact | Decision required from |
|---|---|---|---|---|

None.

### Verification results

| Timestamp (UTC) | Step | Command or check | Result | Evidence |
|---|---|---|---|---|
| 2026-10-11T13:26:08Z | PLAN-00011-STEP-01 | just test-deployed | pass (deployed_the_server_answers_the_viewer_and_workspace, deployed_check_passes_with_the_read_write_key) | recipe probes /readyz, prunes, runs both test targets with --test-threads=1 |
| 2026-10-11T13:26:08Z | PLAN-00011-STEP-01 | cargo test -p passalong-https --test deployed; -p passalong --test cli_deployed_server (no --ignored) | 1 ignored each, redact unit test passes | default runs leave the suite out |
| 2026-10-11T13:26:08Z | PLAN-00011-STEP-01 | just test-https | pass: 27 + 12 tests, 0 deployed_ tests run | --skip deployed_ in effect |
| 2026-10-11T13:26:08Z | PLAN-00011-STEP-01 | cargo clippy -p passalong-https -p passalong --all-targets --all-features -D warnings; cargo fmt | pass | — |
| 2026-10-11T13:27:56Z | PLAN-00011-STEP-02 | just test-deployed | pass: pin forms, wrong pin (not retryable, under 1 s, names the pin), 3 URLs under the pin and by handshake, unknown key UNAUTHENTICATED once, read-only key FORBIDDEN_ROLE on the probe with counts unchanged | — |
| 2026-10-11T13:27:56Z | PLAN-00011-STEP-02 | cargo clippy -p passalong-https --all-targets --all-features -D warnings; cargo fmt | pass | — |

### Completion summary

- **Implementation status:** `in-progress`
- **Completed requirements:** None
- **Incomplete requirements:** All
- **Outstanding blockers:** None
- **Review request:** Not ready
<!-- BUILDER_WORK_LOG_END -->

## 18. Planning change log

| Timestamp (UTC) | Plan status | Change | Reason | Requested/approved by |
|---|---|---|---|---|
| 2026-10-11T12:24:44Z | draft | Created | User request for integration tests against the deployed server; D-01..D-04 answered by the user the same day; the workspace and keys were created and `.env` written during planning | User |
| 2026-10-11T13:23:08Z | approved | Approved as drafted | "I've reviewed, approved and pushed the Plan. Please build." | User |

## 19. External references

1. **Passalong HTTPS Server (Docker)**, Joe Lee's Obsidian vault,
   `30_areas/Passalong HTTPS Server.md`, written 2026-09-25, read
   2026-10-11. The deployment this plan tests: ports, pin, SANs, admin
   commands, hardening, and the note's own verification.

## 20. Confidence

**High.** The test harness, the CLI test patterns, and the recipes this plan
extends were read at the baseline, and the deployment was exercised from
this machine with the exact `.env` the suite will use: `init --yes`,
`check`, text and file round trips, both alternative URLs, a wrong pin,
and an over-quota upload all behaved as the plan asserts. The read-only and unknown-key
refusals were seen too. The residual uncertainty is whether `workspace
delete` removes a sealed throwaway workspace (STEP-05 stop condition) and
whether the rewrite recovery path behaves on the deployment as it does on
the harness (STEP-06 stop condition).
