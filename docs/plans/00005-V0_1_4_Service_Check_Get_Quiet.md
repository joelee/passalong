---
title: "Delivery Plan 00005: V0 1 4 Service Check Get Quiet"
aliases:
  - "Plan 00005"
tags:
  - delivery-plan
  - implementation
  - opencode
type: delivery-plan
plan_id: "PLAN-00005"
plan_status: approved              # draft | approved | cancelled
plan_kind: initial                 # initial | superseding
created_at: "2026-09-13T20:40:20Z"
approved_at: "2026-09-13T20:50:03Z"
planner_agent: "Claude Code"
planner_model: "anthropic/claude-opus-5"
triggered_by: user                 # user | agent:<agent-name>
request_kind: direct               # idea | review | idea-and-review | direct | unplanned-query
repository: "joelee/passalong"
baseline_branch: "feature/00005-v0.1.4"
baseline_commit: "5e3fc2fbf0dea99f7abfbc2460f76f3c34a603e7"
source_ideas: []
source_reviews: []
previous_plan: null
requirements_count: 12
steps_count: 8
acceptance_criteria_count: 17
blocking_decisions: 0
build_ready: true
web_research_used: true
confidence: medium                # high | medium | low

# Builder-maintained front matter. Builder may update only these keys after
# explicit user approval; Delivery Planner initializes them.
implementation_status: not-started # not-started | in-progress | blocked | completed | abandoned
builder_agent: null
builder_model: null
execution_branch: null
execution_started_at: null
execution_updated_at: null
execution_completed_at: null
current_step: null
---

# Delivery Plan 00005: V0 1 4 Service Check Get Quiet

> [!abstract] Plan status: `approved`
> Deliver `passalong` v0.1.4: the eight items of the maintainer's v0.1.4
> road map in `docs/backlog.md`, namely a quiet Release workflow, working
> links on crates.io, a Mermaid "How it works" diagram, `cat` without a
> trailing log line, a global `--quiet`, and the new `get`, `check`, and
> `install-service` commands. The user settled D-01 to D-04 on 2026-09-13
> (all as recommended); approved by the user at 2026-09-13T20:50:03Z; Builder-ready.

## 1. Objective and outcome

v0.1.3 is released. v0.1.4 finishes the maintainer's v0.1.4 road map:

1. The Release workflow passes without error annotations: no rust-cache
   `ENOENT` errors from `target/package`, and no Node.js 20 deprecation
   warnings.
2. Every link in the project's Markdown works on GitHub and on
   <https://crates.io/crates/passalong>. Today all 12 relative links in the
   README are broken on crates.io.
3. The README's "How it works" diagram is a Mermaid diagram.
4. `passalong cat` prints only the item's content at the default log level.
5. A global `--quiet` hides results and logs, so scripts rely on the exit
   code.
6. `passalong get <ID>` prints one item's metadata.
7. `passalong check` tests the configuration and the server, including a
   write test that other devices never see.
8. `passalong install-service` installs `serve` as a systemd user unit on
   Linux or a launchd agent on macOS, enables it, and starts it.

## 2. Source traceability

| Requirement | Source | Source location | Interpretation |
|---|---|---|---|
| PLAN-00005-REQ-01 | User; Repository | `docs/backlog.md` "@joelee road map" v0.1.4 item 1 and Engineering item "Clean up the Release workflow's annotations"; `.github/workflows/release.yml`; Release run 34772392506 annotations | No error or deprecation annotations in the Release workflow |
| PLAN-00005-REQ-02 | User; Repository | Backlog v0.1.4 item 2; `crates/passalong-cli/Cargo.toml` `readme = "../../README.md"`; crates.io API `GET /api/v1/crates/passalong/0.1.3/readme` | All Markdown links work on GitHub and crates.io, and a check keeps it so |
| PLAN-00005-REQ-03 | User | Backlog v0.1.4 item 8; `README.md` "How it works" | Mermaid diagram replaces the ASCII one |
| PLAN-00005-REQ-04 | User; Repository | Backlog v0.1.4 item 5; `crates/passalong-cli/src/commands/cat.rs` `tracing::info!(… "item printed")`; D-02 | `cat` adds nothing after the content at the default level |
| PLAN-00005-REQ-05 | User | Backlog v0.1.4 item 6; D-01 | Global `--quiet` |
| PLAN-00005-REQ-06 | User | Backlog v0.1.4 item 3; D-08 | `passalong get <ID>` |
| PLAN-00005-REQ-07 | User | Backlog v0.1.4 item 7; D-04 | `passalong check` with a hidden write probe |
| PLAN-00005-REQ-08 | User; Repository | Backlog v0.1.4 item 4; `docs/service/passalong-serve.service`, `docs/service/com.passalong.serve.plist`; D-03, D-09 | `passalong install-service` for systemd and launchd |
| PLAN-00005-REQ-09 | Repository | Root `AGENTS.md` "Docs to maintain", "Backlog rules" | Documentation and backlog |
| PLAN-00005-REQ-10 | Repository | Root `AGENTS.md` "Release workflow" | v0.1.4 release preparation |
| PLAN-00005-REQ-11 | Repository | Root `AGENTS.md` "Non-negotiables", "Commands" | Quality gates |
| PLAN-00005-REQ-12 | User | Road map (2026-09-12): v0.1.x is the SSH-only CLI; backlog v0.1.5 and v0.2.0 sections | Road-map guard |

## 3. Repository baseline

| Field | Value |
|---|---|
| Repository | `joelee/passalong` (`git@github.com:joelee/passalong.git`) |
| Branch | `feature/00005-v0.1.4`, created from `main` at the user's request; not pushed |
| HEAD | `5e3fc2fbf0dea99f7abfbc2460f76f3c34a603e7`: `main` at `737f425` (merge of PR #5, tag `v0.1.3`) plus `docs(backlog): v0.1.4 road map` |
| Working tree at publication | Clean before allocation; the only change is this plan file |
| Release state | v0.1.3 released 2026-09-13: GitHub release with Linux and macOS binaries; the three crates at 0.1.3 on crates.io |
| Applicable instructions | Root `AGENTS.md`; `docs/plans/AGENTS.md`; `docs/backlog.md` road map |

Findings from the planning research, 2026-09-13:

- **Release run 34772392506 (v0.1.3):**
  - "Check the tag and the packages" and "Publish to crates.io" each carry 10
    `ENOENT … opendir '…/target/package/<crate>-0.1.3/tests/{target,trybuild}'`
    error annotations.
  - The build and release jobs carry Node.js 20 warnings for
    `actions/upload-artifact@v4` and `actions/download-artifact@v4`.
  - The latest CI run on `main` has no annotations; its cache is usually a
    hit, so rust-cache's cleanup does not run there.
  - `Swatinem/rust-cache@v2` (v2.9.2) `cleanProfileTarget` calls
    `cleanTargetDir` for `tests/target` and `tests/trybuild` without
    `await`, so its `try/catch` never catches the rejection.
- **Link scan:**
  - All 26 tracked Markdown files have no broken relative link or heading
    anchor on GitHub.
  - All 23 external URLs answer HTTP 200. crates.io pages need an
    `Accept: text/html` header, or they answer 404 to scripts.
  - On crates.io, the `passalong` README's 12 relative links are rewritten
    to `https://github.com/joelee/passalong/blob/HEAD/crates/passalong-cli/<path>`,
    which does not exist. These are CHANGELOG.md, CONTRIBUTING.md,
    SECURITY.md, LICENSE, six `docs/*.md`, and `docs/service/`.
  - The `passalong-core` and `passalong-ssh` READMEs use only absolute
    links.
- **Mermaid:** crates.io renders Mermaid in READMEs since July 2023
  (discussion #5724; issue #6786 closed 2023-07-14).
- **Logging observed with a local store:**
  - `passalong cat` printed `hello there2026-09-13T20:29:06Z info passalong::commands::cat op=… item printed id=… size=11`.
  - `file` and `delete` print one `info` line on stderr beside their result.
- **Newer action majors:**
  - `actions/upload-artifact` v7.0.1 and `actions/download-artifact` v8.0.1
    run on Node.js 24.
  - `download-artifact` v8 fails on a digest mismatch by default.
  - `download-artifact` v5's path change applies only to downloads by
    artifact ID; the release job downloads by pattern with
    `merge-multiple`.

## 4. Scope

### In scope

- **Release workflow:** remove `target/package` before rust-cache's post
  step, move the artifact actions to their Node.js 24 majors, and add
  Dependabot version updates for GitHub Actions.
- **Links:** make the README's links absolute so they work on crates.io,
  add a link check to `just check`, and record a one-off scan of every
  external link.
- **README:** the "How it works" diagram in Mermaid.
- **`cat`:** its "item printed" record at `verbose` level.
- **Global `-q`/`--quiet`** (D-01).
- **`passalong get <ID> [--json]`** (D-08).
- **`passalong check`:** includes `Store::probe_write` with a default
  implementation and an `FsStore` override (D-04).
- **`passalong install-service [--no-start] [--force] [--uninstall]`:**
  systemd on Linux, launchd on macOS (D-03, D-09).
- **Release records:** documentation, backlog, version 0.1.4, and the draft
  release notes.

### Out of scope

- The maintainer's v0.1.5 and v0.2.0 items: the Android cross-compile check,
  `passalong choose`, Windows support, and Homebrew.
- Services on other platforms: Windows is v0.2; other init systems are not
  planned.
- Changing the default log level of other commands; per D-02, only `cat`
  changes.
- Editing published v0.1.1 to v0.1.3 GitHub release text or crates.io
  READMEs of published versions, which are immutable.
- Tagging, publishing, and the release finalisation commit, which follow
  the root `AGENTS.md` "Release workflow".

## 5. Constraints and preserved decisions

- Every decision of PLAN-00001 to PLAN-00004 stays in force, including the
  item schema (version 1), the storage layout, and the exit codes 0, 1, 2,
  and 3.
- `passalong-core` and `passalong-ssh` are published. The new `Store`
  method has a default implementation, and new public enums are
  `#[non_exhaustive]`, so implementations outside this repository keep
  compiling within 0.1.x.
- **Library crates must not spawn processes.** `install-service` runs
  `systemctl` and `launchctl` from the CLI crate only, behind a trait that
  tests replace.
- **Tests must never touch the real service manager, the real user
  services, or the real clipboard.** Service tests use a recording fake
  and, in binary tests, fake `systemctl` and `launchctl` scripts on `PATH`
  with `HOME` and `XDG_CONFIG_HOME` in a temporary directory.
- **Quality rules:**
  - TDD, with a failing test first and mocks for external interfaces.
  - `just check` green at every step, and line coverage at least 80 %.
  - Rustdoc for public items, no `unsafe`, and no `println!` for logs.
- **Release steps** follow the root `AGENTS.md` "Release workflow".
  Release notes use absolute, tag-pinned links. The Builder never tags or
  publishes.
- **Builder** works on `feature/00005-v0.1.4`, one commit per step, without
  review pauses unless blocked.

## 6. Assumptions

None. Unresolved matters are recorded as decisions.

## 7. Decisions and blockers

| ID | Decision or blocker | Resolution | Owner | Status |
|---|---|---|---|---|
| D-01 | What `--quiet` hides. | **Confirmed by user (2026-09-13):**<br>• A global `-q`/`--quiet` hides every command's standard output except `cat`'s content.<br>• It lowers logging to `error`, unless `--log-level` or `PASSALONG_LOG_LEVEL` is set; an explicit level wins over `--quiet`, and `--quiet` wins over `client.log_level`.<br>• It never hides errors or prompts. When a prompt follows, what it asks about is still shown on standard error: `init`'s host-key fingerprint, and `prune`'s list before its confirmation.<br>• Scripts use the exit code. | User | Resolved |
| D-02 | How `cat` stops printing a log line after the content. | **Confirmed by user (2026-09-13):** `cat` records "item printed" at `verbose` instead of `info`, so a default run prints only the content, and `--log-level verbose` still shows it. Other commands keep their `info` records. | User | Resolved |
| D-03 | What `install-service` does. | **Confirmed by user (2026-09-13):**<br>• It writes the unit with this binary's absolute path, plus `--config <absolute path>` when `--config` was given, then enables and starts it.<br>• `--no-start` only writes the unit. `--force` replaces an existing unit that differs; an identical unit is left alone and reported. `--uninstall` stops, disables, and removes it.<br>• It refuses while `serve` is already running.<br>• Other platforms get "install-service supports Linux (systemd) and macOS (launchd) only". | User | Resolved |
| D-04 | How far `check` tests the server. | **Confirmed by user (2026-09-13):**<br>• It checks the config, connects (verifying the pinned host key for `ssh`), and lists the store.<br>• It then creates and removes a probe directory under the store's `tmp/`, which listings and other devices never see, through a new `Store::probe_write` with a default implementation.<br>• It prints one line per check and exits 1 at the first failure, marking later checks as skipped. | User | Resolved |
| D-05 | How the README's links work on crates.io. | **Resolved by planner:** relative links in `README.md` become absolute GitHub URLs on `main` (`https://github.com/joelee/passalong/blob/main/<path>`, and `tree/main/docs/service` for the folder). Same-page `#anchor` links stay. A new `scripts/check-links.sh`, run by `just check`, fails when a crate's `readme` file (found through the `readme` keys of the crates' `Cargo.toml`) has a relative link. It also fails when any tracked Markdown file has a relative link to a missing file or to a missing heading. **Alternative rejected:** a separate crate README for the CLI would duplicate the root README and drift from it. | Planner | Resolved |
| D-06 | Mermaid and crates.io. | **Resolved by planner:** crates.io renders Mermaid since July 2023, so the README uses one Mermaid `flowchart` for both sites. It shows the laptop, desktop, and phone (Android, planned) connecting over SSH/SFTP with a pinned host key to `server:/srv/passalong`, holding `items/<time>-<hash>/{content,meta.json}`. The GitHub rendering is checked on the pushed branch. The crates.io rendering can only be checked after the v0.1.4 publish. | Planner | Resolved |
| D-07 | Release workflow fix details. | **Resolved by planner:**<br>• In `release.yml`, the `verify` and `crates` jobs end with a step `rm -rf target/package` that runs `if: always()`, so it runs before rust-cache's post step even after a failure.<br>• `just publish-dry-run` removes `target/package` after its dry run, which covers CI's Linux job when its cache misses.<br>• `actions/upload-artifact@v4` becomes `@v7`, and `actions/download-artifact@v4` becomes `@v8`.<br>• A new `.github/dependabot.yml` asks for monthly `github-actions` version updates; Cargo updates stay manual. | Planner | Resolved |
| D-08 | What `get` prints. | **Resolved by planner:**<br>• One `field: value` line each for `id`, `kind` (`image` for clipboard images), `name` or `preview`, `mime`, `size` (human and bytes), `sha256`, `device`, `origin` when set, and `created` (local time and UTC).<br>• `--json` prints the item's metadata object, the same schema as one element of `list --json`.<br>• The id works like `load` and `cat`: 4 or more characters, with the choice prompt when ambiguous. `get` reads only `meta.json`, through `Store::get_meta`. | Planner | Resolved |
| D-09 | Service details. | **Resolved by planner:**<br>• **Templates** live in the CLI crate, so they are part of the published package; `docs/service/*` stay as the manual route and a test keeps them identical to the templates rendered with their documented placeholders.<br>• **Linux:** `$XDG_CONFIG_HOME/systemd/user/passalong-serve.service` (default `~/.config/systemd/user/`); then `systemctl --user daemon-reload` and `systemctl --user enable --now passalong-serve.service`.<br>• **macOS:** `~/Library/LaunchAgents/com.passalong.serve.plist`; then `launchctl bootstrap gui/<uid> <plist>`, with the uid read from the home directory's owner, not from `unsafe` or a new dependency. It logs to `~/Library/Logs/passalong/serve.log`, the same file as `serve --daemon`.<br>• **Working directory:** both units set it to the home directory, so a `~/.env` holding the key passphrase is found; this is documented.<br>• **Uninstall:** `systemctl --user disable --now` or `launchctl bootout`, then the file is removed. | Planner | Resolved |
| D-10 | Branch and version. | **Resolved by planner:** v0.1.4 on `feature/00005-v0.1.4`, which carries the road map commit. | Planner | Resolved |

Blocking decisions: 0.

## 8. Affected architecture and components

| Area | Paths | Change |
|---|---|---|
| Workflows | `.github/workflows/release.yml`, `.github/dependabot.yml` (new), `justfile` `publish-dry-run` | Clean `target/package`; Node.js 24 artifact actions; Dependabot |
| Links | `README.md`, `scripts/check-links.sh` (new), `crates/passalong-cli/tests/link_check_script.rs` (new), `justfile` (`links` recipe, part of `check`) | Absolute README links; link check |
| CLI surface | `crates/passalong-cli/src/cli.rs`, `app.rs` | `--quiet`; `get`, `check`, `install-service` subcommands |
| Commands | `crates/passalong-cli/src/commands/{cat,get,check,install_service,init,prune}.rs` | New commands; `cat` record level; prompt context under `--quiet` |
| Services | `crates/passalong-cli/src/service.rs` (new; templates and the service-manager trait), `docs/service/*` | Unit rendering, install, uninstall |
| Store | `crates/passalong-core/src/store/{mod,fs_store}.rs`, `crates/passalong-ssh/tests/sftp_docker.rs` | `Store::probe_write`, `WriteProbe` |
| Docs | `README.md`, `docs/{usage,configuration,architecture,developer-guide,backlog}.md`, `CHANGELOG.md`, `docs/release/v0.1.4.md` (new), `Cargo.toml` versions | Updated |

```rust
// passalong-core::store
#[non_exhaustive]
pub enum WriteProbe { Verified, NotSupported }
// Store trait, with a default returning Ok(WriteProbe::NotSupported)
async fn probe_write(&self) -> Result<WriteProbe, StoreError>;
```

## 9. Requirement catalogue

### PLAN-00005-REQ-01 — Quiet Release workflow

- **Requirement:** Per D-07.
- **Rationale:** The v0.1.3 run passed with 20 error annotations and 3
  deprecation warnings, which hide real problems.
- **Source:** Backlog v0.1.4 item 1 and its Engineering entry.
- **Acceptance evidence:** AC-01, AC-02.

### PLAN-00005-REQ-02 — Working links everywhere

- **Requirement:** Per D-05. Scan every link in the tracked Markdown, fix
  the broken ones, and add the link check to `just check`.
- **Rationale:** 12 links on the crates.io page are broken.
- **Source:** Backlog v0.1.4 item 2.
- **Acceptance evidence:** AC-03, AC-04.

### PLAN-00005-REQ-03 — Mermaid "How it works"

- **Requirement:** Per D-06.
- **Rationale:** User request.
- **Source:** Backlog v0.1.4 item 8.
- **Acceptance evidence:** AC-05.

### PLAN-00005-REQ-04 — `cat` prints only the content

- **Requirement:** Per D-02.
- **Rationale:** The trailing log line garbles the terminal output.
- **Source:** Backlog v0.1.4 item 5.
- **Acceptance evidence:** AC-06.

### PLAN-00005-REQ-05 — `--quiet`

- **Requirement:** Per D-01, for every command, including those this plan
  adds.
- **Rationale:** User request.
- **Source:** Backlog v0.1.4 item 6.
- **Acceptance evidence:** AC-07.

### PLAN-00005-REQ-06 — `passalong get <ID>`

- **Requirement:** Per D-08.
- **Rationale:** Inspect one item without listing the store.
- **Source:** Backlog v0.1.4 item 3.
- **Acceptance evidence:** AC-08.

### PLAN-00005-REQ-07 — `passalong check`

- **Requirement:** Per D-04. The lines are `config`, `server`,
  `storage read`, and `storage write`. A backend without a write probe
  reports `storage write: not supported by this backend` without failing.
  The probe never leaves anything under `tmp/` or `items/`.
- **Rationale:** User request.
- **Source:** Backlog v0.1.4 item 7.
- **Acceptance evidence:** AC-09, AC-10.

### PLAN-00005-REQ-08 — `passalong install-service`

- **Requirement:** Per D-03 and D-09.
- **Rationale:** Replaces the manual copy-and-edit steps in `docs/service/`.
- **Source:** Backlog v0.1.4 item 4.
- **Acceptance evidence:** AC-11, AC-12, AC-13.

### PLAN-00005-REQ-09 — Documentation

- **Requirement:**
  - **README:** the command table gains `get`, `check`, and
    `install-service`, and step 5 of the quick start uses
    `install-service`.
  - **`docs/usage.md`:** `-q`/`--quiet` under global options; new sections
    for `get`, `check`, and `install-service`; the `cat` logging note.
  - **`docs/configuration.md`:** `--quiet` in the logging order.
  - **`docs/architecture.md`:** the command flow, `probe_write`, and
    services.
  - **`docs/developer-guide.md`:** the link check and Dependabot.
  - **`docs/backlog.md`:** the v0.1.4 road map items and the Engineering
    entry removed.
- **Rationale:** Documentation rules.
- **Source:** Root `AGENTS.md` "Docs to maintain", "Backlog rules".
- **Acceptance evidence:** AC-14.

### PLAN-00005-REQ-10 — v0.1.4 release preparation

- **Requirement:** All crates and internal `=` requirements at `0.1.4`.
  `CHANGELOG.md` `Unreleased` lists the changes, and
  `docs/release/v0.1.4.md` is drafted with absolute, tag-pinned links.
- **Rationale:** The user tags v0.1.4 after this plan.
- **Source:** Root `AGENTS.md` "Release workflow".
- **Acceptance evidence:** AC-15.

### PLAN-00005-REQ-11 — Quality gates

- **Requirement:**
  - TDD evidence per step.
  - `just check` green at every step and `just ci` green locally.
  - Branch CI green on Linux, macOS, and Xvfb.
  - Line coverage at least 80 %.
- **Rationale:** Repository rules.
- **Source:** Root `AGENTS.md`.
- **Acceptance evidence:** AC-16.

### PLAN-00005-REQ-12 — Road map guard

- **Requirement:** No GUI, Windows, Android, or new backend code;
  `BackendRegistry` kinds remain `local` and `ssh`.
- **Rationale:** v0.1.x is the SSH-only CLI.
- **Source:** User road map.
- **Acceptance evidence:** AC-17.

## 10. Delivery strategy

1. **Release workflow (STEP-01).** Independent and small; it lands first so
   every later CI run exercises it.
2. **Links and diagram (STEP-02).** Documentation plus a check script, with
   no code dependency.
3. **Output control (STEP-03).** `cat` and `--quiet` come before the new
   commands, so those are built and tested with `--quiet` from the start.
4. **New commands (STEP-04 to STEP-06).** `get`, the smallest and read-only,
   comes first; then `check` with `probe_write`; then `install-service`,
   which is the largest and platform-specific.
5. **Docs, release preparation, final gate (STEP-07, STEP-08).**

Each step ends with `just check` green and one commit. Docker-backed tests
run at STEP-05 and STEP-08. Platform code is tested on Linux locally and on
macOS through CI's `just check`.

## 11. Detailed implementation steps

### PLAN-00005-STEP-01 — Quiet Release workflow

- **Objective:** Implement REQ-01.
- **Requirements:** `PLAN-00005-REQ-01`
- **Depends on:** None
- **Affected components:** `.github/workflows/release.yml`, `justfile`,
  `.github/dependabot.yml` (new)
- **Preconditions:** Plan approved.
- **Test or evidence first:**
  - Run `just publish-dry-run` and record that `target/package` exists
    afterwards.
  - Record the annotations of Release run 34772392506 (20 `ENOENT` errors,
    3 Node.js 20 warnings).
- **Implementation tasks:**
  1. In `release.yml`, add a final `rm -rf target/package` step with
     `if: always()` to the `verify` and `crates` jobs, with a comment
     naming the rust-cache cleanup it avoids.
  2. `publish-dry-run` removes `target/package` after the dry run.
  3. `upload-artifact@v7` and `download-artifact@v8`.
  4. `.github/dependabot.yml`: version 2, `package-ecosystem:
     github-actions`, directory `/`, schedule monthly.
- **Documentation/configuration/operations:** Developer guide, publishing
  and Dependabot notes.
- **Verification:**
  - `just publish-dry-run`, then `test ! -e target/package`.
  - `just lint-workflows` (actionlint).
  - Validate `dependabot.yml` against its schema with `check-jsonschema`
    if it is available, otherwise review it.
  - `just check`.
- **Completion criteria:** AC-01. AC-02 is verified at release step 8.
- **Rollback or recovery:** Revert the commit.
- **Builder stop conditions:** actionlint rejects the new action versions.

### PLAN-00005-STEP-02 — Working links and the Mermaid diagram

- **Objective:** Implement REQ-02 and REQ-03.
- **Requirements:** `PLAN-00005-REQ-02`, `PLAN-00005-REQ-03`
- **Depends on:** `PLAN-00005-STEP-01`
- **Affected components:** `README.md`, `scripts/check-links.sh` (new),
  `crates/passalong-cli/tests/link_check_script.rs` (new), `justfile`
- **Preconditions:** None.
- **Test or evidence first:**
  - `link_check_script.rs` builds fixture repositories in a temporary
    directory, like `release_tag_script.rs`. The script must fail, naming
    the file and link, on:
    - a relative link to a missing file;
    - a link to a missing heading anchor;
    - a relative link in a crate's `readme` file.
  - It must pass on absolute links, `#anchors`, and existing relative
    targets.
  - The tests fail first because the script does not exist.
  - Running the script on the repository fails on today's README.
- **Implementation tasks:**
  1. Write `scripts/check-links.sh`, portable to macOS (BSD) tools. It
     reads tracked Markdown from `git ls-files`, skips fenced code, and
     finds each crate's readme through `readme` in `crates/*/Cargo.toml`.
  2. Add a `links` recipe to the justfile and include it in `check`.
  3. Make the README's relative links absolute per D-05.
  4. Replace the "How it works" block with the Mermaid `flowchart` of
     D-06.
  5. Scan every external URL in tracked Markdown with `curl -L` and
     `Accept: text/html`. Record the results and fix any failure.
- **Documentation/configuration/operations:** Developer guide, link check.
- **Verification:**
  - `cargo test -p passalong --test link_check_script`.
  - `scripts/check-links.sh`.
  - The external scan script, recorded in the work log.
  - `just check`.
  - After the branch is pushed, the README on the branch page shows the
    diagram.
- **Completion criteria:** AC-03, AC-04, AC-05.
- **Rollback or recovery:** Revert.
- **Builder stop conditions:** An external link is broken and has no
  replacement.

### PLAN-00005-STEP-03 — `cat` without a log line, and `--quiet`

- **Objective:** Implement REQ-04 and REQ-05.
- **Requirements:** `PLAN-00005-REQ-04`, `PLAN-00005-REQ-05`
- **Depends on:** `PLAN-00005-STEP-02`
- **Affected components:** `crates/passalong-cli/src/{cli,app}.rs`,
  `crates/passalong-cli/src/commands/{cat,init,prune,serve}.rs`,
  `crates/passalong-core/src/config.rs` (level resolution, if the order is
  implemented there), `crates/passalong-cli/tests/cli_local_backend.rs`
- **Preconditions:** None.
- **Test or evidence first:**
  - A binary test: `cat` with the default level leaves stderr empty, and
    with `--log-level verbose` it shows "item printed".
  - Level-resolution unit tests: an explicit level beats `--quiet`, and
    `--quiet` beats `client.log_level`.
  - Binary tests with `--quiet` for `file`, `clipboard --stdin`,
    `load <id> <dest>`, `delete`, `prune --yes`, `list`, and
    `serve --status`: stdout and stderr are empty on success, and the exit
    code is unchanged (3 for `serve --status` when not running).
  - An error under `--quiet` still prints `error:` and exits 1, and `cat`'s
    content is unchanged.
  - Unit tests: `init`'s fingerprint and `prune`'s list reach the prompt
    stream under `--quiet` when a prompt follows.
- **Implementation tasks:**
  1. `cat` records "item printed" with `tracing::debug!`, which
     `LogLevel::level_filter` shows at `verbose` (syslog `notice`) and
     above.
  2. Add `-q`/`--quiet` as a global flag. `app` swaps standard output for a
     sink for every command except `cat`, and passes `quiet` to the level
     resolution.
  3. `init` and `prune` write prompt context through the prompt's stream
     when quiet and a prompt follows.
  4. `serve --daemon` passes the effective level to the background process.
- **Documentation/configuration/operations:** Usage: global options,
  `cat`. Configuration: logging order.
- **Verification:** `cargo test -p passalong`; `cargo test -p
  passalong-core --all-features config`; `just check`.
- **Completion criteria:** AC-06, AC-07.
- **Rollback or recovery:** Revert.
- **Builder stop conditions:** A command's output cannot be separated from
  its prompt without changing the `Prompt` trait's contract for tests.
  Adding a method with a default is allowed.

### PLAN-00005-STEP-04 — `passalong get <ID>`

- **Objective:** Implement REQ-06.
- **Requirements:** `PLAN-00005-REQ-06`
- **Depends on:** `PLAN-00005-STEP-03`
- **Affected components:** `crates/passalong-cli/src/commands/get.rs`
  (new), `cli.rs`, `app.rs`, `output.rs`, `tests/cli_local_backend.rs`
- **Preconditions:** None.
- **Test or evidence first:**
  - Unit tests over `TestStore`:
    - text, file, and clipboard-image items print the D-08 fields;
    - `--json` parses back to the same `ItemMeta` and matches the item's
      `list --json` element;
    - a prefix resolves;
    - an unknown id fails with "no item matches";
    - only one `meta.json` is read and the content is never opened
      (`FaultyFs` counters).
  - A binary test runs `get` against a local store, with and without
    `--json` and `--quiet`.
- **Implementation tasks:** Add the subcommand, the renderer in
  `output.rs`, and dispatch through `Lookup` with the chooser.
- **Documentation/configuration/operations:** Usage `get`.
- **Verification:** `cargo test -p passalong get`; `just check`.
- **Completion criteria:** AC-08.
- **Rollback or recovery:** Revert.
- **Builder stop conditions:** None expected.

### PLAN-00005-STEP-05 — `passalong check` and `Store::probe_write`

- **Objective:** Implement REQ-07.
- **Requirements:** `PLAN-00005-REQ-07`
- **Depends on:** `PLAN-00005-STEP-04`
- **Affected components:** `crates/passalong-core/src/store/{mod,fs_store}.rs`,
  `crates/passalong-cli/src/commands/check.rs` (new), `cli.rs`, `app.rs`,
  `crates/passalong-ssh/tests/sftp_docker.rs`,
  `crates/passalong-cli/tests/{cli_local_backend,cli_ssh_backend}.rs`
- **Preconditions:** None.
- **Test or evidence first:**
  - Core tests:
    - the default `probe_write` returns `NotSupported`;
    - the `FsStore` probe creates and removes `tmp/probe-<random>` and
      leaves `tmp/` and `items/` unchanged;
    - a failing create or remove is returned as an error (`FaultyFs`).
  - CLI unit tests with a fake backend: every line prints for a healthy
    store; a failing connect marks later checks `skipped` and exits 1; a
    read-only store fails `storage write`; a backend without a probe
    reports "not supported" and exits 0.
  - Binary tests: `check` against a local store passes; a missing config
    fails `config` with exit 1; an unwritable store directory fails
    `storage write`.
  - Docker tests: `probe_write` over SFTP; `check` against the test sshd
    passes, and a wrong pinned host key fails `server`.
- **Implementation tasks:**
  1. Add `WriteProbe` and `Store::probe_write`, with the `FsStore` override.
  2. Add the `check` command. It resolves and loads the config itself so a
     config error is reported as the `config` line, then opens the backend,
     calls `list_ids`, and calls `probe_write`.
- **Documentation/configuration/operations:** Usage `check`; architecture
  storage section.
- **Verification:** `cargo test -p passalong-core --all-features store`;
  `cargo test -p passalong check`; `just test-integration`; `just check`.
- **Completion criteria:** AC-09, AC-10.
- **Rollback or recovery:** Revert.
- **Builder stop conditions:** The probe cannot be made invisible to
  listings on one of the two backends.

### PLAN-00005-STEP-06 — `passalong install-service`

- **Objective:** Implement REQ-08.
- **Requirements:** `PLAN-00005-REQ-08`
- **Depends on:** `PLAN-00005-STEP-05`
- **Affected components:** `crates/passalong-cli/src/service.rs` (new),
  `crates/passalong-cli/src/commands/install_service.rs` (new), `cli.rs`,
  `app.rs`, `docs/service/*`, `tests/cli_local_backend.rs`
- **Preconditions:** None.
- **Test or evidence first:**
  - Unit tests with a recording `ServiceManager` fake, for both platforms
    through an explicit platform parameter so macOS logic is also tested
    on Linux:
    - the rendered unit or plist contains the absolute binary path,
      `--config <absolute>` when given, the working directory, and the
      macOS log path;
    - install runs `daemon-reload` then `enable --now`, or
      `launchctl bootstrap gui/<uid>`;
    - `--no-start` runs nothing;
    - an identical existing unit is reported, and a different one needs
      `--force`;
    - `--uninstall` stops, disables, and removes the unit, and "not
      installed" exits 0;
    - install is refused while the pid lock shows `serve` running;
    - a failing manager command is reported with the unit left in place
      and named.
  - A test asserts that `docs/service/*` equal the templates rendered with
    their documented placeholders.
  - A Linux binary test, with fake `systemctl` on `PATH` and `HOME` and
    `XDG_CONFIG_HOME` in a temporary directory, checks the written file
    and the recorded arguments.
- **Implementation tasks:**
  1. Templates and rendering in `service.rs`.
  2. A `ServiceManager` trait with the process-backed implementation
     (`systemctl --user`, `launchctl`).
  3. The command with `--no-start`, `--force`, and `--uninstall`, honouring
     `--quiet`.
  4. Regenerate `docs/service/*` from the templates, keeping their install
     comments.
- **Documentation/configuration/operations:** Usage `install-service`; the
  README quick start; architecture "Background `serve`".
- **Verification:** `cargo test -p passalong service`; `cargo test -p
  passalong --test cli_local_backend install_service`; `just check`
  (macOS CI runs the unit tests there).
- **Completion criteria:** AC-11, AC-12, AC-13.
- **Rollback or recovery:** Revert.
- **Builder stop conditions:** launchd needs a mechanism other than
  `launchctl bootstrap` and `bootout` on the macOS runner's version.

### PLAN-00005-STEP-07 — Documentation and v0.1.4 release preparation

- **Objective:** Implement REQ-09 and REQ-10.
- **Requirements:** `PLAN-00005-REQ-09`, `PLAN-00005-REQ-10`
- **Depends on:** `PLAN-00005-STEP-06`
- **Affected components:** `README.md`, `docs/`, `CHANGELOG.md`,
  `Cargo.toml`, `Cargo.lock`, version-pinning tests,
  `docs/release/v0.1.4.md` (new)
- **Preconditions:** None.
- **Test or evidence first:** Documentation step. The evidence is the
  consistency check, `scripts/check-links.sh`, and `passalong --version`.
- **Implementation tasks:**
  1. Complete the documentation of REQ-09 and remove the delivered backlog
     items.
  2. Bump to 0.1.4.
  3. Write the CHANGELOG `Unreleased` entries.
  4. Draft the release notes with absolute, tag-pinned links.
- **Documentation/configuration/operations:** This step.
- **Verification:** The documentation consistency check (every flag,
  command, and recipe documented); `scripts/check-links.sh`; `just check`;
  `just publish-dry-run`.
- **Completion criteria:** AC-14, AC-15.
- **Rollback or recovery:** Revert.
- **Builder stop conditions:** Documented behaviour disagrees with code.

### PLAN-00005-STEP-08 — Final quality gate

- **Objective:** Prove the plan.
- **Requirements:** `PLAN-00005-REQ-11`, `PLAN-00005-REQ-12`
- **Depends on:** `PLAN-00005-STEP-07`
- **Affected components:** None new.
- **Test or evidence first:** Verification-only step.
- **Implementation tasks:**
  1. Run `just ci`.
  2. Push `feature/00005-v0.1.4` and record the branch CI result for
     Linux, macOS, and Xvfb.
  3. Check that the Mermaid diagram renders on the branch page.
  4. Review the diff for road-map scope, and record coverage.
- **Documentation/configuration/operations:** None.
- **Verification:** `just ci`; GitHub CI; `git diff --stat
  origin/main..HEAD` scope review.
- **Completion criteria:** AC-16, AC-17.
- **Rollback or recovery:** Not applicable.
- **Builder stop conditions:** Any gate fails in a way that needs a scope or
  threshold change.

## 12. Cross-cutting concerns

| Area | Applicability | Planned action or reason not applicable | Step or requirement |
|---|---|---|---|
| Compatibility and APIs | Applicable | `probe_write` has a default; `WriteProbe` is `#[non_exhaustive]`; new subcommands and flags only; exit codes unchanged; storage and config formats unchanged | REQ-05 to REQ-08 |
| Data and migration | Not applicable | No storage or config format change | — |
| Security and privacy | Applicable | The write probe stays in `tmp/`; units run as the user and contain no secrets; `check` never prints the passphrase or key contents | REQ-07, REQ-08 |
| Performance and scale | Applicable | `get` reads one `meta.json`; `check` lists ids only | REQ-06, REQ-07 |
| Reliability and failure handling | Applicable | The probe cleans up on failure; `install-service` reports a failed manager command and leaves the unit to inspect; a refused start while `serve` runs | REQ-07, REQ-08 |
| Observability and operations | Applicable | `--quiet` keeps errors; the service logs to the journal or the macOS log file shared with `serve --daemon` | REQ-05, REQ-08 |
| Dependencies and supply chain | Applicable | No new crates; the artifact actions move majors; Dependabot proposes action updates | REQ-01 |
| Accessibility and UX | Applicable | Clean `cat` output, `--quiet`, one-command service install, a readable `check` report | REQ-04 to REQ-08 |
| Documentation and release | Applicable | Links checked in `just check`; README, usage, architecture, backlog, release notes | REQ-02, REQ-09, REQ-10 |
| Deployment and rollback | Applicable | Tag workflow with the `release` approval; `install-service --uninstall` reverses an install | REQ-08, REQ-10 |

## 13. Verification strategy

| Level | Evidence or command | When | Required result |
|---|---|---|---|
| Unit | `cargo test -p <crate> <module>` | Every step, red then green | Pass |
| Workspace gate | `just check` (now including `links`) | Every step | Exit 0 |
| Script | `cargo test -p passalong --test link_check_script` | STEP-02, 08 | Pass |
| Binary | `cargo test -p passalong --test cli_local_backend` | STEP-03 to 06 | Pass |
| SSH integration | `just test-integration` | STEP-05, 08 | Exit 0 |
| Workflows | `just lint-workflows` | STEP-01, 08 | No findings |
| Packaging | `just publish-dry-run`, then no `target/package` | STEP-01, 07, 08 | Exit 0 and absent |
| Links | `scripts/check-links.sh`; external URL scan | STEP-02, 07 | No broken links |
| Full | `just ci` locally and GitHub CI | STEP-08 | Pass on Linux, macOS, and Xvfb |
| Release | Annotations of the v0.1.4 Release run | Release step 8 | None of the `ENOENT` or Node.js 20 kinds |

## 14. Acceptance criteria

- [ ] `PLAN-00005-AC-01` `release.yml`'s `verify` and `crates` jobs end with an `if: always()` step that removes `target/package`; after `just publish-dry-run`, `target/package` does not exist; the workflows use `actions/upload-artifact@v7` and `actions/download-artifact@v8`; `.github/dependabot.yml` asks for monthly `github-actions` updates; `just lint-workflows` reports nothing.
- [ ] `PLAN-00005-AC-02` The v0.1.4 Release run has no `ENOENT` error annotations and no Node.js 20 deprecation warnings. This is checked after tagging, at release workflow step 8.
- [ ] `PLAN-00005-AC-03` `README.md` has no relative links except same-page `#anchors`. `scripts/check-links.sh` passes on the repository, is part of `just check`, and its tests show it fails on a missing relative target, a missing anchor, and a relative link in a crate's readme.
- [ ] `PLAN-00005-AC-04` Every external URL in tracked Markdown answers HTTP 200 in the recorded scan, or is replaced.
- [ ] `PLAN-00005-AC-05` "How it works" in `README.md` is a single ```` ```mermaid ```` flowchart showing the laptop, desktop, and phone, SSH/SFTP with a pinned host key, `server:/srv/passalong`, and `items/<time>-<hash>/{content,meta.json}`; the branch page on GitHub renders it.
- [ ] `PLAN-00005-AC-06` At the default level, `passalong cat <id>` writes exactly the item's bytes to stdout and nothing to stderr; with `--log-level verbose`, stderr has the "item printed" record.
- [ ] `PLAN-00005-AC-07` With `--quiet`: `file`, `clipboard --stdin`, `load <id> <dest>`, `delete`, `prune --yes`, `list`, `get`, `check`, `serve --status`, and `install-service --no-start` write nothing to stdout or stderr on success, and keep their exit codes. `cat` output is unchanged. Errors still print `error:` and exit 1. `--log-level` and `PASSALONG_LOG_LEVEL` override the level `--quiet` sets. `init`'s fingerprint and `prune`'s list still appear before their prompts.
- [ ] `PLAN-00005-AC-08` `passalong get <prefix>` prints the D-08 fields for text, file, and clipboard-image items; `--json` round-trips to the same `ItemMeta` as `list --json`; an unknown id exits 1 with "no item matches"; the command reads one `meta.json` and never opens `content`.
- [ ] `PLAN-00005-AC-09` `passalong check` prints `config`, `server`, `storage read`, and `storage write` lines and exits 0 against a local store and against the Docker sshd. A wrong host key fails `server`, marks later checks `skipped`, and exits 1. A missing config fails `config` with exit 1. An unwritable store fails `storage write` with exit 1.
- [ ] `PLAN-00005-AC-10` `Store::probe_write` defaults to `NotSupported`; `FsStore` creates and removes `tmp/probe-<random>`, leaving `tmp/` and `items/` as they were, locally and over SFTP; a backend without a probe makes `check` report "not supported" and still exit 0.
- [ ] `PLAN-00005-AC-11` On Linux, `install-service` writes `$XDG_CONFIG_HOME/systemd/user/passalong-serve.service` with `ExecStart=<absolute binary> serve` (plus `--config <absolute>` when given) and `WorkingDirectory=%h`, then runs `systemctl --user daemon-reload` and `systemctl --user enable --now passalong-serve.service`, as recorded by the fake. `--no-start` runs nothing. A different existing unit needs `--force`. `--uninstall` runs `disable --now`, removes the file, and reloads. Install is refused while `serve` runs.
- [ ] `PLAN-00005-AC-12` For macOS, the rendered plist in `~/Library/LaunchAgents/com.passalong.serve.plist` has `ProgramArguments` with the absolute binary, `WorkingDirectory` set to the home directory, and `StandardErrorPath` `~/Library/Logs/passalong/serve.log`; install runs `launchctl bootstrap gui/<uid> <plist>`, and `--uninstall` runs `launchctl bootout gui/<uid>/com.passalong.serve`. These tests pass on Linux and on CI's macOS job. On other platforms the command fails with the D-03 message.
- [ ] `PLAN-00005-AC-13` A test proves `docs/service/passalong-serve.service` and `docs/service/com.passalong.serve.plist` equal the templates rendered with their documented placeholders.
- [ ] `PLAN-00005-AC-14` The README, usage, configuration, architecture, and developer guide describe every new command, flag, and recipe. The documentation consistency check reports no gaps. The backlog no longer lists the v0.1.4 road map items or the Release-annotations Engineering entry.
- [ ] `PLAN-00005-AC-15` All crates are at `0.1.4`; `passalong --version` prints `passalong 0.1.4`; `CHANGELOG.md` `Unreleased` lists the v0.1.4 changes; `docs/release/v0.1.4.md` exists with only absolute links.
- [ ] `PLAN-00005-AC-16` `just ci` passes locally, and GitHub CI passes on Linux, macOS, and Xvfb for the final commit; line coverage is at least 80 %.
- [ ] `PLAN-00005-AC-17` The diff adds no GUI, Windows, Android, or backend code; `BackendRegistry` kinds are still `local` and `ssh`.

## 15. Risks and mitigations

| Risk | Likelihood | Impact | Mitigation or test | Owner/step |
|---|---|---|---|---|
| The rust-cache errors come from another path than `target/package` | Low | Low | The annotations name only `target/package/<crate>-<version>/tests/…`; AC-02 checks the next Release run | STEP-01 |
| `download-artifact@v8`'s digest check fails a release | Low | Medium | A mismatch means a damaged artifact, which should fail; the job can be re-run | STEP-01 |
| The link check script behaves differently on macOS | Medium | Low | Portable tools only; CI's macOS job runs `just check` | STEP-02 |
| Absolute README links point at `main`, not the viewed branch or tag | High | Low | Accepted: `main` holds the released docs; release notes stay tag-pinned | STEP-02 |
| crates.io renders the Mermaid diagram differently from GitHub | Low | Low | crates.io supports Mermaid since 2023; checked on the v0.1.4 page after release | STEP-02 |
| `--quiet` hides something a user must see | Medium | Medium | D-01 keeps errors, prompts, and prompt context; tests for `init` and `prune` | STEP-03 |
| `install-service` misbehaves on a real systemd or launchd | Medium | Medium | Commands follow the documented manual steps; the unit is left in place and named on failure; `--no-start` and `--uninstall` recover; the user can try it after the release | STEP-06 |
| A service and `serve --daemon` both run | Low | Medium | Install refuses while the pid lock is held; `serve`'s own lock refuses a second copy | STEP-06 |
| A crash between creating and removing the probe leaves `tmp/probe-*` | Low | Low | `prune` already removes stale staging directories under `tmp/` | STEP-05 |

## 16. Builder hand-off

- **Start condition:** User approval and a clean repository.
- **First step:** `PLAN-00005-STEP-01` — Quiet Release workflow.
- **Required sequence:** STEP-01 → 02 → 03 → 04 → 05 → 06 → 07 → 08, one
  commit per step without review pauses unless blocked.
- **Parallel-safe work:** None (single Builder).
- **Do not change:** approved scope, requirements, steps, acceptance
  criteria, or content outside Builder's permitted work-log area.
- **Escalate when:** a Builder stop condition triggers, or a decision in § 7
  turns out unworkable.
- **Completion hand-off:** the work log with evidence, local and GitHub CI
  results, coverage, and readiness for the release workflow's approval
  step. AC-02 remains open until the v0.1.4 Release run.

<!-- BUILDER_WORK_LOG_START -->
## 17. Builder Work Log

> [!warning] Builder-maintained section
> Delivery Planner creates this section. After approval, Builder may update only
> this delimited section and the Builder-maintained front-matter fields. Builder
> must preserve prior entries and use UTC timestamps.

### Step status

| Step | Status | Started (UTC) | Completed (UTC) | Evidence | Builder notes |
|---|---|---|---|---|---|
| PLAN-00005-STEP-01 | not-started | — | — | — | — |
| PLAN-00005-STEP-02 | not-started | — | — | — | — |
| PLAN-00005-STEP-03 | not-started | — | — | — | — |
| PLAN-00005-STEP-04 | not-started | — | — | — | — |
| PLAN-00005-STEP-05 | not-started | — | — | — | — |
| PLAN-00005-STEP-06 | not-started | — | — | — | — |
| PLAN-00005-STEP-07 | not-started | — | — | — | — |
| PLAN-00005-STEP-08 | not-started | — | — | — | — |

Allowed status values: `not-started`, `in-progress`, `blocked`, `completed`,
`skipped`. A skipped step requires explicit user approval recorded in Evidence.

### Execution log

| Timestamp (UTC) | Step | Event | Evidence or reference | Next action |
|---|---|---|---|---|

### Deviations and blockers

| Timestamp (UTC) | Step | Deviation or blocker | Impact | Decision required from |
|---|---|---|---|---|

None.

### Verification results

| Timestamp (UTC) | Step | Command or check | Result | Evidence |
|---|---|---|---|---|

### Completion summary

- **Implementation status:** `not-started`
- **Completed requirements:** None
- **Incomplete requirements:** All
- **Outstanding blockers:** None
- **Review request:** Not ready
<!-- BUILDER_WORK_LOG_END -->

## 18. Planning change log

| Timestamp (UTC) | Plan status | Change | Reason | Requested/approved by |
|---|---|---|---|---|
| 2026-09-13T20:40:20Z | draft | Created with 12 requirements, 8 steps, 17 acceptance criteria, and decisions D-01 to D-10; D-01 to D-04 were answered by the user before drafting (all as recommended), so none blocks approval | User request to plan v0.1.4 from the backlog road map | User |
| 2026-09-13T20:50:03Z | approved | Approved; `plan_status`, `approved_at`, and `build_ready` set | User approval after committing the draft (a50fb55) | User |

## 19. External references

- "Request for Mermaid inline diagram support", rust-lang/crates.io
  discussion #5724, opened 2022-12-21, maintainer update 2023-07-12; read
  2026-09-13; <https://github.com/rust-lang/crates.io/discussions/5724>.
- "Mermaid diagram does not render", rust-lang/crates.io issue #6786,
  closed 2023-07-14; read 2026-09-13;
  <https://github.com/rust-lang/crates.io/issues/6786>.
- `Swatinem/rust-cache` `src/cleanup.ts` at tag `v2` (v2.9.2, 2026-08-06),
  `cleanProfileTarget`; read 2026-09-13;
  <https://github.com/Swatinem/rust-cache/blob/v2/src/cleanup.ts>.
- `actions/upload-artifact` release v6.0.0 (2025-12-12, Node.js 24) and
  v7.0.1 (2026-04-10); read 2026-09-13;
  <https://github.com/actions/upload-artifact/releases>.
- `actions/download-artifact` releases v5.0.0 (2025-08-05, path change for
  downloads by ID), v7.0.0 (2025-12-12, Node.js 24), and v8.0.0
  (2026-02-26, digest mismatch fails by default); read 2026-09-13;
  <https://github.com/actions/download-artifact/releases>.
- crates.io rendered README of `passalong` 0.1.3,
  `https://crates.io/api/v1/crates/passalong/0.1.3/readme`; read 2026-09-13.

## 20. Confidence

**Medium.** The repository, the release run's annotations, the crates.io
rendering, and the actual command output were checked directly, and the four
behavioural decisions were settled by the user before drafting. The main
uncertainty is `install-service` against real systemd and launchd, which
tests can only fake. The fix for the Release annotations can only be
confirmed by the next Release run.
