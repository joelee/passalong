---
title: "Delivery Plan 00006: V0 1 5 Idle CPU And Choose TUI"
aliases:
  - "Plan 00006"
tags:
  - delivery-plan
  - implementation
  - opencode
type: delivery-plan
plan_id: "PLAN-00006"
plan_status: approved              # draft | approved | cancelled
plan_kind: initial                 # initial | superseding
created_at: "2026-09-14T07:48:16Z"
approved_at: "2026-09-14T08:15:45Z"
planner_agent: "Claude Code"
planner_model: "anthropic/claude-opus-5"
triggered_by: user                 # user | agent:<agent-name>
request_kind: direct               # idea | review | idea-and-review | direct | unplanned-query
repository: "joelee/passalong"
baseline_branch: "feature/00006-v0.1.5"
baseline_commit: "b30cec474302c200fa706aa08bea9aa242f7dd84"
source_ideas: []
source_reviews: []
previous_plan: null
requirements_count: 12
steps_count: 8
acceptance_criteria_count: 14
blocking_decisions: 0
build_ready: true
web_research_used: false
confidence: medium                # high | medium | low

# Builder-maintained front matter. Builder may update only these keys after
# explicit user approval; Delivery Planner initializes them.
implementation_status: in-progress # not-started | in-progress | blocked | completed | abandoned
builder_agent: "Claude Code"
builder_model: "anthropic/claude-opus-5"
execution_branch: "feature/00006-v0.1.5"
execution_started_at: "2026-09-14T08:16:47Z"
execution_updated_at: "2026-09-14T08:22:28Z"
execution_completed_at: null
current_step: "PLAN-00006-STEP-02"
---

# Delivery Plan 00006: V0 1 5 Idle CPU And Choose TUI

> [!abstract] Plan status: `approved`
> Deliver `passalong` v0.1.5, the first release since v0.1.3. It fixes the
> idle CPU bug in `serve` and measures what idle costs afterwards. It adds
> `passalong choose`, a terminal UI for picking and acting on items. It
> renames `install-service` to `service-install` and adds `service-remove`.
> `check` also reports whether `serve` is running, and CI gains an Android
> build check. It ships the unreleased v0.1.4 work too. The user settled
> D-01 to D-04 on 2026-09-14 (all as recommended); approved by the user
> at 2026-09-14T08:15:45Z; Builder-ready.

## 1. Objective and outcome

v0.1.4 was merged but, by the user's decision, never tagged or published.
v0.1.5 releases that work together with the maintainer's v0.1.5 road map:

1. An idle `serve` no longer spins. Today it uses about 134 % CPU.
   Afterwards its idle cost is measured, and tuned if it is still above
   about 1 % CPU.
2. `passalong choose` opens a full-screen list of items. It can be
   filtered, and loads, prints, shows, or deletes the chosen one.
3. `passalong service-install` and `passalong service-remove` replace
   `install-service` and its `--uninstall` option.
4. `passalong check` also reports whether `serve` is running.
5. CI builds `passalong-core` and `passalong-ssh` for Android (aarch64).
6. The release records cover everything since v0.1.3.

## 2. Source traceability

| Requirement | Source | Source location | Interpretation |
|---|---|---|---|
| PLAN-00006-REQ-01 | User; Repository | `docs/backlog.md` v0.1.5 "Bug: `serve` burns CPU when idle"; `crates/passalong-core/src/serve/drop_watcher.rs` `watch_folder`; notify 8.2.0 `src/inotify.rs` watch mask | The drop-folder watcher ignores the events its own scans cause |
| PLAN-00006-REQ-02 | User; D-03 | Same backlog item, "then measure the remaining idle cost" | Measure idle cost, and tune when above about 1 % CPU |
| PLAN-00006-REQ-03 | User | Backlog v0.1.5 "Rename `install-service` to `service-install` and add `service-remove`" | Rename, and a separate removal command |
| PLAN-00006-REQ-04 | User | Backlog v0.1.5 "`check` reports whether `serve` is running" | A `serve` line in `check` |
| PLAN-00006-REQ-05, REQ-06 | User; D-01, D-02, D-05, D-06 | Backlog v0.1.5 "`passalong choose` - invoke a TUI to select" | A ratatui item picker that acts on the choice |
| PLAN-00006-REQ-07 | User; D-04 | Backlog v0.1.5 "Android cross-compile check in CI" | A CI build check for aarch64-linux-android |
| PLAN-00006-REQ-08 | User; Repository | User decision on 2026-09-14 not to release v0.1.4; `CHANGELOG.md` Unreleased; `docs/release/v0.1.4.md` (draft) | Release records for v0.1.5 that include the v0.1.4 work |
| PLAN-00006-REQ-09 | Repository | Root `AGENTS.md` "Docs to maintain", "Backlog rules" | Documentation and backlog |
| PLAN-00006-REQ-10 | Repository | Root `AGENTS.md` "Non-negotiables" | Quality gates |
| PLAN-00006-REQ-11 | User | Road map: v0.1.x is the SSH-only CLI | Road-map guard |
| PLAN-00006-REQ-12 | Repository | PLAN-00005 AC-02, left open because v0.1.4 was not released | A Release run without annotations |

## 3. Repository baseline

| Field | Value |
|---|---|
| Repository | `joelee/passalong` |
| Branch | `feature/00006-v0.1.5`, from `main` at the user's request; not pushed |
| HEAD | `b30cec474302c200fa706aa08bea9aa242f7dd84`: merge of PR #6 (PLAN-00005, v0.1.4, not tagged) |
| Working tree at publication | Clean before allocation; the only change is this plan file |
| Release state | Last release v0.1.3 (crates.io and GitHub). The workspace is at 0.1.4, which will never be tagged or published |
| Open PRs | #7 (Dependabot: `actions/checkout` 5 to 7) |
| Applicable instructions | Root `AGENTS.md`; `docs/plans/AGENTS.md`; `docs/backlog.md` road map |

Findings from the planning research, 2026-09-14:

- **Idle CPU.**
  - A `serve` running v0.1.4 on the user's machine used 134 % CPU over
    5 s, with 38 threads. The busy threads were the tokio workers and the
    `notify` thread.
  - A sandboxed `serve` with no display, a local store, and an empty drop
    folder used 139 %.
  - The cause is confirmed from the source. notify 8.2.0's inotify backend
    watches `OPEN` and `CLOSE_NOWRITE`, and reports them as
    `EventKind::Access(Open)` and `Access(Close(Read))`. `watch_folder`
    forwards every event. `drop_loop`'s `scan` opens the folder to list it,
    so each scan produces events that trigger the next one.
- **ratatui 0.30.2 with crossterm 0.29.0**, tried in a scratch copy:
  - Licences pass.
  - `cargo deny check bans` fails on two duplicates, even with
    `default-features = false, features = ["crossterm"]`:
    - `hashbrown` 0.16.1 comes in through `kasuari` (the layout solver) and
      `ratatui-core`.
    - `foldhash` 0.2.0 comes in with it, while `foldhash` 0.1.5 already
      comes from `hashbrown` 0.15 through petgraph and wl-clipboard-rs.
  - `lru` and `kasuari` are required dependencies of `ratatui-core` 0.1.2,
    so no feature removes them.
  - About 35 new crates are added in total.
- **Android.**
  - `passalong-ssh` depends on `ring` 0.17, whose build script compiles C
    and so needs the NDK's clang for Android.
  - This machine has neither the NDK nor an Android Rust target. GitHub's
    Ubuntu runners have the NDK installed (`ANDROID_NDK_LATEST_HOME`), so
    the check can run only in CI.

## 4. Scope

### In scope

- The drop-folder watcher fix, with a regression test; idle measurement
  before and after; tuning if needed (D-03, D-09).
- `service-install` and `service-remove` (D-08).
- A `serve` line in `check` (D-07).
- `passalong choose` with ratatui and crossterm, and the two `deny.toml`
  skips it needs (D-05, D-06).
- An Android aarch64 `cargo check` job in CI, and a `just` recipe for it
  (D-10).
- Release records: the v0.1.4 work folded into v0.1.5, and version 0.1.5
  (D-11).

### Out of scope

- The Android client itself, GUI, and Windows (v0.2 and later).
- PR #7, Dependabot's `actions/checkout` update. The user can merge it
  separately; the plan works with either version.
- Keeping `install-service` as an alias. It was never released.
- Tagging, publishing, and the release finalisation commit.

## 5. Constraints and preserved decisions

- Every decision of PLAN-00001 to PLAN-00005 stays in force, except that
  D-03 of PLAN-00005 (the `install-service` command name and
  `--uninstall`) is replaced by D-08 here.
- `passalong-core` and `passalong-ssh` are published. Public API changes
  must be additive within 0.1.x.
- No new dependency may add a duplicate without a `deny.toml` skip that
  names the dependency path (`docs/developer-guide.md`, "Duplicate
  dependencies").
- **Tests must never touch the real clipboard, the user's services, or
  the user's running `serve`.** Measurements run a sandboxed `serve` with
  a temporary home, a local store, and no display.
- **Quality rules:**
  - TDD, with mocks for external interfaces.
  - `just check` green at every step, and line coverage at least 80 %.
  - Rustdoc for public items, and no `unsafe`.
- **Release steps** follow the root `AGENTS.md` "Release workflow". The
  Builder never tags or publishes.
- **Builder** works on `feature/00006-v0.1.5`, one commit per step, without
  review pauses unless blocked.

## 6. Assumptions

None. Unresolved matters are recorded as decisions.

## 7. Decisions and blockers

| ID | Decision or blocker | Resolution | Owner | Status |
|---|---|---|---|---|
| D-01 | What `choose` does with the chosen item. | **Confirmed by user (2026-09-14):** it acts on it. A full-screen list, newest first, that can be filtered; keys load it, print it, show its metadata, or delete it after confirmation. | User | Resolved |
| D-02 | Terminal UI library. | **Confirmed by user (2026-09-14):** ratatui with its crossterm backend. | User | Resolved |
| D-03 | How far to go on idle cost. | **Confirmed by user (2026-09-14):** fix the bug with a regression test, measure idle CPU, wake-ups, and threads, and tune when idle CPU is still above about 1 %. | User | Resolved |
| D-04 | Android targets. | **Confirmed by user (2026-09-14):** aarch64-linux-android only, as a `cargo check` in CI. | User | Resolved |
| D-05 | `choose` layout and keys. | **Resolved by planner:**<br>• **Layout:** a table with id, kind, name or preview, size, device, and age, newest first, from `Store::list`, and a status line with the key hints.<br>• **Keys:** Up/Down, PageUp/PageDown, and Home/End move. `/` starts a filter, which matches the id, name or preview, device, and kind, case-insensitively; Enter or Esc ends it. Enter loads the item, as `load` does without a destination. `c` prints it, as `cat` does. `g` shows its metadata, as `get` does. `d` deletes it after a `y` in the status line. `r` reloads the list. `q` or Esc quits.<br>• **Leaving the screen:** loading, printing, and showing metadata leave the full-screen view first, then print what `load`, `cat`, or `get` prints, so the output stays in the terminal. Deleting stays in the list.<br>• **Needs a terminal:** without one, `choose` fails with "choose needs a terminal". `--quiet` hides the result lines printed after leaving.<br>• **Always restores the terminal:** on errors and panics too. | Planner | Resolved |
| D-06 | ratatui's duplicate dependencies. | **Resolved by planner:** `ratatui = { version = "0.30", default-features = false, features = ["crossterm"] }`, using ratatui's re-exported crossterm rather than a second direct dependency. `deny.toml` gains two skips: `hashbrown@0.16.1` (kasuari through ratatui-core) and `foldhash@0.1.5` (hashbrown 0.15 through petgraph and wl-clipboard-rs, while kasuari's hashbrown 0.16 uses foldhash 0.2). No feature removes them (see § 3). | Planner | Resolved |
| D-07 | The `serve` line in `check`. | **Resolved by planner:** a fifth line, `serve`, after the storage lines. It shows `ok` with `running (pid N)`, or `off` with `not running`. It is informational and never fails. It is reported even when an earlier check failed, because it needs neither the config nor the server. | Planner | Resolved |
| D-08 | The service commands. | **Resolved by planner:** `passalong service-install [--no-start] [--force]` with `install-service`'s behaviour, and `passalong service-remove`, which does what `--uninstall` did. `install-service` and `--uninstall` are removed without an alias. The generated unit headers, `docs/service/*`, messages, and docs use the new names. | Planner | Resolved |
| D-09 | How idle cost is measured and tuned. | **Resolved by planner:**<br>• **How it is measured:** a sandboxed release-build `serve` (temporary `HOME`, local store, empty drop folder, no display) runs for 30 s. CPU is taken from `/proc/<pid>/stat` over the last 20 s, along with the thread count and the change in voluntary and involuntary context switches. This is recorded before the fix, after it, and after any tuning.<br>• **When it is tuned:** only if idle CPU is still above 1 %.<br>• **Tuning, in order of preference:** cap the runtime's blocking threads; build a current-thread runtime for `serve`; only then look at clipboard polling. Each change is kept only if it measurably helps and all tests still pass. | Planner | Resolved |
| D-10 | The Android check. | **Resolved by planner:** a new CI job, "Android build check (aarch64)", on `ubuntu-latest`. It installs the `aarch64-linux-android` target and points `CC_aarch64_linux_android` and `AR_aarch64_linux_android` at the runner's NDK clang (API 24) and llvm-ar. It runs `just android-check`, which is `cargo check --locked --target aarch64-linux-android -p passalong-core -p passalong-ssh --no-default-features`. The recipe documents the NDK variables it needs. The job is not a required check until the user adds it to the `main` ruleset. | Planner | Resolved |
| D-11 | Release records after the unreleased v0.1.4. | **Resolved by planner:** the CHANGELOG `Unreleased` section keeps the v0.1.4 entries, rewritten for the new service command names, and gains the v0.1.5 ones. `docs/release/v0.1.4.md` is removed, since no v0.1.4 tag exists. `docs/release/v0.1.5.md` covers everything since v0.1.3 and says v0.1.4 was not released. The version becomes 0.1.5. | Planner | Resolved |
| D-12 | Branch and version. | **Resolved by planner:** v0.1.5 on `feature/00006-v0.1.5`. | Planner | Resolved |

Blocking decisions: 0.

## 8. Affected architecture and components

| Area | Paths | Change |
|---|---|---|
| Drop watcher | `crates/passalong-core/src/serve/drop_watcher.rs` | Ignore `Access` events except close-after-write |
| Runtime (if tuned) | `crates/passalong-cli/src/main.rs`, `crates/passalong-cli/src/commands/serve.rs` | Per D-09 |
| Service commands | `crates/passalong-cli/src/{cli,app,service}.rs`, `commands/install_service.rs` (renamed), `docs/service/*` | `service-install`, `service-remove` |
| Check | `crates/passalong-cli/src/commands/check.rs`, `app.rs` | `serve` line |
| Choose | `crates/passalong-cli/src/commands/choose/` (new: state, view, terminal), `cli.rs`, `app.rs`, `Cargo.toml`, `deny.toml`, `Cargo.lock` | The terminal UI |
| CI | `.github/workflows/ci.yml`, `justfile` | Android job and recipe |
| Docs and records | `README.md`, `docs/{usage,architecture,developer-guide,configuration,backlog}.md`, `CHANGELOG.md`, `docs/release/v0.1.4.md` (removed), `docs/release/v0.1.5.md` (new) | Updated |

## 9. Requirement catalogue

### PLAN-00006-REQ-01 — The drop watcher ignores its own scans

- **Requirement:**
  - `watch_folder` forwards only events that can mean a change in the
    folder: create, modify, remove, rename, and close after writing.
  - It ignores `Access(Open)`, `Access(Close(Read))`, and other access
    events.
- **Rationale:** The confirmed cause of the idle CPU bug.
- **Source:** Backlog v0.1.5 bug entry.
- **Acceptance evidence:** AC-01.

### PLAN-00006-REQ-02 — Idle cost measured and tuned

- **Requirement:** Per D-09. The measurements are recorded in the work log
  and the release notes. After this plan, a sandboxed idle `serve` uses at
  most 1 % CPU, or the work log explains what remains and why.
- **Rationale:** The user asked for the remaining idle cost to be looked at.
- **Source:** Backlog; D-03.
- **Acceptance evidence:** AC-02.

### PLAN-00006-REQ-03 — `service-install` and `service-remove`

- **Requirement:** Per D-08.
- **Rationale:** User request.
- **Source:** Backlog v0.1.5.
- **Acceptance evidence:** AC-03.

### PLAN-00006-REQ-04 — `check` reports `serve`

- **Requirement:** Per D-07.
- **Rationale:** User request.
- **Source:** Backlog v0.1.5.
- **Acceptance evidence:** AC-04.

### PLAN-00006-REQ-05 — `passalong choose`

- **Requirement:** Per D-01 and D-05.
- **Rationale:** User request.
- **Source:** Backlog v0.1.5.
- **Acceptance evidence:** AC-05, AC-06, AC-07.

### PLAN-00006-REQ-06 — Terminal UI dependencies

- **Requirement:** Per D-02 and D-06. `just audit` passes, and no duplicate
  is added beyond the two documented skips.
- **Rationale:** Duplicate-dependency policy.
- **Source:** `deny.toml`; developer guide.
- **Acceptance evidence:** AC-08.

### PLAN-00006-REQ-07 — Android build check in CI

- **Requirement:** Per D-04 and D-10.
- **Rationale:** Keeps the library crates usable for the planned Android
  client.
- **Source:** Backlog v0.1.5.
- **Acceptance evidence:** AC-09.

### PLAN-00006-REQ-08 — v0.1.5 release records

- **Requirement:** Per D-11. The version is 0.1.5 and the release notes are
  drafted with absolute, tag-pinned links.
- **Rationale:** v0.1.4 was not released.
- **Source:** User decision; root `AGENTS.md` "Release workflow".
- **Acceptance evidence:** AC-11.

### PLAN-00006-REQ-09 — Documentation

- **Requirement:**
  - **README:** the command table and quick start.
  - **`docs/usage.md`:** new `choose`, `service-install`, and
    `service-remove` sections; the `check` `serve` line.
  - **`docs/architecture.md`:** the watcher events, the `choose` flow, and
    any runtime change.
  - **`docs/developer-guide.md`:** the `android-check` recipe and the new
    skips.
  - **`docs/backlog.md`:** the delivered v0.1.5 items removed.
- **Rationale:** Documentation rules.
- **Source:** Root `AGENTS.md`.
- **Acceptance evidence:** AC-10.

### PLAN-00006-REQ-10 — Quality gates

- **Requirement:**
  - TDD evidence per step.
  - `just check` green at every step and `just ci` green locally.
  - Branch CI green on Linux, macOS, Xvfb, and the new Android job.
  - Line coverage at least 80 %.
- **Rationale:** Repository rules.
- **Source:** Root `AGENTS.md`.
- **Acceptance evidence:** AC-12.

### PLAN-00006-REQ-11 — Road map guard

- **Requirement:** No GUI, Windows, or Android client code. The Android
  job only builds the library crates. The `BackendRegistry` kinds remain
  `local` and `ssh`.
- **Rationale:** v0.1.x is the SSH-only CLI.
- **Source:** User road map.
- **Acceptance evidence:** AC-13.

### PLAN-00006-REQ-12 — Release run without annotations

- **Requirement:** The v0.1.5 Release run has no `ENOENT` error annotations
  and no Node.js 20 warnings. This carries over PLAN-00005 AC-02.
- **Rationale:** PLAN-00005's workflow fix is first exercised by this
  release.
- **Source:** PLAN-00005 AC-02.
- **Acceptance evidence:** AC-14.

## 10. Delivery strategy

1. **Idle CPU (STEP-01, STEP-02).** This is the bug users feel. The
   baseline is measured before any change.
2. **Service rename, then the `check` line (STEP-03, STEP-04).** Small
   changes to code built in PLAN-00005.
3. **`choose` (STEP-05).** The largest step: dependencies, state machine,
   rendering, terminal handling, and actions.
4. **Android CI (STEP-06).** Independent; verified on the pushed branch.
5. **Records and final gate (STEP-07, STEP-08).**

Each step ends with `just check` green and one commit. Docker tests run at
STEP-04 and STEP-08.

## 11. Detailed implementation steps

### PLAN-00006-STEP-01 — Drop watcher ignores its own scans

- **Objective:** Implement REQ-01, and record the idle baseline.
- **Requirements:** `PLAN-00006-REQ-01`, `PLAN-00006-REQ-02`
- **Depends on:** None
- **Affected components:** `crates/passalong-core/src/serve/drop_watcher.rs`
- **Preconditions:** Plan approved.
- **Test or evidence first:**
  - The D-09 idle measurement on the baseline release build.
  - A watcher test fails first. With `watch_folder` on a temporary folder,
    calling `scan` on it produces no change notification within 300 ms.
    Creating, writing, renaming, and removing a file each still produces
    one.
- **Implementation tasks:**
  1. Classify notify events: forward `Create`, `Modify`, `Remove`, and
     `Access(Close(Write))`; drop other `Access` kinds and `Other`.
  2. Measure again with the fixed release build.
- **Documentation/configuration/operations:** Architecture, "`serve`".
- **Verification:** `cargo test -p passalong-core --all-features
  drop_watcher`; `cargo test -p passalong-core --all-features --test
  serve_local`; the measurement; `just check`.
- **Completion criteria:** AC-01, and the before and after figures
  recorded.
- **Rollback or recovery:** Revert.
- **Builder stop conditions:** Idle CPU stays above 50 % after the fix,
  which would mean another cause.

### PLAN-00006-STEP-02 — Idle tuning

- **Objective:** Complete REQ-02 per D-09.
- **Requirements:** `PLAN-00006-REQ-02`
- **Depends on:** `PLAN-00006-STEP-01`
- **Affected components:** Per D-09, only if needed:
  `crates/passalong-cli/src/main.rs`, `commands/serve.rs`,
  `crates/passalong-core/src/serve/mod.rs`
- **Preconditions:** The STEP-01 measurement.
- **Test or evidence first:** The STEP-01 figures. For each change: a
  unit or binary test that the behaviour is unchanged, and the measurement
  before and after.
- **Implementation tasks:**
  1. If idle CPU is at most 1 %, record it and change nothing.
  2. Otherwise apply D-09's tuning in order, keeping only changes that
     help.
- **Documentation/configuration/operations:** Architecture, if the runtime
  changes.
- **Verification:** The measurement; `just check`.
- **Completion criteria:** AC-02.
- **Rollback or recovery:** Revert.
- **Builder stop conditions:** Tuning changes behaviour that tests cover.

### PLAN-00006-STEP-03 — `service-install` and `service-remove`

- **Objective:** Implement REQ-03.
- **Requirements:** `PLAN-00006-REQ-03`
- **Depends on:** `PLAN-00006-STEP-02`
- **Affected components:** `crates/passalong-cli/src/{cli,app,service}.rs`,
  `crates/passalong-cli/src/commands/install_service.rs` (renamed to
  `service.rs` or similar), `tests/cli_local_backend.rs`, `docs/service/*`
- **Preconditions:** None.
- **Test or evidence first:**
  - Parse tests for `service-install [--no-start] [--force]` and
    `service-remove`, and that `install-service` and `--uninstall` are
    rejected.
  - The existing unit and binary tests, renamed. The removal tests call
    `service-remove`.
  - The generated headers name `service-remove`.
- **Implementation tasks:** Rename the command, split removal into its own
  subcommand, and update messages, headers, and `docs/service/*`.
- **Documentation/configuration/operations:** Usage, README, architecture,
  CHANGELOG (the Unreleased entries are rewritten).
- **Verification:** `cargo test -p passalong`; `just check`.
- **Completion criteria:** AC-03.
- **Rollback or recovery:** Revert.
- **Builder stop conditions:** None expected.

### PLAN-00006-STEP-04 — `check` reports `serve`

- **Objective:** Implement REQ-04.
- **Requirements:** `PLAN-00006-REQ-04`
- **Depends on:** `PLAN-00006-STEP-03`
- **Affected components:** `crates/passalong-cli/src/commands/check.rs`,
  `app.rs`, tests
- **Preconditions:** None.
- **Test or evidence first:** Unit tests for the `serve` line, using the
  existing `Status` values, when running with a pid, running without a
  pid, and not running. The line is present after a config failure. The
  exit code does not depend on it. A binary test holds the pid lock in a
  temporary state folder and expects `running`.
- **Implementation tasks:** Pass `daemon::status` into `check::run` and
  print the fifth line.
- **Documentation/configuration/operations:** Usage, `check`.
- **Verification:** `cargo test -p passalong check`; `just
  test-integration`; `just check`.
- **Completion criteria:** AC-04.
- **Rollback or recovery:** Revert.
- **Builder stop conditions:** None expected.

### PLAN-00006-STEP-05 — `passalong choose`

- **Objective:** Implement REQ-05 and REQ-06.
- **Requirements:** `PLAN-00006-REQ-05`, `PLAN-00006-REQ-06`
- **Depends on:** `PLAN-00006-STEP-04`
- **Affected components:** `crates/passalong-cli/Cargo.toml`, `Cargo.lock`,
  `deny.toml`, `crates/passalong-cli/src/commands/choose/` (new), `cli.rs`,
  `app.rs`, `tests/cli_local_backend.rs`
- **Preconditions:** None.
- **Test or evidence first:**
  - `just audit` fails on the two duplicates before the skips are added.
    This is recorded as evidence, not committed as a failure.
  - **State-machine tests over synthetic key events:**
    - movement and clamping;
    - `/` filter matching, and ending the filter with Enter or Esc;
    - Enter, `c`, and `g` return the matching action for the selected
      item;
    - `d` then `y` deletes, and `d` then another key cancels;
    - `r` reloads;
    - `q` and Esc quit.
  - **Rendering tests with ratatui's `TestBackend`:** the columns, the
    selection, the filter line, and the key hints.
  - **Action tests over `TestStore`:** the chosen action runs the same
    code as `load`, `cat`, `get`, or `delete`.
  - **A binary test:** without a terminal, `choose` exits 1 with "choose
    needs a terminal".
- **Implementation tasks:**
  1. Add the dependency and skips per D-06.
  2. Write the state machine, independent of the terminal.
  3. Write the view.
  4. Add the terminal guard, which enters and leaves the alternate screen
     and raw mode, and restores the terminal on drop and in a panic hook.
  5. Run the actions through the existing command functions.
- **Documentation/configuration/operations:** Usage `choose`, README,
  architecture, developer guide (skips).
- **Verification:** `cargo test -p passalong choose`; `just audit`;
  `just check`.
- **Completion criteria:** AC-05, AC-06, AC-07, AC-08.
- **Rollback or recovery:** Revert, including `Cargo.lock` and `deny.toml`.
- **Builder stop conditions:** ratatui needs more duplicates than D-06
  lists.

### PLAN-00006-STEP-06 — Android build check in CI

- **Objective:** Implement REQ-07.
- **Requirements:** `PLAN-00006-REQ-07`
- **Depends on:** `PLAN-00006-STEP-05`
- **Affected components:** `.github/workflows/ci.yml`, `justfile`,
  developer guide
- **Preconditions:** None.
- **Test or evidence first:** `just lint-workflows` on the new job; the
  job's first run on the pushed branch is its evidence (no NDK locally).
- **Implementation tasks:** Add the `android-check` recipe and the CI job
  per D-10.
- **Documentation/configuration/operations:** Developer guide: the recipe,
  the NDK variables, and that the check is CI-only unless an NDK is
  installed.
- **Verification:** `just lint-workflows`; `just check`; push the branch
  and record the Android job's result.
- **Completion criteria:** AC-09.
- **Rollback or recovery:** Revert.
- **Builder stop conditions:** `passalong-core` or `passalong-ssh` does not
  build for Android without code changes beyond `cfg` gates.

### PLAN-00006-STEP-07 — Documentation and v0.1.5 release preparation

- **Objective:** Implement REQ-08 and REQ-09.
- **Requirements:** `PLAN-00006-REQ-08`, `PLAN-00006-REQ-09`
- **Depends on:** `PLAN-00006-STEP-06`
- **Affected components:** Docs, `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock`,
  version tests, `docs/release/`
- **Preconditions:** None.
- **Test or evidence first:** Documentation step. The evidence is the
  documentation consistency check, `scripts/check-links.sh`, and
  `passalong --version`.
- **Implementation tasks:**
  1. Complete the docs.
  2. Remove the delivered backlog items.
  3. Bump to 0.1.5.
  4. Remove `docs/release/v0.1.4.md`.
  5. Draft `docs/release/v0.1.5.md` with the idle measurements.
- **Documentation/configuration/operations:** This step.
- **Verification:** Consistency check; `just check`; `just publish-dry-run
  --allow-dirty`.
- **Completion criteria:** AC-10, AC-11.
- **Rollback or recovery:** Revert.
- **Builder stop conditions:** Documented behaviour disagrees with code.

### PLAN-00006-STEP-08 — Final quality gate

- **Objective:** Prove the plan.
- **Requirements:** `PLAN-00006-REQ-10`, `PLAN-00006-REQ-11`
- **Depends on:** `PLAN-00006-STEP-07`
- **Affected components:** None new.
- **Test or evidence first:** Verification-only step.
- **Implementation tasks:**
  1. Run `just ci`.
  2. Push and record CI for Linux, macOS, Xvfb, and Android.
  3. Review the diff for scope, and record coverage.
- **Documentation/configuration/operations:** None.
- **Verification:** `just ci`; GitHub CI; scope review.
- **Completion criteria:** AC-12, AC-13. AC-14 is checked at release step
  8.
- **Rollback or recovery:** Not applicable.
- **Builder stop conditions:** A gate fails in a way that needs a scope
  change.

## 12. Cross-cutting concerns

| Area | Applicability | Planned action or reason not applicable | Step or requirement |
|---|---|---|---|
| Compatibility and APIs | Applicable | `install-service` renamed without an alias (never released); no public library API removed; storage and config unchanged | REQ-03 |
| Data and migration | Not applicable | No format change | — |
| Security and privacy | Applicable | `choose` shows only metadata already in `list`; deleting needs a `y` confirmation | REQ-05 |
| Performance and scale | Applicable | The idle CPU fix and measurement; `choose` reads `Store::list` once and on `r` | REQ-01, REQ-02, REQ-05 |
| Reliability and failure handling | Applicable | The terminal is restored on error and panic; errors are shown in the status line | REQ-05 |
| Observability and operations | Applicable | `check` reports `serve`; the idle figures are recorded | REQ-02, REQ-04 |
| Dependencies and supply chain | Applicable | ratatui and crossterm (MIT), two documented skips, `just audit` passes | REQ-06 |
| Accessibility and UX | Applicable | Keyboard-only picker with visible key hints; no colour-only meaning | REQ-05 |
| Documentation and release | Applicable | Docs, backlog, release records for v0.1.5 | REQ-08, REQ-09 |
| Deployment and rollback | Applicable | Tag workflow; `service-remove` reverses `service-install` | REQ-03, REQ-12 |

## 13. Verification strategy

| Level | Evidence or command | When | Required result |
|---|---|---|---|
| Unit | `cargo test -p <crate> <module>` | Every step, red then green | Pass |
| Workspace gate | `just check` | Every step | Exit 0 |
| Idle measurement | Sandboxed `serve`, 30 s, per D-09 | STEP-01, 02, 08 | Recorded; ≤ 1 % CPU or explained |
| Audit | `just audit` | STEP-05, 08 | Exit 0 |
| SSH integration | `just test-integration` | STEP-04, 08 | Exit 0 |
| Workflows | `just lint-workflows` | STEP-06, 08 | No findings |
| Android | CI job "Android build check (aarch64)" | STEP-06, 08 | Success |
| Full | `just ci` and GitHub CI | STEP-08 | Pass |
| Release | Annotations of the v0.1.5 Release run | Release step 8 | None |

## 14. Acceptance criteria

- [ ] `PLAN-00006-AC-01` `watch_folder` produces no change notification when the folder is only scanned, and one for each create, write, rename, and removal of a file; the test failed before the fix.
- [ ] `PLAN-00006-AC-02` The work log records idle CPU, threads, and context switches of a sandboxed `serve` before the fix, after it, and after any tuning; idle CPU after this plan is at most 1 %, or the work log explains the remainder.
- [ ] `PLAN-00006-AC-03` `service-install [--no-start] [--force]` and `service-remove` behave as `install-service` and `--uninstall` did; `install-service` and `--uninstall` are rejected as unknown; generated units, `docs/service/*`, and messages name `service-remove`.
- [ ] `PLAN-00006-AC-04` `check` prints a fifth line, `serve`, with `ok` and `running (pid N)` or `off` and `not running`, also after an earlier failure, and it never changes the exit code.
- [ ] `PLAN-00006-AC-05` `choose` shows items newest first with id, kind, name or preview, size, device, and age; `/` filters case-insensitively on id, name or preview, device, and kind; movement keys work and clamp at the ends.
- [ ] `PLAN-00006-AC-06` In `choose`, Enter loads, `c` prints, and `g` shows the metadata of the selected item after leaving the full-screen view, through the same code as `load`, `cat`, and `get`; `d` deletes only after `y`; `r` reloads; `q` and Esc quit; the terminal is restored after every exit, including errors.
- [ ] `PLAN-00006-AC-07` Without a terminal, `passalong choose` exits 1 with "choose needs a terminal"; `--quiet` hides the result lines after an action.
- [ ] `PLAN-00006-AC-08` ratatui is a dependency with `default-features = false` and the crossterm feature, with no separate crossterm; `just audit` passes with exactly the two D-06 skips added.
- [ ] `PLAN-00006-AC-09` The CI job "Android build check (aarch64)" runs `just android-check` and succeeds on the branch.
- [ ] `PLAN-00006-AC-10` The README, usage, architecture, and developer guide describe `choose`, `service-install`, `service-remove`, the `serve` line, the watcher fix, and `android-check`; the consistency check reports no gaps; the delivered v0.1.5 backlog items are removed.
- [ ] `PLAN-00006-AC-11` All crates are at 0.1.5; `passalong --version` prints `passalong 0.1.5`; `CHANGELOG.md` `Unreleased` lists the v0.1.4 and v0.1.5 changes with the new command names; `docs/release/v0.1.4.md` is gone; `docs/release/v0.1.5.md` exists with only absolute links.
- [ ] `PLAN-00006-AC-12` `just ci` passes locally and GitHub CI passes on Linux, macOS, Xvfb, and Android for the final commit; line coverage is at least 80 %.
- [ ] `PLAN-00006-AC-13` The diff adds no GUI, Windows, or Android client code; `BackendRegistry` kinds are still `local` and `ssh`.
- [ ] `PLAN-00006-AC-14` The v0.1.5 Release run has no `ENOENT` error annotations and no Node.js 20 warnings; checked after tagging, at release workflow step 8.

## 15. Risks and mitigations

| Risk | Likelihood | Impact | Mitigation or test | Owner/step |
|---|---|---|---|---|
| Filtering events misses a real change | Low | Medium | Tests for create, write, rename, and remove; the periodic rescan remains a backstop | STEP-01 |
| macOS (FSEvents) reports events differently | Medium | Low | Classification by `EventKind`; the macOS CI job runs the watcher tests; the rescan backstop | STEP-01 |
| Idle cost has a second cause | Low | Medium | Measured after the fix; STEP-02 and its stop condition | STEP-01, 02 |
| A TUI is hard to test end to end | Medium | Medium | A terminal-independent state machine, `TestBackend` rendering, and action tests; the user tries it | STEP-05 |
| The terminal is left in raw mode after a crash | Low | High | A drop guard plus a panic hook, tested | STEP-05 |
| ~35 new crates enlarge the binary and the audit surface | High | Low | `default-features = false`; `just audit`; the sizes are recorded | STEP-05 |
| The runner's NDK layout changes | Low | Low | Paths read from `ANDROID_NDK_LATEST_HOME`; the job fails loudly | STEP-06 |

## 16. Builder hand-off

- **Start condition:** User approval and a clean repository.
- **First step:** `PLAN-00006-STEP-01`.
- **Required sequence:** STEP-01 → … → STEP-08, one commit per step
  without review pauses unless blocked.
- **Parallel-safe work:** None (single Builder).
- **Do not change:** approved scope, requirements, steps, acceptance
  criteria, or content outside Builder's permitted work-log area.
- **Escalate when:** a stop condition triggers, or a § 7 decision proves
  unworkable.
- **Completion hand-off:** the work log with evidence, the idle figures,
  CI results, and coverage. AC-14 remains open until the Release run.

<!-- BUILDER_WORK_LOG_START -->
## 17. Builder Work Log

> [!warning] Builder-maintained section
> Delivery Planner creates this section. After approval, Builder may update only
> this delimited section and the Builder-maintained front-matter fields. Builder
> must preserve prior entries and use UTC timestamps.

### Step status

| Step | Status | Started (UTC) | Completed (UTC) | Evidence | Builder notes |
|---|---|---|---|---|---|
| PLAN-00006-STEP-01 | completed | 2026-09-14T08:16:47Z | 2026-09-14T08:22:28Z | Commit `build: complete PLAN-00006-STEP-01 - Drop watcher ignores its own scans`; `just check` green, 92.48% lines; idle 136.45 % to 0.00 % CPU | AC-01. Cause confirmed: notify 8.2 inotify reports OPEN and CLOSE_NOWRITE as Access events, and every scan opened the folder. watch_folder now forwards only events for which is_change is true (all but Access, plus Access(Close(Write))). Architecture drop watcher bullet and CHANGELOG Fixed entry updated |
| PLAN-00006-STEP-02 | not-started | — | — | — | — |
| PLAN-00006-STEP-03 | not-started | — | — | — | — |
| PLAN-00006-STEP-04 | not-started | — | — | — | — |
| PLAN-00006-STEP-05 | not-started | — | — | — | — |
| PLAN-00006-STEP-06 | not-started | — | — | — | — |
| PLAN-00006-STEP-07 | not-started | — | — | — | — |
| PLAN-00006-STEP-08 | not-started | — | — | — | — |

Allowed status values: `not-started`, `in-progress`, `blocked`, `completed`,
`skipped`. A skipped step requires explicit user approval recorded in Evidence.

### Execution log

| Timestamp (UTC) | Step | Event | Evidence or reference | Next action |
|---|---|---|---|---|
| 2026-09-14T08:16:47Z | PLAN-00006 | Plan approved (commit 693aa90); Builder starts on feature/00006-v0.1.5 | `docs(plan): approve PLAN-00006 - V0 1 5 Idle CPU And Choose TUI` | Begin PLAN-00006-STEP-01 |
| 2026-09-14T08:16:47Z | PLAN-00006-STEP-01 | Started | — | Red phase |
| 2026-09-14T08:22:28Z | PLAN-00006-STEP-01 | Verified and committed (continuous execution authorised by the user) | `build: complete PLAN-00006-STEP-01 - Drop watcher ignores its own scans` | Begin PLAN-00006-STEP-02 |

### Deviations and blockers

| Timestamp (UTC) | Step | Deviation or blocker | Impact | Decision required from |
|---|---|---|---|---|

None.

### Verification results

| Timestamp (UTC) | Step | Command or check | Result | Evidence |
|---|---|---|---|---|
| 2026-09-14T08:22:28Z | PLAN-00006-STEP-01 | Idle baseline (D-09): sandboxed release serve on b30cec4, no display, local store, empty drop folder, 30 s run, last 20 s measured | Recorded | cpu 136.45 %, threads 37, 7,901,150 context switches in 20 s (395,057/s) |
| 2026-09-14T08:22:28Z | PLAN-00006-STEP-01 | Red: `cargo test -p passalong-core --all-features --lib drop_watcher` | Exit 101 (expected) | scanning_the_folder_does_not_trigger_the_watcher FAILED: a scan or a read notified the watcher; the create/write/rename/remove test passed |
| 2026-09-14T08:22:28Z | PLAN-00006-STEP-01 | `cargo test -p passalong-core --all-features --lib drop_watcher` | Pass | 12 passed: scan and read do not notify; create, write, rename, remove each notify; only_events_that_may_change_files_count (Access other than close-after-write ignored) |
| 2026-09-14T08:22:28Z | PLAN-00006-STEP-01 | Idle after the fix (D-09, same method) | Recorded | cpu 0.00 %, threads 35, 41 context switches in 20 s (2/s); the sandbox has no clipboard, so clipboard polling is measured in STEP-02 |
| 2026-09-14T08:22:28Z | PLAN-00006-STEP-01 | `just check` | Exit 0 | Lines 92.48% (first run failed clippy type_complexity on the test; rewritten as sequential steps) |

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
| 2026-09-14T07:48:16Z | draft | Created with 12 requirements, 8 steps, 14 acceptance criteria, and decisions D-01 to D-12; D-01 to D-04 were answered by the user before drafting (all as recommended), so none blocks approval | User request to plan v0.1.5 from the backlog road map | User |
| 2026-09-14T08:15:45Z | approved | Approved; `plan_status`, `approved_at`, and `build_ready` set | User approval after committing the draft (07eb565) | User |

## 19. External references

None. The planning research used the repository, the installed notify
8.2.0 and ratatui-core 0.1.2 sources, `cargo tree` and `cargo deny` in a
scratch copy, and measurements on this machine.

## 20. Confidence

**Medium.** The idle CPU cause is confirmed in notify's source and
reproduced in a sandbox, and the dependency cost of ratatui was measured
with `cargo deny`. The main uncertainties are how testable the terminal UI
is end to end, and the Android job, which can only be proved in CI.
