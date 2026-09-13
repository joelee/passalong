---
title: "Delivery Plan 00004: V0 1 3 Review Fixes And Pull Ids"
aliases:
  - "Plan 00004"
tags:
  - delivery-plan
  - implementation
  - opencode
type: delivery-plan
plan_id: "PLAN-00004"
plan_status: approved              # draft | approved | cancelled
plan_kind: initial                 # initial | superseding
created_at: "2026-09-13T11:01:43Z"
approved_at: "2026-09-13T11:21:15Z"
planner_agent: "Claude Code"
planner_model: "anthropic/claude-opus-5"
triggered_by: user                 # user | agent:<agent-name>
request_kind: direct               # idea | review | idea-and-review | direct | unplanned-query
repository: "joelee/passalong"
baseline_branch: "feature/00004-v0.1.3"
baseline_commit: "c44d14d4e243b8cf1f5c63a27e6a4cb279553123"
source_ideas: []
source_reviews:
  - "docs/reviews/00001-v0_1_2_Release_Review.md"
previous_plan: null
requirements_count: 9
steps_count: 6
acceptance_criteria_count: 11
blocking_decisions: 0
build_ready: true
web_research_used: false
confidence: medium                # high | medium | low

# Builder-maintained front matter. Builder may update only these keys after
# explicit user approval; Delivery Planner initializes them.
implementation_status: completed # not-started | in-progress | blocked | completed | abandoned
builder_agent: "Claude Code"
builder_model: "anthropic/claude-opus-5"
execution_branch: "feature/00004-v0.1.3"
execution_started_at: "2026-09-13T11:21:15Z"
execution_updated_at: "2026-09-13T11:53:56Z"
execution_completed_at: "2026-09-13T11:53:56Z"
current_step: null
---

# Delivery Plan 00004: V0 1 3 Review Fixes And Pull Ids

> [!abstract] Plan status: `approved`
> Deliver `passalong` v0.1.3, a small release: the release-notes link fix
> for issue #4 (already committed on the branch), the two non-blocking
> findings of Code Review 00001, and, subject to D-03, pull mode tracking
> the item ids it has seen instead of relying on device clocks. D-03 was
> confirmed as proposed (include); approved by the user at 2026-09-13T11:21:15Z;
> Builder-ready.

## 1. Objective and outcome

v0.1.2 shipped clipboard images, pull mode, downloads, and `passalong cat`.
v0.1.3 fixes what its release and review found:

1. Release notes link to their delivery plans with absolute URLs, and the
   release check rejects relative links (GitHub issue #4).
2. Choosing between ambiguous ids reads only the metadata of the items it
   shows, not the whole store (REV-00001-MED-01).
3. Two downloads of the same name into the same directory can never pick the
   same file name (REV-00001-LOW-01).
4. Per D-03, pull mode applies every new item from another device exactly
   once, whatever that device's clock says.

## 2. Source traceability

| Requirement | Source | Evidence |
|---|---|---|
| PLAN-00004-REQ-01 | User request item 1 (2026-09-13); GitHub issue #4 "Incorrect links to Delivery Plans on Release Notes" | Fixed on the baseline by c44d14d |
| PLAN-00004-REQ-02, REQ-03 | `docs/reviews/00001-v0_1_2_Release_Review.md` REV-00001-MED-01; `docs/backlog.md` "Targeted metadata reads for ambiguous ids" | `crates/passalong-cli/src/resolve.rs` calls `store.list()` |
| PLAN-00004-REQ-04 | `docs/reviews/00001-v0_1_2_Release_Review.md` REV-00001-LOW-01; `docs/backlog.md` "Atomic download names" | `crates/passalong-core/src/download.rs` `free_target` only checks existence |
| PLAN-00004-REQ-05 | User request item 1 (pick backlog items); `docs/backlog.md` "Pull mode and clock skew"; D-03 | `crates/passalong-core/src/serve/pull.rs` keeps a position compared with `list_after` |
| PLAN-00004-REQ-06, REQ-07 | Root `AGENTS.md` "Docs to maintain" and "Release workflow" | `docs/`, `CHANGELOG.md`, `docs/release/` |
| PLAN-00004-REQ-08 | Root `AGENTS.md` "Non-negotiables"; PLAN-00001 § 5 | `justfile` `check`, `ci` |
| PLAN-00004-REQ-09 | User road map (2026-09-12): v0.1.x is the SSH-only CLI | — |

## 3. Repository baseline

| Field | Value |
|---|---|
| Repository | `joelee/passalong` (`git@github.com:joelee/passalong.git`) |
| Branch | `feature/00004-v0.1.3`, renamed from the local `fix/release-notes-links` at the user's request; not pushed |
| HEAD | `c44d14d4e243b8cf1f5c63a27e6a4cb279553123`: `main` at `8f58125` (merge of PR #3, tag `v0.1.2`) plus the issue #4 fix |
| Working tree at publication | Clean before allocation; the only change is this plan file |
| Release state | v0.1.2 released 2026-09-13: GitHub release with Linux and macOS binaries; `passalong`, `passalong-core`, `passalong-ssh` 0.1.2 on crates.io |
| Repository settings | `main` requires a PR and the Linux, macOS, and Xvfb checks; `v*` tags protected; `release` environment with a required reviewer holds `CARGO_REGISTRY_TOKEN`; Dependabot on |
| Review input | Code Review 00001: approve with comments; REV-00001-MED-01 (Medium) and REV-00001-LOW-01 (Low), both non-blocking |

## 4. Scope

### In scope

- Issue #4: verify and record the committed fix (absolute plan links in
  `docs/release/*.md`; `scripts/check-release-tag.sh` rejects relative
  links).
- `Store::get_meta` and targeted metadata reads in the choice prompt.
- Exclusive reservation of download names for `load` and pull mode.
- Per D-03, pull mode tracking seen item ids, with `Store::list_ids`.
- Documentation, backlog updates, and v0.1.3 release preparation.

### Out of scope

- The text of the published v0.1.1 and v0.1.2 GitHub releases, which keep
  their broken plan link; the user decided on 2026-09-13 not to change them.
- ssh-agent authentication, connection reuse, and the Android cross-compile
  CI check, which are larger than this release allows (D-04).
- GUI, Windows, Android, and any backend other than `ssh` and `local`.
- Tagging, publishing, and the release finalisation commit, which follow the
  root `AGENTS.md` "Release workflow" after the user approves the work.

## 5. Constraints and preserved decisions

- Every decision of PLAN-00001 to PLAN-00003 stays in force, including the
  item schema (version 1), download naming (D-02 of PLAN-00003), and pull
  mode's behaviour apart from how it finds new items.
- `passalong-core` and `passalong-ssh` are published crates. New `Store`
  methods get default implementations, so implementations outside this
  repository keep compiling within 0.1.x.
- Quality rules: TDD with a failing test first, mocks for external
  interfaces, `just check` green at every step, line coverage at least
  80 %, rustdoc for public items, no `unsafe`, no `println!` for logs.
- Library crates must not spawn processes.
- Release steps follow the root `AGENTS.md` "Release workflow": release
  notes use absolute links, and the Builder never tags or publishes.
- Builder works on `feature/00004-v0.1.3`, one commit per step, without
  review pauses unless blocked.

## 6. Assumptions

None. Unresolved matters are recorded as decisions.

## 7. Decisions and blockers

| ID | Decision or blocker | Resolution | Owner | Status |
|---|---|---|---|---|
| D-01 | How to read a single item's metadata. | **Resolved by planner:** add `Store::get_meta(&ItemId) -> Result<ItemMeta, StoreError>`, returning `StoreError::NotFound` for an unknown id. The default implementation calls `get` and drops the content; `FsStore` overrides it with its existing `read_meta`, one `meta.json` read. The choice prompt fetches metadata only for the candidates it shows, at most 9. | Planner | Resolved |
| D-02 | How to make download names atomic. | **Resolved by planner:** `free_target` becomes `reserve_target`: it creates the chosen file exclusively (`create_new`), empty, and moves to the next number when the name already exists, so two writers can never hold the same name. The verified part file is then renamed over the writer's own reservation. On any failure the part file and the reservation are both removed. An explicit `DEST` and `--force` keep their v0.1.2 behaviour. | Planner | Resolved |
| D-03 | Include the pull-mode clock-skew fix in v0.1.3. | **Confirmed by user (2026-09-13): include.** Pull mode remembers the ids it has seen instead of a position ordered by creation time. At start it records every stored id; each poll lists the ids with one directory read (`Store::list_ids`, default from `list_after(None)`, `FsStore` override from its directory listing) and handles the unseen ones from other devices: files oldest id first, and the newest text or clipboard image by id. Items from a device whose clock runs behind are then applied too, and the clock-sync caveat leaves the documentation. Ids no longer in the store are forgotten, so memory stays bounded. **Alternative:** defer it and keep v0.1.3 to the two review findings and issue #4. | User | Resolved |
| D-04 | Which other backlog items to include. | **Resolved by planner:** none. ssh-agent authentication needs an agent-based Docker test, connection reuse changes every command's lifecycle, and the Android CI check needs the Android NDK for `ring`; each is larger than this release. | Planner | Resolved |
| D-05 | Branch and version. | **Resolved by planner:** v0.1.3 on `feature/00004-v0.1.3`, which already carries the issue #4 fix (c44d14d). The PR that merges it should say "Fixes #4". | Planner | Resolved |

Blocking decisions: 0. The user confirmed D-03 as proposed (include) on
2026-09-13; the plan body already reflects it.

## 8. Affected architecture and components

| Area | Paths | Change |
|---|---|---|
| Release tooling | `scripts/check-release-tag.sh`, `crates/passalong-cli/tests/release_tag_script.rs`, `docs/release/*.md` | Already changed by c44d14d; verified in STEP-01 |
| Store | `crates/passalong-core/src/store/{mod,fs_store}.rs`, test stores | `get_meta`, `list_ids` with defaults and `FsStore` overrides |
| Choice prompt | `crates/passalong-cli/src/resolve.rs` | Candidate metadata through `get_meta` |
| Downloads | `crates/passalong-core/src/download.rs`, `crates/passalong-cli/src/commands/load.rs`, `crates/passalong-core/src/serve/pull.rs` | Exclusive name reservation |
| Pull mode | `crates/passalong-core/src/serve/{pull,mod}.rs` | Seen-id tracking (D-03) |
| Docs | `docs/usage.md`, `docs/architecture.md`, `docs/backlog.md`, `CHANGELOG.md`, `docs/release/v0.1.3.md` (new), `Cargo.toml` versions | Updated |

```rust
// passalong-core::store::Store (both with default implementations)
async fn get_meta(&self, id: &ItemId) -> Result<ItemMeta, StoreError>;
async fn list_ids(&self) -> Result<Vec<ItemId>, StoreError>; // newest first

// passalong-core::download
pub async fn reserve_target(dir: &Path, name: &str) -> Result<PathBuf, DownloadError>;
```

## 9. Requirement catalogue

### PLAN-00004-REQ-01 — Absolute links in release notes (issue #4)

- **Requirement:** Every `docs/release/*.md` links to its delivery plan with
  an absolute URL pinned to the release tag, and `scripts/check-release-tag.sh`
  fails on any relative link in the release notes, naming each one.
- **Rationale:** GitHub release pages resolve relative links against the
  release URL, which drops `docs/`.
- **Source:** User request; GitHub issue #4.
- **Acceptance evidence:** AC-01.

### PLAN-00004-REQ-02 — `Store::get_meta`

- **Requirement:** Per D-01, with a default implementation and an `FsStore`
  override that reads one `meta.json`; unknown ids give `NotFound`.
- **Rationale:** Needed for targeted reads (REQ-03).
- **Source:** REV-00001-MED-01.
- **Acceptance evidence:** AC-02.

### PLAN-00004-REQ-03 — Targeted metadata in the choice prompt

- **Requirement:** When a prefix is ambiguous, `resolve_item` reads metadata
  only for the candidates it shows, never calling `Store::list`.
- **Rationale:** A full-store read is one SFTP request per stored item.
- **Source:** REV-00001-MED-01.
- **Acceptance evidence:** AC-03.

### PLAN-00004-REQ-04 — Atomic download names

- **Requirement:** Per D-02, for downloads by `load` without `DEST` and by
  pull mode.
- **Rationale:** Two simultaneous downloads of the same name could pick the
  same file, and one would be lost.
- **Source:** REV-00001-LOW-01.
- **Acceptance evidence:** AC-04, AC-05.

### PLAN-00004-REQ-05 — Pull mode tracks seen ids

- **Requirement:** Per D-03: seen-id tracking with `Store::list_ids`; items
  present at start are never applied; own-device items are ignored; store
  errors leave unseen items for the next poll; each poll reads one directory
  listing plus the metadata of new items only.
- **Rationale:** Removes pull mode's dependence on device clocks.
- **Source:** `docs/backlog.md` "Pull mode and clock skew"; D-03.
- **Acceptance evidence:** AC-06, AC-07.

### PLAN-00004-REQ-06 — Documentation

- **Requirement:** `docs/architecture.md` (targeted reads, name reservation,
  seen ids), `docs/usage.md` (clock caveat removed under D-03),
  `docs/backlog.md` (delivered items removed), all passing the documentation
  consistency script.
- **Rationale:** Documentation rules.
- **Source:** Root `AGENTS.md` "Docs to maintain".
- **Acceptance evidence:** AC-08.

### PLAN-00004-REQ-07 — v0.1.3 release preparation

- **Requirement:** All crates and internal `=` requirements at `0.1.3`;
  `CHANGELOG.md` `Unreleased` lists the v0.1.3 changes;
  `docs/release/v0.1.3.md` drafted with absolute, tag-pinned links. The
  finalisation commit follows the release workflow after approval.
- **Rationale:** The user tags v0.1.3 after this plan.
- **Source:** Root `AGENTS.md` "Release workflow".
- **Acceptance evidence:** AC-09.

### PLAN-00004-REQ-08 — Quality gates

- **Requirement:** TDD evidence per step; `just check` green at every step;
  `just ci` green locally; branch CI green on Linux, macOS, and Xvfb; line
  coverage at least 80 %.
- **Rationale:** Repository quality rules.
- **Source:** Root `AGENTS.md`.
- **Acceptance evidence:** AC-10.

### PLAN-00004-REQ-09 — Road map guard

- **Requirement:** No GUI, Windows, Android, or new backend code;
  `BackendRegistry` kinds remain `local` and `ssh`.
- **Rationale:** v0.1.x is the SSH-only CLI.
- **Source:** User road map.
- **Acceptance evidence:** AC-11.

## 10. Delivery strategy

1. **Record the carried fix (STEP-01).** Evidence only; no code change.
2. **Store additions and targeted reads (STEP-02).** Small, self-contained.
3. **Download names (STEP-03).** Shared by `load` and pull mode, so it lands
   before the pull change.
4. **Pull seen ids (STEP-04).** Builds on `list_ids` and the reservation.
5. **Docs, release preparation, final gate (STEP-05, STEP-06).**

Each step ends with `just check` green and one commit. Docker-backed tests
run at STEP-02 and STEP-06.

## 11. Detailed implementation steps

### PLAN-00004-STEP-01 — Verify the issue #4 fix

- **Status placeholder:** `not-started`
- **Objective:** Implement REQ-01 (already committed; record the evidence).
- **Requirements:** `PLAN-00004-REQ-01`
- **Depends on:** None
- **Affected components:** None changed; evidence from
  `scripts/check-release-tag.sh`, `docs/release/*.md`
- **Preconditions:** Plan approved.
- **Test or evidence first:** `relative_links_in_the_release_notes_fail`
  (written first in c44d14d, failing before the check existed).
- **Implementation tasks:** Run the tag-check tests; confirm no relative
  links remain in `docs/release/*.md`; confirm the three pinned plan URLs
  answer with HTTP 200.
- **Documentation/configuration/operations:** None.
- **Verification:** `cargo test -p passalong --test release_tag_script`;
  `grep` for relative links; `curl` on the three URLs.
- **Completion criteria:** AC-01.
- **Rollback or recovery:** Not applicable.
- **Builder stop conditions:** A test fails.

### PLAN-00004-STEP-02 — `Store::get_meta` and targeted candidate metadata

- **Status placeholder:** `not-started`
- **Objective:** Implement REQ-02 and REQ-03.
- **Requirements:** `PLAN-00004-REQ-02`, `PLAN-00004-REQ-03`
- **Depends on:** `PLAN-00004-STEP-01`
- **Affected components:** `crates/passalong-core/src/store/{mod,fs_store}.rs`,
  `crates/passalong-core/src/serve/upload.rs` (test store),
  `crates/passalong-cli/src/resolve.rs`, `crates/passalong-ssh/tests/sftp_docker.rs`
- **Preconditions:** None.
- **Test or evidence first:** `FsStore` tests with the `FaultyFs` counter:
  `get_meta` reads one `meta.json`, unknown ids give `NotFound`, and the
  default implementation matches `get`. A choice-prompt test with 20 items,
  11 of them matching, counts exactly 9 metadata reads.
- **Implementation tasks:**
  1. Trait method with a default; `FsStore` override.
  2. `resolve_item` uses `get_meta` for the shown candidates.
  3. SFTP Docker test for `get_meta`.
- **Documentation/configuration/operations:** Architecture: storage.
- **Verification:** `cargo test -p passalong-core store`;
  `cargo test -p passalong resolve`; `just test-integration`; `just check`.
- **Completion criteria:** AC-02, AC-03.
- **Rollback or recovery:** Revert.
- **Builder stop conditions:** None expected.

### PLAN-00004-STEP-03 — Atomic download names

- **Status placeholder:** `not-started`
- **Objective:** Implement REQ-04.
- **Requirements:** `PLAN-00004-REQ-04`
- **Depends on:** `PLAN-00004-STEP-02`
- **Affected components:** `crates/passalong-core/src/download.rs`,
  `crates/passalong-cli/src/commands/load.rs`,
  `crates/passalong-core/src/serve/pull.rs`
- **Preconditions:** None.
- **Test or evidence first:** Two reservations of the same name in one
  directory get `name` and `name (1)`; two downloads of same-named items run
  together with `tokio::join!` produce two distinct, intact files; a failed
  verification removes both the part file and the reservation.
- **Implementation tasks:**
  1. `reserve_target` replaces `free_target`, creating the file exclusively.
  2. `write_verified` renames over the caller's reservation and cleans up
     both files on failure.
  3. `load` and pull mode use `reserve_target`; explicit `DEST` unchanged.
- **Documentation/configuration/operations:** Architecture: downloads.
- **Verification:** `cargo test -p passalong-core download`;
  `cargo test -p passalong load`; `just check`.
- **Completion criteria:** AC-04, AC-05.
- **Rollback or recovery:** Revert.
- **Builder stop conditions:** Exclusive creation proves unreliable on a
  supported platform.

### PLAN-00004-STEP-04 — Pull mode tracks seen ids

- **Status placeholder:** `not-started`
- **Objective:** Implement REQ-05 per D-03.
- **Requirements:** `PLAN-00004-REQ-05`
- **Depends on:** `PLAN-00004-STEP-03`
- **Affected components:** `crates/passalong-core/src/store/{mod,fs_store}.rs`
  (`list_ids`), `crates/passalong-core/src/serve/pull.rs`,
  `crates/passalong-core/tests/serve_local.rs`
- **Preconditions:** D-03 resolved as "include"; otherwise this step is
  skipped with the user's approval recorded in the work log.
- **Test or evidence first:** `Puller` tests: an item whose id is older than
  one already handled (a device with a slow clock) is applied once; items
  present at start are never applied; a store error leaves unseen items for
  the next poll; a poll reads one directory listing and only the new items'
  metadata (`FaultyFs` counters); forgotten ids do not accumulate.
- **Implementation tasks:**
  1. `Store::list_ids` with a default and an `FsStore` override.
  2. `Puller` keeps a set of seen ids in place of its position.
  3. Remove `Store::newest_id` if nothing else uses it, keeping the trait's
     default so external callers still compile (deprecate rather than
     delete).
- **Documentation/configuration/operations:** Usage: remove the clock-sync
  caveat; architecture: pull loop.
- **Verification:** `cargo test -p passalong-core pull`;
  `cargo test -p passalong-core --all-features --test serve_local`;
  `cargo test -p passalong --test cli_local_backend serve_daemon_pulls`;
  `just check`.
- **Completion criteria:** AC-06, AC-07.
- **Rollback or recovery:** Revert; pull mode returns to v0.1.2 behaviour.
- **Builder stop conditions:** D-03 is deferred.

### PLAN-00004-STEP-05 — Documentation and v0.1.3 release preparation

- **Status placeholder:** `not-started`
- **Objective:** Implement REQ-06 and REQ-07.
- **Requirements:** `PLAN-00004-REQ-06`, `PLAN-00004-REQ-07`
- **Depends on:** `PLAN-00004-STEP-04`
- **Affected components:** `docs/`, `CHANGELOG.md`, `Cargo.toml`,
  `Cargo.lock`, version-pinning tests, `docs/release/v0.1.3.md` (new)
- **Preconditions:** None.
- **Test or evidence first:** Documentation step; evidence is the
  consistency script and `passalong --version`.
- **Implementation tasks:** Update docs; remove the three delivered backlog
  items; bump versions to 0.1.3; draft the release notes with absolute,
  tag-pinned links.
- **Documentation/configuration/operations:** This step.
- **Verification:** Consistency script; `just check`;
  `just publish-dry-run`.
- **Completion criteria:** AC-08, AC-09.
- **Rollback or recovery:** Revert.
- **Builder stop conditions:** Documented behaviour disagrees with code.

### PLAN-00004-STEP-06 — Final quality gate

- **Status placeholder:** `not-started`
- **Objective:** Prove the plan.
- **Requirements:** `PLAN-00004-REQ-08`, `PLAN-00004-REQ-09`
- **Depends on:** `PLAN-00004-STEP-05`
- **Affected components:** None new.
- **Test or evidence first:** Verification-only step.
- **Implementation tasks:** Run `just ci`; push `feature/00004-v0.1.3` and
  record the branch CI result for Linux, macOS, and Xvfb; review the diff
  for road-map scope; record coverage.
- **Documentation/configuration/operations:** None.
- **Verification:** `just ci`; GitHub CI; `git diff --stat` scope review.
- **Completion criteria:** AC-10, AC-11.
- **Rollback or recovery:** Not applicable.
- **Builder stop conditions:** Any gate fails in a way that needs scope or
  threshold changes.

## 12. Cross-cutting concerns

| Area | Applicability | Planned action or reason not applicable | Step or requirement |
|---|---|---|---|
| Compatibility and APIs | Applicable | New `Store` methods have default implementations, so external implementations keep compiling; schema unchanged; no config change | REQ-02, REQ-05 |
| Data and migration | Not applicable | No storage or config format change | — |
| Security and privacy | Applicable | Exclusive reservation keeps the "never overwrite" promise under concurrency; release notes cannot link wrongly | REQ-01, REQ-04 |
| Performance and scale | Applicable | Candidate metadata only; pull polls read one directory listing plus new metadata | REQ-03, REQ-05 |
| Reliability and failure handling | Applicable | Reservations and part files removed on failure; unseen items retried after store errors | REQ-04, REQ-05 |
| Observability and operations | Applicable | Pull logs each applied or skipped id as before | REQ-05 |
| Dependencies and supply chain | Not applicable | No new dependencies | — |
| Accessibility and UX | Applicable | Faster choice prompt on large stores; no clock caveat for pull mode | REQ-03, REQ-05 |
| Documentation and release | Applicable | Architecture, usage, backlog, release notes with absolute links | REQ-06, REQ-07 |
| Deployment and rollback | Applicable | Released through the tag workflow with the `release` environment approval | REQ-07 |

## 13. Verification strategy

| Level | Evidence or command | When | Required result |
|---|---|---|---|
| Unit | `cargo test -p <crate> <module>` | Every step, red then green | Pass |
| Workspace gate | `just check` | Every step | Exit 0 |
| SSH integration | `just test-integration` | STEP-02, 06 | Exit 0 |
| Release check | `cargo test -p passalong --test release_tag_script` | STEP-01, 06 | Pass |
| Packaging | `just publish-dry-run` | STEP-05, 06 | Exit 0 |
| Docs | consistency script | STEP-05 | No gaps |
| Full | `just ci` locally and GitHub CI | STEP-06 | Pass on Linux, macOS, and Xvfb |

## 14. Acceptance criteria

- [ ] `PLAN-00004-AC-01` `docs/release/*.md` contain no relative links; `relative_links_in_the_release_notes_fail` passes; the three pinned plan URLs answer with HTTP 200.
- [ ] `PLAN-00004-AC-02` `get_meta` returns an item's metadata with exactly one `meta.json` read, gives `NotFound` for an unknown id, and the default implementation agrees with `get`; the SFTP Docker test passes.
- [ ] `PLAN-00004-AC-03` With 20 stored items and a prefix matching 11, the choice prompt performs exactly 9 metadata reads and never calls `Store::list`.
- [ ] `PLAN-00004-AC-04` Two downloads of same-named items into one directory, run together, produce `name` and `name (1)`, both intact.
- [ ] `PLAN-00004-AC-05` A failed verified write leaves neither a part file nor a reservation; explicit `DEST` and `--force` behave as in v0.1.2.
- [ ] `PLAN-00004-AC-06` Under D-03, an item from another device whose id is older than one already handled is applied once; items present at start are never applied; a store error leaves unseen items for the next poll.
- [ ] `PLAN-00004-AC-07` Under D-03, a pull poll performs one directory listing and reads only the new items' metadata, and ids no longer stored are forgotten.
- [ ] `PLAN-00004-AC-08` The architecture and usage docs describe the new behaviour, the three delivered backlog items are removed, and the consistency script reports no gaps.
- [ ] `PLAN-00004-AC-09` All crates are at `0.1.3`; `passalong --version` prints `passalong 0.1.3`; `CHANGELOG.md` `Unreleased` lists the v0.1.3 changes; `docs/release/v0.1.3.md` exists with only absolute links.
- [ ] `PLAN-00004-AC-10` `just ci` passes locally and GitHub CI passes on Linux, macOS, and Xvfb for the final commit; line coverage is at least 80 %.
- [ ] `PLAN-00004-AC-11` The diff adds no GUI, Windows, Android, or backend code; `BackendRegistry` kinds are still `local` and `ssh`.

## 15. Risks and mitigations

| Risk | Likelihood | Impact | Mitigation or test | Owner/step |
|---|---|---|---|---|
| A default `Store` method is slow for external implementations (`get_meta` via `get`) | Low | Low | Documented; `FsStore` overrides it | STEP-02 |
| Reservation files are left behind after a crash | Low | Low | Empty reservations are visible and harmless; failures inside the process clean up | STEP-03 |
| Exclusive creation behaves differently on network filesystems | Low | Medium | Uses the standard `create_new` open; NFSv3 and later honour exclusive creates | STEP-03 |
| The seen-id set grows with the store | Low | Low | Ids no longer stored are forgotten each poll; ids are about 21 bytes | STEP-04 |
| Changing pull mode breaks v0.1.2 behaviour | Medium | Medium | Existing Puller, `serve_local`, and binary pull tests must keep passing | STEP-04 |

## 16. Builder hand-off

- **Start condition:** User approval and a clean repository.
- **First step:** `PLAN-00004-STEP-01` — Verify the issue #4 fix.
- **Required sequence:** STEP-01 → 02 → 03 → 04 → 05 → 06, one commit per
  step without review pauses unless blocked.
- **Parallel-safe work:** None (single Builder).
- **Do not change:** approved scope, requirements, steps, acceptance
  criteria, or content outside Builder's permitted work-log area.
- **Escalate when:** a Builder stop condition triggers; a decision in § 7
  turns out unworkable.
- **Completion hand-off:** Work log with evidence, local and GitHub CI
  results, coverage, and readiness for the release workflow's approval step.

<!-- BUILDER_WORK_LOG_START -->
## 17. Builder Work Log

> [!warning] Builder-maintained section
> Delivery Planner creates this section. After approval, Builder may update only
> this delimited section and the Builder-maintained front-matter fields. Builder
> must preserve prior entries and use UTC timestamps.

### Step status

| Step | Status | Started (UTC) | Completed (UTC) | Evidence | Builder notes |
|---|---|---|---|---|---|
| PLAN-00004-STEP-01 | completed | 2026-09-13T11:21:15Z | 2026-09-13T11:21:36Z | Commit `build: complete PLAN-00004-STEP-01 - Verify the issue #4 fix`; `just check` green, 92.19% lines | AC-01. Evidence-only step: the fix is c44d14d on the baseline (absolute tag-pinned plan links in docs/release/v0.1.0, v0.1.1, v0.1.2; check-release-tag.sh rejects relative links; AGENTS.md and the developer guide state the rule). The PR merging this branch should say Fixes #4 |
| PLAN-00004-STEP-02 | completed | 2026-09-13T11:21:36Z | 2026-09-13T11:24:33Z | Commit `build: complete PLAN-00004-STEP-02 - Store::get_meta and targeted candidate metadata`; `just check` green, 92.09% lines | AC-02, AC-03. resolve_item no longer calls Store::list; a candidate deleted or damaged since listing shows a ? row, and other store errors are returned. Architecture documents get_meta |
| PLAN-00004-STEP-03 | completed | 2026-09-13T11:24:33Z | 2026-09-13T11:27:20Z | Commit `build: complete PLAN-00004-STEP-03 - Atomic download names`; `just check` green, 91.91% lines | AC-04, AC-05. The load regression test passed before the change too (the old code never created the target early) and guards that a reservation is removed after a failed write. CHANGELOG Fixed entries added for STEP-02 and STEP-03 |
| PLAN-00004-STEP-04 | completed | 2026-09-13T11:27:20Z | 2026-09-13T11:31:27Z | Commit `build: complete PLAN-00004-STEP-04 - Pull mode tracks seen ids`; `just check` green, 92.01% lines | AC-06, AC-07. Puller keeps a HashSet of seen ids (all stored ids at start); each poll lists ids once, forgets ids no longer stored, reads get_meta only for unseen ids, and marks each item seen once handled, so a store error leaves the rest unseen. Usage drops the clock-sync caveat; architecture describes list_ids and seen ids; CHANGELOG Fixed entry |
| PLAN-00004-STEP-05 | completed | 2026-09-13T11:31:27Z | 2026-09-13T11:34:29Z | Commit `build: complete PLAN-00004-STEP-05 - Documentation and v0.1.3 release preparation`; `just check` green, 92.00% lines | AC-08, AC-09. docs/release/v0.1.3.md drafted with absolute links only (plan link pinned to v0.1.3; issue #4 and docs/plans links absolute); CHANGELOG Unreleased has Added (get_meta, list_ids), Deprecated (newest_id, free_target), and Fixed (pull clock skew, targeted reads, atomic names, release-note links); backlog drops pull clock skew, targeted metadata reads, atomic download names. Architecture and usage were updated in STEP-02 to STEP-04 |
| PLAN-00004-STEP-06 | completed | 2026-09-13T11:34:29Z | 2026-09-13T11:53:56Z | Commit `build: complete PLAN-00004-STEP-06 - Final quality gate`; `just check` green, 91.83% lines | AC-10 by local just ci and GitHub CI (coverage 91.82 % without Docker, 94.18 % with; release notes updated to these figures); AC-11 by the scope review. This commit changes only the plan and the release notes' coverage figures, and its own CI run is checked after the push |

Allowed status values: `not-started`, `in-progress`, `blocked`, `completed`,
`skipped`. A skipped step requires explicit user approval recorded in Evidence.

### Execution log

| Timestamp (UTC) | Step | Event | Evidence or reference | Next action |
|---|---|---|---|---|
| 2026-09-13T11:21:15Z | PLAN-00004 | Plan approved (commit 1f3ec27); Builder starts on feature/00004-v0.1.3 | `docs(plan): approve PLAN-00004 - V0 1 3 Review Fixes And Pull Ids` | Begin PLAN-00004-STEP-01 |
| 2026-09-13T11:21:15Z | PLAN-00004-STEP-01 | Started | — | Red phase |
| 2026-09-13T11:21:36Z | PLAN-00004-STEP-01 | Verified and committed (continuous execution authorised by the user) | `build: complete PLAN-00004-STEP-01 - Verify the issue #4 fix` | Begin PLAN-00004-STEP-02 |
| 2026-09-13T11:21:36Z | PLAN-00004-STEP-02 | Started | — | Red phase |
| 2026-09-13T11:24:33Z | PLAN-00004-STEP-02 | Verified and committed (continuous execution authorised by the user) | `build: complete PLAN-00004-STEP-02 - Store::get_meta and targeted candidate metadata` | Begin PLAN-00004-STEP-03 |
| 2026-09-13T11:24:33Z | PLAN-00004-STEP-03 | Started | — | Red phase |
| 2026-09-13T11:27:20Z | PLAN-00004-STEP-03 | Verified and committed (continuous execution authorised by the user) | `build: complete PLAN-00004-STEP-03 - Atomic download names` | Begin PLAN-00004-STEP-04 |
| 2026-09-13T11:27:20Z | PLAN-00004-STEP-04 | Started | — | Red phase |
| 2026-09-13T11:31:27Z | PLAN-00004-STEP-04 | Verified and committed (continuous execution authorised by the user) | `build: complete PLAN-00004-STEP-04 - Pull mode tracks seen ids` | Begin PLAN-00004-STEP-05 |
| 2026-09-13T11:31:27Z | PLAN-00004-STEP-05 | Started | — | Red phase |
| 2026-09-13T11:34:29Z | PLAN-00004-STEP-05 | Verified and committed (continuous execution authorised by the user) | `build: complete PLAN-00004-STEP-05 - Documentation and v0.1.3 release preparation` | Begin PLAN-00004-STEP-06 |
| 2026-09-13T11:34:29Z | PLAN-00004-STEP-06 | Started | — | Evidence first |
| 2026-09-13T11:53:56Z | PLAN-00004-STEP-06 | Verified and committed (continuous execution authorised by the user) | `build: complete PLAN-00004-STEP-06 - Final quality gate` | Builder hand-off |

### Deviations and blockers

| Timestamp (UTC) | Step | Deviation or blocker | Impact | Decision required from |
|---|---|---|---|---|
| 2026-09-13T11:27:20Z | PLAN-00004-STEP-03 | `download::free_target` stays as a deprecated function instead of being removed, because passalong-core 0.1.2 is published with it; nothing in the workspace calls it. Pull mode now fetches the item before reserving its name, so a store error cannot leave an empty reservation. STEP-02's CHANGELOG entry was added in this step. | None | None (routine) |
| 2026-09-13T11:31:27Z | PLAN-00004-STEP-04 | `Store::newest_id` is deprecated rather than removed (published API); pull mode no longer calls it. An unseen item that cannot be read because it was deleted or is corrupt is logged and marked seen, like list's handling of corrupt items, so it is not retried every poll. | None | None (routine) |
| 2026-09-13T11:53:56Z | PLAN-00004-STEP-06 | D-02's mechanism changed after approval, by the user's decision (option A, 2026-09-13): instead of creating the final name empty and renaming the verified part over it, `download::download_into` writes and verifies an exclusively created part file (<name>.<random>.passalong-part) and hard-links it to the first free name, falling back to rename where hard links are unsupported. The reservation made a download visible, empty, under its final name until it finished, which CI run 34754834734 caught. download_into replaces the unreleased reserve_target and write_reserved (commit 7ef1dbe). D-02's goals are kept: never overwrite, never two downloads on one name. | Downloads again appear only when complete | None (routine) |
| 2026-09-13T16:06:02Z | PLAN-00004-STEP-06 | Post-gate fix requested by the user after a live test in which images copied in Chromium were stored as text: Chromium offers no text/plain for a copied image, so the Wayland text read fell back to `text/x-moz-url` (UTF-16 link and title) and that non-blank text hid the image. `clipboard::read_payload`, shared by `passalong clipboard` and `serve`, now drops text containing NUL characters and lets the image win over text that only refers to it (a lone link, optionally with a title line, or an `<img>` tag); real text still wins. Commit `fix(clipboard): send the copied image, not the link a browser adds`. | Browser image copies are sent as images | User |

None.

### Verification results

| Timestamp (UTC) | Step | Command or check | Result | Evidence |
|---|---|---|---|---|
| 2026-09-13T11:21:15Z | PLAN-00004-STEP-01 | Red (recorded in c44d14d, before the check existed): `cargo test -p passalong --test release_tag_script` | Exit 101 (expected) | relative_links_in_the_release_notes_fail FAILED; 8 other tag-check tests passed |
| 2026-09-13T11:21:36Z | PLAN-00004-STEP-01 | `cargo test -p passalong --test release_tag_script` | Exit 0 | 9 passed, including relative_links_in_the_release_notes_fail (relative link rejected and named; absolute and #anchor links pass) |
| 2026-09-13T11:21:36Z | PLAN-00004-STEP-01 | `cargo test -p passalong --test release_tag_script` | Pass | test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s |
| 2026-09-13T11:21:36Z | PLAN-00004-STEP-01 | `/tmp/claude-1000/-home-joel-Projects-GitHub-passalong/cb409cac-8fd2-4e7d-be1b-e82764887e17/scratchpad/check_release_links.sh` | Pass | no relative links in docs/release: verified |
| 2026-09-13T11:21:36Z | PLAN-00004-STEP-01 | `/tmp/claude-1000/-home-joel-Projects-GitHub-passalong/cb409cac-8fd2-4e7d-be1b-e82764887e17/scratchpad/check_plan_urls.sh` | Pass | 3 tag-pinned plan URLs answer 200: verified |
| 2026-09-13T11:21:36Z | PLAN-00004-STEP-01 | `scripts/check-release-tag.sh v0.1.2` | Pass | tag v0.1.2 matches the workspace version and the release records are final |
| 2026-09-13T11:21:36Z | PLAN-00004-STEP-01 | `/tmp/claude-1000/-home-joel-Projects-GitHub-passalong/cb409cac-8fd2-4e7d-be1b-e82764887e17/scratchpad/doccheck2.sh` | Pass | documentation consistent: config keys, variables, flags, recipes, links, release documents, CHANGELOG |
| 2026-09-13T11:21:36Z | PLAN-00004-STEP-01 | `just check` | Exit 0 | Lines 92.19% (8927 lines, 697 missed) |
| 2026-09-13T11:21:36Z | PLAN-00004-STEP-02 | Red: `cargo test -p passalong-core --all-features --lib store` and `cargo test -p passalong --bin passalong resolve` | Exit 101 (expected) | core: 4 x E0599 no method get_meta; CLI: the_prompt_reads_only_the_candidates_it_shows read 20 meta.json files (left 20, right 9) |
| 2026-09-13T11:24:33Z | PLAN-00004-STEP-02 | `cargo test -p passalong-core --all-features --lib store` | Exit 0 | get_meta reads exactly one meta.json and gives NotFound for an unknown id; the default get_meta (DefaultMeta wrapper) agrees with get |
| 2026-09-13T11:24:33Z | PLAN-00004-STEP-02 | `cargo test -p passalong` | Exit 0 | the_prompt_reads_only_the_candidates_it_shows: 20 stored, 11 matching, exactly 9 metadata reads; existing resolve and delete tests pass |
| 2026-09-13T11:24:33Z | PLAN-00004-STEP-02 | `just test-integration` (Docker) | Exit 0 | get_meta_works_over_sftp passed with the other ignored SSH tests |
| 2026-09-13T11:24:33Z | PLAN-00004-STEP-02 | `cargo test -p passalong-core --all-features --lib store` | Pass | test result: ok. 31 passed; 0 failed; 0 ignored; 0 measured; 141 filtered out; finished in 0.17s |
| 2026-09-13T11:24:33Z | PLAN-00004-STEP-02 | `cargo test -p passalong --bin passalong resolve` | Pass | test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 85 filtered out; finished in 0.01s |
| 2026-09-13T11:24:33Z | PLAN-00004-STEP-02 | `/tmp/claude-1000/-home-joel-Projects-GitHub-passalong/cb409cac-8fd2-4e7d-be1b-e82764887e17/scratchpad/doccheck2.sh` | Pass | documentation consistent: config keys, variables, flags, recipes, links, release documents, CHANGELOG |
| 2026-09-13T11:24:33Z | PLAN-00004-STEP-02 | `just check` | Exit 0 | Lines 92.09% (9023 lines, 714 missed); fs_store.rs 96.76%; resolve.rs 95.55% |
| 2026-09-13T11:24:33Z | PLAN-00004-STEP-03 | Red: `cargo test -p passalong-core --all-features --lib download` with the reservation tests | Exit 101 (expected) | 9 x E0425: reserve_target and write_reserved not found |
| 2026-09-13T11:27:20Z | PLAN-00004-STEP-03 | `cargo test -p passalong-core --all-features --lib` | Exit 0 | reservations_never_share_a_name (a.txt, a (1).txt, both held); simultaneous_downloads_of_the_same_name_keep_both_files (tokio::join, both contents intact); a_failed_write_into_a_reservation_leaves_nothing_behind; numbering tests now use reserve_target; pull tests pass |
| 2026-09-13T11:27:20Z | PLAN-00004-STEP-03 | `cargo test -p passalong` | Exit 0 | existing download, --force, and explicit-DEST tests pass; a_damaged_download_leaves_nothing_in_the_download_directory |
| 2026-09-13T11:27:20Z | PLAN-00004-STEP-03 | `cargo test -p passalong-core --all-features --lib download` | Pass | test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 163 filtered out; finished in 0.02s |
| 2026-09-13T11:27:20Z | PLAN-00004-STEP-03 | `cargo test -p passalong --bin passalong load` | Pass | test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 78 filtered out; finished in 0.02s |
| 2026-09-13T11:27:20Z | PLAN-00004-STEP-03 | `cargo test -p passalong-core --all-features --lib pull` | Pass | test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 164 filtered out; finished in 0.01s |
| 2026-09-13T11:27:20Z | PLAN-00004-STEP-03 | `/tmp/claude-1000/-home-joel-Projects-GitHub-passalong/cb409cac-8fd2-4e7d-be1b-e82764887e17/scratchpad/doccheck2.sh` | Pass | documentation consistent: config keys, variables, flags, recipes, links, release documents, CHANGELOG |
| 2026-09-13T11:27:20Z | PLAN-00004-STEP-03 | `just check` | Exit 0 | Lines 91.91% (9140 lines, 739 missed); download.rs 88.45%; load.rs 98.76%; pull.rs 87.41% |
| 2026-09-13T11:27:20Z | PLAN-00004-STEP-04 | Red: `cargo test -p passalong-core --all-features --lib` with the list_ids and seen-id Puller tests | Exit 101 (expected) | 4 x E0599: no method list_ids (store and default-wrapper tests) and no method seen_len (Puller) |
| 2026-09-13T11:31:27Z | PLAN-00004-STEP-04 | `cargo test -p passalong-core --all-features` | Exit 0 | list_ids reads one directory and no metadata; the default list_ids agrees with list; an_item_from_a_device_with_a_slow_clock_is_applied_once (id an hour older than the one already applied); a_poll_reads_one_listing_and_only_the_new_metadata (1 ReadDir, 3 OpenRead for 3 new items beside 5 old); ids_that_leave_the_store_are_forgotten; every earlier Puller and serve_local test still passes |
| 2026-09-13T11:31:27Z | PLAN-00004-STEP-04 | `cargo test -p passalong --test cli_local_backend serve_daemon_pulls` | Exit 0 | the daemon still pulls a file sent by a second device |
| 2026-09-13T11:31:27Z | PLAN-00004-STEP-04 | `cargo test -p passalong-core --all-features --lib pull` | Pass | test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 165 filtered out; finished in 0.01s |
| 2026-09-13T11:31:27Z | PLAN-00004-STEP-04 | `cargo test -p passalong-core --all-features --test serve_local` | Pass | test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.61s |
| 2026-09-13T11:31:27Z | PLAN-00004-STEP-04 | `cargo test -p passalong --test cli_local_backend serve_daemon_pulls` | Pass | test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 25 filtered out; finished in 1.13s |
| 2026-09-13T11:31:27Z | PLAN-00004-STEP-04 | `/tmp/claude-1000/-home-joel-Projects-GitHub-passalong/cb409cac-8fd2-4e7d-be1b-e82764887e17/scratchpad/doccheck2.sh` | Pass | documentation consistent: config keys, variables, flags, recipes, links, release documents, CHANGELOG |
| 2026-09-13T11:31:27Z | PLAN-00004-STEP-04 | `just check` | Exit 0 | Lines 92.01% (9253 lines, 739 missed); pull.rs 89.16%; fs_store.rs 97.37% |
| 2026-09-13T11:31:27Z | PLAN-00004-STEP-05 | Before: version and records | 0.1.2 (expected) | workspace at 0.1.2; no docs/release/v0.1.3.md; backlog still listed the three items delivered by STEP-02 to STEP-04 |
| 2026-09-13T11:34:29Z | PLAN-00004-STEP-05 | `cargo run -q -p passalong -- --version` | Exit 0 | passalong 0.1.3; Cargo.lock updated for the three workspace crates only |
| 2026-09-13T11:34:29Z | PLAN-00004-STEP-05 | `just coverage-full` (before the bump, for the release notes) | Exit 0 | 94.36 % lines; 14 Docker tests passed; desktop tests filtered (they run under Xvfb in CI) |
| 2026-09-13T11:34:29Z | PLAN-00004-STEP-05 | `just publish-dry-run --allow-dirty` | Exit 0 | 3 crates packaged and verified at 0.1.3 |
| 2026-09-13T11:34:29Z | PLAN-00004-STEP-05 | `cargo run -q -p passalong -- --version` | Pass | passalong 0.1.3 |
| 2026-09-13T11:34:29Z | PLAN-00004-STEP-05 | `/tmp/claude-1000/-home-joel-Projects-GitHub-passalong/cb409cac-8fd2-4e7d-be1b-e82764887e17/scratchpad/check_release_links.sh` | Pass | no relative links in docs/release: verified |
| 2026-09-13T11:34:29Z | PLAN-00004-STEP-05 | `/tmp/claude-1000/-home-joel-Projects-GitHub-passalong/cb409cac-8fd2-4e7d-be1b-e82764887e17/scratchpad/doccheck2.sh` | Pass | documentation consistent: config keys, variables, flags, recipes, links, release documents, CHANGELOG |
| 2026-09-13T11:34:29Z | PLAN-00004-STEP-05 | `just check` | Exit 0 | Lines 92.00% (9253 lines, 740 missed) |
| 2026-09-13T11:53:56Z | PLAN-00004-STEP-06 | `just ci` locally on e02b917 (first run) | Exit 0 | check 92.01 %; audit ok; publish dry run 3 crates; actionlint; 14 Docker SSH tests; deploy example; coverage-full 94.36 % |
| 2026-09-13T11:53:56Z | PLAN-00004-STEP-06 | GitHub CI run 34754834734 on e02b917 | Linux fail; macOS and Xvfb pass | serve_daemon_pulls_files_sent_by_another_device read the pulled file as empty under coverage: the D-02 reservation made the final name visible, empty, before the content was written. Fixed in 7ef1dbe (option A, chosen by the user) |
| 2026-09-13T11:53:56Z | PLAN-00004-STEP-06 | `just ci` locally on 7ef1dbe (after the fix) | Exit 0 | check 91.82 % lines; audit advisories/bans/licenses/sources ok; publish dry run verified 3 crates; actionlint no findings; 14 Docker SSH tests; deploy example host key unchanged after re-creation; coverage-full 94.18 % lines; daemon pull test passed 5 runs in a row; no passalong processes or containers left |
| 2026-09-13T11:53:56Z | PLAN-00004-STEP-06 | GitHub CI run 34755410015 on 7ef1dbe (after the fix) | Pass | Linux (just ci with Docker SSH tests, including the daemon pull test under coverage), macOS (just check), Linux desktop clipboard (Xvfb): all success |
| 2026-09-13T11:53:56Z | PLAN-00004-STEP-06 | Scope review: `git diff --stat origin/main..HEAD -- crates` | Pass | new crate-level files none; registry kinds local and ssh only; no GUI, Windows, Android, or backend code |
| 2026-09-13T11:53:56Z | PLAN-00004-STEP-06 | `/tmp/claude-1000/-home-joel-Projects-GitHub-passalong/cb409cac-8fd2-4e7d-be1b-e82764887e17/scratchpad/check_release_links.sh` | Pass | no relative links in docs/release: verified |
| 2026-09-13T11:53:56Z | PLAN-00004-STEP-06 | `/tmp/claude-1000/-home-joel-Projects-GitHub-passalong/cb409cac-8fd2-4e7d-be1b-e82764887e17/scratchpad/doccheck2.sh` | Pass | documentation consistent: config keys, variables, flags, recipes, links, release documents, CHANGELOG |
| 2026-09-13T11:53:56Z | PLAN-00004-STEP-06 | `cargo test -p passalong --test release_tag_script` | Pass | test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s |
| 2026-09-13T11:53:56Z | PLAN-00004-STEP-06 | `just check` | Exit 0 | Lines 91.83% (9306 lines, 760 missed) |

### Completion summary

- **Implementation status:** `not-started`
- **Completed requirements:** REQ-01 to REQ-09
- **Incomplete requirements:** None
- **Outstanding blockers:** None
- **Review request:** Not ready
<!-- BUILDER_WORK_LOG_END -->

## 18. Planning change log

| Timestamp (UTC) | Plan status | Change | Reason | Requested/approved by |
|---|---|---|---|---|
| 2026-09-13T11:01:43Z | draft | Created with 9 requirements, 6 steps, 11 acceptance criteria, and decisions D-01 to D-05 (D-03 proposed and blocking) | User request for a small v0.1.3 plan | User |
| 2026-09-13T11:21:15Z | draft | D-03 confirmed as proposed (include); blocking decisions 0 | User answer: "Proceed as recommended" | User |
| 2026-09-13T11:21:15Z | approved | Approved; `plan_status`, `approved_at`, and `build_ready` set | User approval after committing the draft (1671bca) | User |

## 19. External references

- GitHub issue #4, "Incorrect links to Delivery Plans on Release Notes",
  joelee/passalong, opened 2026-09-13; read 2026-09-13;
  <https://github.com/joelee/passalong/issues/4>.

## 20. Confidence

**Medium.** The repository, the review findings, and the affected code were
read directly, and the issue #4 fix is already tested on the baseline. The
main uncertainty is the pull-mode change under D-03, which replaces a
tested mechanism, and exclusive-create behaviour on network filesystems.
