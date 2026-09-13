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
implementation_status: not-started # not-started | in-progress | blocked | completed | abandoned
builder_agent: null
builder_model: null
execution_branch: null
execution_started_at: null
execution_updated_at: null
execution_completed_at: null
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
| PLAN-00004-STEP-01 | not-started | — | — | — | — |
| PLAN-00004-STEP-02 | not-started | — | — | — | — |
| PLAN-00004-STEP-03 | not-started | — | — | — | — |
| PLAN-00004-STEP-04 | not-started | — | — | — | — |
| PLAN-00004-STEP-05 | not-started | — | — | — | — |
| PLAN-00004-STEP-06 | not-started | — | — | — | — |

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
