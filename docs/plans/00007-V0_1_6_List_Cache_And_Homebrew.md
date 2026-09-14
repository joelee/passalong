---
title: "Delivery Plan 00007: V0 1 6 List Cache And Homebrew"
aliases:
  - "Plan 00007"
tags:
  - delivery-plan
  - implementation
  - opencode
type: delivery-plan
plan_id: "PLAN-00007"
plan_status: approved              # draft | approved | cancelled
plan_kind: initial                 # initial | superseding
created_at: "2026-09-14T17:12:31Z"
approved_at: "2026-09-14T18:18:32Z"
planner_agent: "Claude Code"
planner_model: "anthropic/claude-opus-5"
triggered_by: user                 # user | agent:<agent-name>
request_kind: direct               # idea | review | idea-and-review | direct | unplanned-query
repository: "joelee/passalong"
baseline_branch: "feature/00007-v0.1.6"
baseline_commit: "d5d4e2283bdb6158406287c5f9f7b33b7b841e5a"
source_ideas: []
source_reviews: []
previous_plan: null
requirements_count: 13
steps_count: 9
acceptance_criteria_count: 15
blocking_decisions: 0
build_ready: true
web_research_used: false
confidence: medium                # high | medium | low

# Builder-maintained front matter. Builder may update only these keys after
# explicit user approval; Delivery Planner initializes them.
implementation_status: in-progress # not-started | in-progress | blocked | completed | abandoned
builder_agent: "Claude Code"
builder_model: "anthropic/claude-opus-5"
execution_branch: "feature/00007-v0.1.6"
execution_started_at: "2026-09-14T18:19:15Z"
execution_updated_at: "2026-09-14T19:08:24Z"
execution_completed_at: null
current_step: "PLAN-00007-STEP-07"
---

# Delivery Plan 00007: V0 1 6 List Cache And Homebrew

> [!abstract] Plan status: `approved`
> Deliver `passalong` v0.1.6. `list` and `choose` read a local cache that
> `serve` keeps current, so they no longer wait for a slow connection.
> `check` reports how long the write probe took. passalong gets a Homebrew
> formula in `joelee/homebrew-oss`, kept current by a script that is part
> of each release. Encryption at rest moves to v0.1.7. The user settled
> D-01 to D-04 on 2026-09-14 (all as recommended); approved by the user
> at 2026-09-14T18:18:32Z; Builder-ready.

## 1. Objective and outcome

v0.1.5 is released. The maintainer's v0.1.6 road map asks for four things.
This plan delivers three of them, and moves the fourth (encryption) to its
own release, as the user decided:

1. **The list opens without waiting.** Over a mobile connection, `list`
   and `choose` take 3 to 5 s today, because each one connects and then
   reads every item's `meta.json`. A cache file that `serve` refreshes
   every minute lets them show the list without connecting. Other devices'
   changes appear within about a minute.
2. **`passalong check` reports write speed.** It times its write probe, now
   128 bytes, and shows the time and bytes per second.
3. **Homebrew install.** `brew install joelee/oss/passalong` installs it,
   and each release updates the formula with one script.

## 2. Source traceability

| Requirement | Source | Source location | Interpretation |
|---|---|---|---|
| PLAN-00007-REQ-01 to REQ-06 | User; D-01, D-02, D-05 to D-08 | `docs/backlog.md` v0.1.6 "**`serve` keeps the item list cached.**" and its sub-items; Agent suggested "serve keeps the item list cached" | A list cache kept by `serve`, used by `list` and `choose` |
| PLAN-00007-REQ-07 | User; D-09 | Backlog v0.1.6 "`passalong check` on storage write test, track and report the time to write a 128 bytes … file" | Timed 128-byte probe |
| PLAN-00007-REQ-08, REQ-09 | User; D-04, D-10 | Backlog v0.1.6 "Add Homebrew package on `https://github.com/joelee/homebrew-oss`"; local tap `../homebrew-oss` | Formula and release update flow |
| PLAN-00007-REQ-10 | Repository | Root `AGENTS.md` "Docs to maintain", "Backlog rules"; D-03 | Documentation; encryption moves to v0.1.7 in the backlog |
| PLAN-00007-REQ-11 | Repository | Root `AGENTS.md` "Release workflow" | v0.1.6 release preparation |
| PLAN-00007-REQ-12 | Repository | Root `AGENTS.md` "Non-negotiables" | Quality gates |
| PLAN-00007-REQ-13 | User | Road map: v0.1.x is the SSH-only CLI; S3 and Windows are v0.2 | Road-map guard |

## 3. Repository baseline

| Field | Value |
|---|---|
| Repository | `joelee/passalong`; tap `joelee/homebrew-oss` at `../homebrew-oss` |
| Branch | `feature/00007-v0.1.6`, from `main` at `8cbd113` plus the road map commit; not pushed |
| HEAD | `d5d4e2283bdb6158406287c5f9f7b33b7b841e5a` |
| Working tree at publication | Clean before allocation; the only change is this plan file |
| Release state | v0.1.5 released 2026-09-14: crates.io, GitHub release, Release run without annotations |
| Tap state | `../homebrew-oss` clean on `main` (`22e0d15`); formulas `cli-bot` (Rust, built from the crates.io `.crate` with `cargo install`) and `gmail-tool`; macOS workflow runs `brew install --build-from-source`, `brew test`, `brew audit --strict` |
| Applicable instructions | Root `AGENTS.md`; `docs/plans/AGENTS.md`; `../homebrew-oss/AGENTS.md` (packaging only, small formula updates) |

Findings from the planning research, 2026-09-14:

- **Why the list is slow.**
  - Every one-shot command opens its own SSH connection.
  - `Store::list` then reads each item's `meta.json`: one SFTP round trip
    per item.
  - `Store::list_ids` is a single directory listing, which pull mode
    already uses.
- **Where the cache code can go.**
  - `serve`'s tasks live in `passalong-core` (`serve::run_with_ready`).
  - `ServeOptions`, `ServeConfig`, and `Config` are public structs without
    `#[non_exhaustive]`. v0.1.2 added `serve.pull` the same way.
- **The Homebrew formula can be written against the published crate.**
  `passalong` 0.1.5 is on crates.io, and its crate includes `Cargo.lock`:
  - checksum `53eb16a803db8a6ae0884fd5f1b8c5e317f3fd79a48ee303c30dc2dc4f89710f`;
  - 102,732 bytes.

  So a formula can be written and tested now, and bumped to 0.1.6 at
  release.
- **Validation environment.** There is no `brew` on this machine. Docker
  is available, so the formula can be validated in the `homebrew/brew`
  image (Linux) before the tap's macOS workflow runs.
- **The current write probe** writes 22 bytes (`"passalong write probe\n"`).

## 4. Scope

### In scope

- A list cache in `passalong-core` (the file format, and refreshing it
  from the store). `serve` refreshes it; commands use and update it. It is
  off for the `local` backend.
- `list --nocache`; `choose` opens from the cache, `r` reloads the cache,
  and `R` reads the server.
- Two `[serve]` keys for the cache.
- `check` times a 128-byte write probe.
- `Formula/passalong.rb` and its CI entry in `../homebrew-oss`.
- `scripts/update-homebrew-formula.sh` in passalong, and a release workflow
  step.
- Docs; the backlog with encryption moved to v0.1.7; version 0.1.6.

### Out of scope

- **Encryption at rest:** v0.1.7, with its own plan (D-03).
- **Prebuilt Homebrew bottles, and pushing to the tap from the Release
  workflow:** the formula builds from source, and the user pushes the tap.
- **A socket between commands and `serve`:** not needed, since D-02 uses
  the cache file.
- **S3, Windows, the GUI, and Android client code:** road map.
- **Tagging, publishing, pushing the tap, and the release finalisation
  commit.**

## 5. Constraints and preserved decisions

- Every decision of PLAN-00001 to PLAN-00006 stays in force.
- **Compatibility of the published crates.**
  - Public API changes are additive.
  - The only exception is two new fields on the public `ServeConfig`
    struct, as in v0.1.2; the release notes say so.
  - `WriteProbe` and `Store::probe_write` keep their shape.
- **Results are the same with or without the cache.**
  - The cache only speeds up listing.
  - Every command that changes or reads items still does so on the server.
  - A cache from another server or config is never used.
- **The cache file holds item metadata, including text previews.** It is
  written with mode 0600 in serve's state folder.
- **Tests never touch the real clipboard, services, `serve`, or the tap's
  `main`.** Homebrew validation runs in a container.
- **Work in `../homebrew-oss`** follows its `AGENTS.md`: packaging only,
  small formula changes. It is committed on a branch there for the user to
  push.
- **Quality rules:** TDD with mocks; `just check` green at every step;
  coverage at least 80 %; rustdoc for public items; no `unsafe`.
- **Builder** works on `feature/00007-v0.1.6`, one commit per step, without
  review pauses unless blocked. It never tags, publishes, or pushes the
  tap.

## 6. Assumptions

None. Unresolved matters are recorded as decisions.

## 7. Decisions and blockers

| ID | Decision or blocker | Resolution | Owner | Status |
|---|---|---|---|---|
| D-01 | How staleness is detected. | **Confirmed by user (2026-09-14):** by comparing the id list. A refresh reads `Store::list_ids` (one directory listing), drops ids that are gone, and reads `get_meta` only for new ids, like pull mode. No timestamp file on the server, so writes by older clients are noticed too. | User | Resolved |
| D-02 | Where the cache lives and how it is used. | **Confirmed by user (2026-09-14):** a local cache file.<br>• `serve` refreshes it.<br>• `list` and `choose` read it without connecting when it was checked recently.<br>• Commands that connect anyway keep it current.<br>• `list --nocache` and `R` in `choose` read the server; `r` reloads the file. | User | Resolved |
| D-03 | Encryption at rest. | **Confirmed by user (2026-09-14):** not in v0.1.6. The backlog moves it to a v0.1.7 heading, with the design questions to answer in its own plan: what metadata is encrypted, the id's content hash, dedup, pull, and keys across devices. | User | Resolved |
| D-04 | How Homebrew installs passalong. | **Confirmed by user (2026-09-14):** built from the crates.io crate, like `cli-bot`. | User | Resolved |
| D-05 | The cache file. | **Resolved by planner:**<br>• **Name and place:** `list-cache.json` next to `serve.pid`: `${XDG_STATE_HOME:-~/.local/state}/passalong/` on Linux, `~/Library/Application Support/passalong/` on macOS.<br>• **Contents (JSON):** `version: 1`; `store` (an identity: `ssh <user>@<host>:<port>/<remote_path>`); `checked_at` (UTC); `items` (the metadata, newest first).<br>• **Writing:** through a temporary file and a rename, with mode 0600, so a reader never sees half a file. The last writer wins.<br>• **When it is used:** only if `store` matches the current config, and `checked_at` is no older than twice the check interval (default 2 min). Otherwise the command reads the server. A missing, unreadable, or future-version file counts as no cache. | Planner | Resolved |
| D-06 | Who keeps the cache current. | **Resolved by planner:**<br>• **`serve`** (ssh backend, `serve.list_cache` on) refreshes it at start and then every `serve.list_cache_check_secs`, over its own store connection, per D-01.<br>• **`list --nocache` and `R`** write the full list they read.<br>• **`file`, `clipboard`, `delete`, and `prune`** apply their own changes to an existing, matching cache without an extra round trip.<br>• **`load`, `cat`, and `get`** refresh it per D-01 after their output is written, at the cost of one directory listing plus any new metadata. Errors are logged at verbose and never fail the command.<br>• **`serve`'s own sends** appear at its next refresh. | Planner | Resolved |
| D-07 | Configuration. | **Resolved by planner:**<br>• `serve.list_cache` (boolean, default `true`), ignored for `local`, whose listing needs no network.<br>• `serve.list_cache_check_secs` (integer, default 60, 10 to 86400).<br>• Both are new public fields of `ServeConfig`, as `pull` was in v0.1.2; the release notes tell library users. | Planner | Resolved |
| D-08 | What `list` and `choose` show. | **Resolved by planner:**<br>• **`list`** prints the same table or JSON from the cache as from the server. It notes the cache at verbose level only, so scripts see no difference. `--nocache` reads the server and refreshes the cache.<br>• **`choose`** opens from a usable cache, with the status line saying `cached N s ago`, and `Loading...` only when it reads the server. `r` reloads the cache file (or the server when no usable cache exists), and `R` reads the server and refreshes the cache (`Reloading...`). `d` keeps its v0.1.5 behaviour: delete, then read the server. `?` lists `R`. | Planner | Resolved |
| D-09 | Timing `check`'s write probe. | **Resolved by planner:**<br>• **The probe:** `FsStore::probe_write` writes 128 bytes (`store::PROBE_BYTES`, a new public constant). `WriteProbe` stays as it is.<br>• **The timing:** `check` times the whole `probe_write` call (create folder, write, close, remove).<br>• **The line:** `storage write  ok    wrote and removed a 128-byte probe in 184 ms (696 B/s)`.<br>• **Documentation:** it says that with 128 bytes the figure mostly shows round-trip latency, not bandwidth. | Planner | Resolved |
| D-10 | Homebrew details. | **Resolved by planner:**<br>• **Formula** `../homebrew-oss/Formula/passalong.rb`, modelled on `cli-bot`: `url` of `passalong-<v>.crate` on static.crates.io, its crates.io checksum, `license "Apache-2.0"`, `depends_on "rust" => :build`, `cargo install *std_cargo_args`. Its caveats point to `passalong init`, `passalong check`, and `passalong service-install`. It starts at the published 0.1.5 and is bumped to 0.1.6 at release.<br>• **Tap files:** its test workflow matrix, README, `AGENTS.md`, and `docs-release.md` gain passalong. The change is committed on a branch `add-passalong` in the tap for the user to push.<br>• **Update script:** `scripts/update-homebrew-formula.sh vX.Y.Z [TAP_DIR]` (default `../homebrew-oss`). It sets the `url` and `sha256` from the crates.io API, and fails when the version is not published yet. `--sha256 <hex>` skips the network for tests.<br>• **Release workflow:** root `AGENTS.md` gains a step after the Release run: run the script, commit in the tap, and the user pushes.<br>• **Validation:** in the `homebrew/brew` container (`brew install --build-from-source`, `brew test`, `brew audit --strict --new`), and by the tap's macOS workflow once pushed. | Planner | Resolved |
| D-11 | Branch and version. | **Resolved by planner:** v0.1.6 on `feature/00007-v0.1.6`. | Planner | Resolved |

Blocking decisions: 0.

## 8. Affected architecture and components

| Area | Paths | Change |
|---|---|---|
| Cache | `crates/passalong-core/src/cache.rs` (new), `crates/passalong-core/src/lib.rs` | `ListCache`: identity, load, save (0600, atomic), freshness, refresh (D-01), apply put/delete; `refresh_loop` |
| Config | `crates/passalong-core/src/config.rs`, `docs/configuration.md` | `serve.list_cache`, `serve.list_cache_check_secs` |
| Store | `crates/passalong-core/src/store/{mod,fs_store}.rs` | `PROBE_BYTES`, 128-byte probe |
| CLI | `crates/passalong-cli/src/{cli,app,daemon}.rs`, `commands/{list,check,serve,file,clipboard,delete,prune,load,cat,get}.rs`, `commands/choose/` | Cache path, `--nocache`, cache use and upkeep, timing, `r`/`R` |
| Scripts | `scripts/update-homebrew-formula.sh` (new), `crates/passalong-cli/tests/homebrew_script.rs` (new) | Formula update |
| Tap | `../homebrew-oss/Formula/passalong.rb` (new), `.github/workflows/test-formula.yml`, `README.md`, `AGENTS.md`, `docs-release.md` | Package |
| Docs | `README.md`, `docs/{usage,configuration,architecture,developer-guide,backlog}.md`, root `AGENTS.md`, `CHANGELOG.md`, `docs/release/v0.1.6.md` (new) | Updated |

```rust
// passalong-core::cache (new, public)
pub struct ListCache { /* version, store, checked_at, items */ }
impl ListCache {
    pub fn store_identity(config: &Config) -> Option<String>; // None for local
    pub fn load(path: &Path) -> Option<ListCache>;
    pub fn save(&self, path: &Path) -> io::Result<()>;          // 0600, atomic
    pub fn is_fresh(&self, now: DateTime<Utc>, max_age: Duration) -> bool;
    pub async fn refresh(&mut self, store: &dyn Store, now: DateTime<Utc>) -> Result<(), StoreError>;
    pub fn apply_put(&mut self, meta: ItemMeta);
    pub fn apply_delete(&mut self, id: &ItemId);
}
pub async fn refresh_loop(/* opener, path, identity, interval, shutdown */);
```

## 9. Requirement catalogue

### PLAN-00007-REQ-01 — The list cache

- **Requirement:** `passalong_core::cache` per D-05 and D-01, with refresh
  reading `list_ids` once and `get_meta` only for new ids.
- **Rationale:** The shared basis of REQ-02 to REQ-05.
- **Source:** Backlog; D-01, D-05.
- **Acceptance evidence:** AC-01, AC-02.

### PLAN-00007-REQ-02 — `serve` keeps the cache current

- **Requirement:** Per D-06 and D-07. `serve` refreshes the cache at start
  and every `serve.list_cache_check_secs`, with the ssh backend only.
- **Rationale:** User request.
- **Source:** Backlog.
- **Acceptance evidence:** AC-03.

### PLAN-00007-REQ-03 — `list` uses the cache; `--nocache`

- **Requirement:** Per D-02, D-05, and D-08. A usable cache means `list`
  makes no connection.
- **Rationale:** The 3 to 5 s wait.
- **Source:** Backlog.
- **Acceptance evidence:** AC-04.

### PLAN-00007-REQ-04 — `choose` with `r` and `R`

- **Requirement:** Per D-08.
- **Rationale:** User request.
- **Source:** Backlog ("In `choose` TUI, `r` will reload from cache, `R`
  will reload from remote").
- **Acceptance evidence:** AC-05.

### PLAN-00007-REQ-05 — Commands keep the cache current

- **Requirement:** Per D-06.
- **Rationale:** User request ("check … on every connection to the remote
  for tasks like `cat`, `load`").
- **Source:** Backlog.
- **Acceptance evidence:** AC-06.

### PLAN-00007-REQ-06 — Cache configuration

- **Requirement:** Per D-07, documented, with invalid values rejected by
  key name.
- **Rationale:** User request ("every minute (configurable)"; "disable for
  local path").
- **Source:** Backlog.
- **Acceptance evidence:** AC-07.

### PLAN-00007-REQ-07 — Timed write probe

- **Requirement:** Per D-09.
- **Rationale:** User request.
- **Source:** Backlog.
- **Acceptance evidence:** AC-08.

### PLAN-00007-REQ-08 — Homebrew formula

- **Requirement:** Per D-10: the formula and the tap's CI, README,
  `AGENTS.md`, and `docs-release.md`, committed on `add-passalong` in
  `../homebrew-oss`.
- **Rationale:** User request.
- **Source:** Backlog.
- **Acceptance evidence:** AC-09, AC-10.

### PLAN-00007-REQ-09 — Formula update script and release step

- **Requirement:** Per D-10.
- **Rationale:** Keeps the formula current with each release.
- **Source:** Backlog; root `AGENTS.md` "Release workflow".
- **Acceptance evidence:** AC-11.

### PLAN-00007-REQ-10 — Documentation and backlog

- **Requirement:**
  - **`README.md`:** Homebrew install, and the cache.
  - **`docs/usage.md`:** `list --nocache`, `choose` `r`/`R`, the `check`
    timing.
  - **`docs/configuration.md`:** the two keys and the cache file.
  - **`docs/architecture.md`:** the cache.
  - **`docs/developer-guide.md`:** the script.
  - **`docs/backlog.md`:** the delivered items removed, and encryption
    under v0.1.7.
- **Rationale:** Documentation rules; D-03.
- **Source:** Root `AGENTS.md`.
- **Acceptance evidence:** AC-12.

### PLAN-00007-REQ-11 — v0.1.6 release preparation

- **Requirement:** Version 0.1.6; CHANGELOG `Unreleased` entries;
  `docs/release/v0.1.6.md` drafted with absolute links.
- **Rationale:** Release workflow.
- **Source:** Root `AGENTS.md`.
- **Acceptance evidence:** AC-13.

### PLAN-00007-REQ-12 — Quality gates

- **Requirement:** TDD evidence; `just check` per step; `just ci` locally;
  branch CI green on all four jobs; coverage at least 80 %.
- **Rationale:** Repository rules.
- **Source:** Root `AGENTS.md`.
- **Acceptance evidence:** AC-14.

### PLAN-00007-REQ-13 — Road map guard

- **Requirement:** No encryption, S3, Windows, GUI, or Android client code;
  backends stay `local` and `ssh`.
- **Rationale:** Road map; D-03.
- **Source:** User.
- **Acceptance evidence:** AC-15.

## 10. Delivery strategy

1. **The probe timing (STEP-01).** Small and independent.
2. **The cache core and configuration (STEP-02).** The basis of the rest.
3. **`serve` refreshing the cache (STEP-03).**
4. **`list`, and the commands keeping the cache current (STEP-04).**
5. **`choose` (STEP-05).**
6. **Homebrew (STEP-06, STEP-07).** The formula in the tap, then the
   update script and the release step.
7. **Records, then the final gate (STEP-08, STEP-09).**

Each step ends with `just check` green and one commit in passalong. STEP-06
also commits in `../homebrew-oss`. Docker tests run at STEP-04 and STEP-09.

## 11. Detailed implementation steps

### PLAN-00007-STEP-01 — Timed write probe

- **Objective:** Implement REQ-07.
- **Requirements:** `PLAN-00007-REQ-07`
- **Depends on:** None
- **Affected components:** `crates/passalong-core/src/store/{mod,fs_store}.rs`,
  `crates/passalong-cli/src/commands/check.rs`
- **Preconditions:** Plan approved.
- **Test or evidence first:**
  - The `FsStore` probe test expects a 128-byte write, counted through
    the test filesystem's call counters or the bytes written.
  - A unit test for the `check` line's formatting: milliseconds, and B/s,
    KiB/s, or MiB/s.
  - The `check` tests match the line's pattern.
- **Implementation tasks:** Add `PROBE_BYTES` and the 128-byte probe; time
  the probe call in `check`; format the line.
- **Documentation/configuration/operations:** Usage, `check`.
- **Verification:** `cargo test -p passalong-core --all-features store`;
  `cargo test -p passalong check`; `just check`.
- **Completion criteria:** AC-08.
- **Rollback or recovery:** Revert.
- **Builder stop conditions:** None expected.

### PLAN-00007-STEP-02 — The list cache and its configuration

- **Objective:** Implement REQ-01 and REQ-06.
- **Requirements:** `PLAN-00007-REQ-01`, `PLAN-00007-REQ-06`
- **Depends on:** `PLAN-00007-STEP-01`
- **Affected components:** `crates/passalong-core/src/cache.rs`,
  `lib.rs`, `config.rs`, `docs/configuration.md`
- **Preconditions:** None.
- **Test or evidence first:**
  - **Identity:** the ssh form, and `None` for `local`.
  - **`save` then `load`:** round-trips; the file has mode 0600; no
    temporary file is left behind.
  - **`load` gives no cache for:** a missing file, invalid JSON, or a newer
    `version`.
  - **Freshness:** the file must match the current store and be no older
    than twice the interval.
  - **`refresh`:** over a `FaultyFs` store, it reads one directory and only
    the new items' metadata, and drops ids that are gone.
  - **`apply_put` and `apply_delete`:** keep the list newest first, without
    duplicates.
  - **Configuration:** the defaults, the key names in errors, and the range
    10 to 86400.
- **Implementation tasks:** Write the module and its config keys per D-05
  and D-07.
- **Documentation/configuration/operations:** Configuration: the keys and
  the cache file.
- **Verification:** `cargo test -p passalong-core --all-features cache
  config`; `just check`.
- **Completion criteria:** AC-01, AC-02, AC-07.
- **Rollback or recovery:** Revert.
- **Builder stop conditions:** Mode 0600 cannot be set on a supported
  platform.

### PLAN-00007-STEP-03 — `serve` refreshes the cache

- **Objective:** Implement REQ-02.
- **Requirements:** `PLAN-00007-REQ-02`
- **Depends on:** `PLAN-00007-STEP-02`
- **Affected components:** `crates/passalong-core/src/cache.rs`
  (`refresh_loop`), `crates/passalong-cli/src/{daemon,commands/serve}.rs`
- **Preconditions:** None.
- **Test or evidence first:**
  - **`refresh_loop` under paused tokio time:**
    - it writes the cache at start and after each interval;
    - a store error keeps the old file;
    - the loop stops on shutdown.
  - **`StatePaths`** includes `list-cache.json` on both platforms.
  - **A binary test:** `serve --daemon` with a local store writes no
    cache.
- **Implementation tasks:** Add `refresh_loop` per D-06; have the CLI's
  `serve` start it for the ssh backend when enabled.
- **Documentation/configuration/operations:** Architecture, "`serve`".
- **Verification:** `cargo test -p passalong-core --all-features cache`;
  `cargo test -p passalong serve`; `just check`.
- **Completion criteria:** AC-03.
- **Rollback or recovery:** Revert.
- **Builder stop conditions:** None expected.

### PLAN-00007-STEP-04 — `list` uses the cache; commands keep it current

- **Objective:** Implement REQ-03 and REQ-05.
- **Requirements:** `PLAN-00007-REQ-03`, `PLAN-00007-REQ-05`
- **Depends on:** `PLAN-00007-STEP-03`
- **Affected components:** `crates/passalong-cli/src/{cli,app}.rs`,
  `commands/{list,file,clipboard,delete,prune,load,cat,get}.rs`, tests
- **Preconditions:** None.
- **Test or evidence first:**
  - **Unit tests over a cache in a temporary state folder:**
    - `list` prints a fresh cache and does not open the store;
    - an old or mismatched cache reads the store and writes the cache;
    - `--nocache` reads the store.
  - **File, clipboard, delete, and prune** update an existing cache.
  - **Load, cat, and get** refresh it after their output.
  - **A Docker test (the key evidence):**
    - with an ssh config and a fresh cache, `list` succeeds with the
      server stopped, which proves no connection;
    - `list --nocache` then fails to connect.
- **Implementation tasks:** Per D-02, D-05, D-06, and D-08.
- **Documentation/configuration/operations:** Usage, `list`.
- **Verification:** `cargo test -p passalong`; `just test-integration`;
  `just check`.
- **Completion criteria:** AC-04, AC-06.
- **Rollback or recovery:** Revert.
- **Builder stop conditions:** A command's result would differ with the
  cache.

### PLAN-00007-STEP-05 — `choose` opens from the cache; `r` and `R`

- **Objective:** Implement REQ-04.
- **Requirements:** `PLAN-00007-REQ-04`
- **Depends on:** `PLAN-00007-STEP-04`
- **Affected components:** `crates/passalong-cli/src/commands/choose/`,
  `app.rs`
- **Preconditions:** None.
- **Test or evidence first:**
  - **`run_picker` with a fake cache source:**
    - it opens from a fresh cache with `cached N s ago`;
    - `r` reloads the cache file;
    - `R` shows `Reloading...`, reads the store, and saves the cache;
    - without a cache it shows `Loading...`, as before.
  - **The help lists `R`.**
- **Implementation tasks:** Per D-08.
- **Documentation/configuration/operations:** Usage, the `choose` keys.
- **Verification:** `cargo test -p passalong choose`; `just check`.
- **Completion criteria:** AC-05.
- **Rollback or recovery:** Revert.
- **Builder stop conditions:** None expected.

### PLAN-00007-STEP-06 — Homebrew formula in the tap

- **Objective:** Implement REQ-08.
- **Requirements:** `PLAN-00007-REQ-08`
- **Depends on:** `PLAN-00007-STEP-05`
- **Affected components:** `../homebrew-oss/{Formula/passalong.rb,
  .github/workflows/test-formula.yml, README.md, AGENTS.md,
  docs-release.md}` on branch `add-passalong`
- **Preconditions:** The tap is clean.
- **Test or evidence first:** `brew audit` and `brew install` fail for a
  missing formula. This is recorded, then the formula is written.
- **Implementation tasks:**
  1. Write the formula at 0.1.5 per D-10.
  2. Add it to the matrix and the tap's docs.
  3. Validate in `homebrew/brew`: tap the local folder, then
     `brew install --build-from-source joelee/oss/passalong`,
     `brew test joelee/oss/passalong`, and
     `brew audit --strict --new joelee/oss/passalong`.
  4. Commit on `add-passalong` in the tap.
- **Documentation/configuration/operations:** The tap's docs.
- **Verification:** The container run; `git -C ../homebrew-oss log -1`;
  `just check` (passalong unchanged apart from the work log).
- **Completion criteria:** AC-09, AC-10 (AC-10 is completed when the user
  pushes the tap).
- **Rollback or recovery:** Delete the tap branch.
- **Builder stop conditions:** `brew audit` requires changes to passalong
  itself, such as licence metadata.

### PLAN-00007-STEP-07 — Formula update script and release step

- **Objective:** Implement REQ-09.
- **Requirements:** `PLAN-00007-REQ-09`
- **Depends on:** `PLAN-00007-STEP-06`
- **Affected components:** `scripts/update-homebrew-formula.sh`,
  `crates/passalong-cli/tests/homebrew_script.rs`, root `AGENTS.md`,
  `docs/developer-guide.md`
- **Preconditions:** None.
- **Test or evidence first:** Tests over a fixture formula, run with
  `--sha256`, so no network is used:
  - the `url` and `sha256` are updated, and nothing else changes;
  - a malformed version is rejected;
  - a missing formula is rejected;
  - a missing tap directory is rejected.
- **Implementation tasks:** Write the script per D-10; add the step to the
  release workflow; document it.
- **Documentation/configuration/operations:** Root `AGENTS.md` release
  workflow; the developer guide.
- **Verification:** `cargo test -p passalong --test homebrew_script`; a dry
  run against a copy of the tap's formula for 0.1.5 using the real
  crates.io checksum; `just check`.
- **Completion criteria:** AC-11.
- **Rollback or recovery:** Revert.
- **Builder stop conditions:** None expected.

### PLAN-00007-STEP-08 — Documentation and v0.1.6 release preparation

- **Objective:** Implement REQ-10 and REQ-11.
- **Requirements:** `PLAN-00007-REQ-10`, `PLAN-00007-REQ-11`
- **Depends on:** `PLAN-00007-STEP-07`
- **Affected components:** Docs, `CHANGELOG.md`, versions,
  `docs/release/v0.1.6.md`
- **Preconditions:** None.
- **Test or evidence first:** Documentation step. The evidence is the
  consistency check, `scripts/check-links.sh`, and `passalong --version`.
- **Implementation tasks:**
  1. Complete the docs.
  2. Update the backlog: drop the delivered items, and move encryption
     under v0.1.7.
  3. Bump to 0.1.6.
  4. Draft the release notes, including the cache timing measured against
     the Docker sshd.
- **Documentation/configuration/operations:** This step.
- **Verification:** Consistency check; `just check`; `just publish-dry-run
  --allow-dirty`.
- **Completion criteria:** AC-12, AC-13.
- **Rollback or recovery:** Revert.
- **Builder stop conditions:** Documented behaviour disagrees with code.

### PLAN-00007-STEP-09 — Final quality gate

- **Objective:** Prove the plan.
- **Requirements:** `PLAN-00007-REQ-12`, `PLAN-00007-REQ-13`
- **Depends on:** `PLAN-00007-STEP-08`
- **Affected components:** None new.
- **Test or evidence first:** Verification-only step.
- **Implementation tasks:**
  1. Run `just ci`.
  2. Push the branch and record CI on all four jobs.
  3. Measure `list` with and without the cache against the Docker sshd.
  4. Review the diff for scope.
- **Documentation/configuration/operations:** None.
- **Verification:** `just ci`; GitHub CI; scope review.
- **Completion criteria:** AC-14, AC-15.
- **Rollback or recovery:** Not applicable.
- **Builder stop conditions:** A gate fails in a way that needs a scope
  change.

## 12. Cross-cutting concerns

| Area | Applicability | Planned action or reason not applicable | Step or requirement |
|---|---|---|---|
| Compatibility and APIs | Applicable | New `cache` module and `PROBE_BYTES`; two new `ServeConfig` fields (noted in the release notes); `WriteProbe` unchanged; storage format unchanged | REQ-01, REQ-06, REQ-07 |
| Data and migration | Applicable | A new local file only; old clients unaffected; no server-side format change | REQ-01 |
| Security and privacy | Applicable | The cache holds metadata and previews: mode 0600 in the private state folder, and never used for another server | REQ-01 |
| Performance and scale | Applicable | `list` and `choose` without a connection; refreshes read one listing plus new metadata; `load`/`cat`/`get` add one listing | REQ-02 to REQ-05 |
| Reliability and failure handling | Applicable | Atomic writes; a bad or old cache falls back to the server; cache errors never fail a command | REQ-01, REQ-05 |
| Observability and operations | Applicable | Cache use and refreshes logged at verbose; `choose` shows the cache age | REQ-03, REQ-04 |
| Dependencies and supply chain | Not applicable | No new crates | — |
| Accessibility and UX | Applicable | Instant list; `r` and `R` explained in the help | REQ-03, REQ-04 |
| Documentation and release | Applicable | Docs, backlog, release notes, release workflow gains the tap step | REQ-09 to REQ-11 |
| Deployment and rollback | Applicable | `serve.list_cache = false` turns the cache off; the tap branch can be dropped | REQ-06, REQ-08 |

## 13. Verification strategy

| Level | Evidence or command | When | Required result |
|---|---|---|---|
| Unit | `cargo test -p <crate> <module>` | Every step, red then green | Pass |
| Workspace gate | `just check` | Every step | Exit 0 |
| SSH integration | `just test-integration`, including `list` from the cache with the server stopped | STEP-04, 09 | Exit 0 |
| Scripts | `cargo test -p passalong --test homebrew_script` | STEP-07, 09 | Pass |
| Homebrew | `homebrew/brew` container: install, test, audit | STEP-06 | Pass |
| Timing | `list` with and without the cache against the Docker sshd | STEP-09 | Recorded |
| Full | `just ci` and GitHub CI | STEP-09 | Pass on all four jobs |
| Tap | The tap's macOS workflow after the user pushes `add-passalong` | Release | Pass |

## 14. Acceptance criteria

- [ ] `PLAN-00007-AC-01` `ListCache` saves with mode 0600 through a temporary file and a rename, loads what it saved, and treats a missing, invalid, newer-version, mismatched-store, or older-than-twice-the-interval file as no cache.
- [ ] `PLAN-00007-AC-02` `ListCache::refresh` performs one directory listing and reads only new items' metadata, dropping ids no longer stored (`FaultyFs` counters).
- [ ] `PLAN-00007-AC-03` With the ssh backend and `serve.list_cache` on, `serve` writes the cache at start and every `serve.list_cache_check_secs`, keeps the old file when the store fails, and writes none for `local`.
- [ ] `PLAN-00007-AC-04` With a fresh cache, `list` (table and `--json`) prints the cached items without connecting, proved by the Docker test with the server stopped; `--nocache`, an old cache, or another store's cache reads the server.
- [ ] `PLAN-00007-AC-05` `choose` opens from a usable cache showing `cached N s ago`; `r` reloads the cache, `R` shows `Reloading...` and reads the server; without a cache it shows `Loading...`; the help lists `R`.
- [ ] `PLAN-00007-AC-06` `file`, `clipboard`, `delete`, and `prune` update an existing cache without an extra round trip; `load`, `cat`, and `get` refresh it after their output; a cache error never fails these commands.
- [ ] `PLAN-00007-AC-07` `serve.list_cache` (default `true`) and `serve.list_cache_check_secs` (default 60, 10 to 86400) parse, are documented, and bad values are rejected naming the key.
- [ ] `PLAN-00007-AC-08` The write probe is 128 bytes and `check` prints `wrote and removed a 128-byte probe in <N> ms (<rate>)`.
- [ ] `PLAN-00007-AC-09` `../homebrew-oss/Formula/passalong.rb` builds passalong 0.1.5 from its crates.io crate; in the `homebrew/brew` container `brew install --build-from-source`, `brew test`, and `brew audit --strict --new` pass; the tap's matrix and docs include passalong; the change is committed on `add-passalong` there.
- [ ] `PLAN-00007-AC-10` After the user pushes `add-passalong`, the tap's macOS workflow passes for passalong.
- [ ] `PLAN-00007-AC-11` `scripts/update-homebrew-formula.sh` updates only the formula's `url` and `sha256`, rejects bad versions and missing files, and the root `AGENTS.md` release workflow includes the tap step.
- [ ] `PLAN-00007-AC-12` The README, usage, configuration, architecture, and developer guide describe the cache, `--nocache`, `r`/`R`, the probe timing, and Homebrew; the consistency check passes; the backlog lists encryption under v0.1.7 and drops the delivered items.
- [ ] `PLAN-00007-AC-13` Crates at 0.1.6; `passalong --version` prints `passalong 0.1.6`; CHANGELOG `Unreleased` lists the changes; `docs/release/v0.1.6.md` exists with only absolute links.
- [ ] `PLAN-00007-AC-14` `just ci` passes locally and GitHub CI passes on Linux, macOS, Xvfb, and Android for the final commit; coverage is at least 80 %.
- [ ] `PLAN-00007-AC-15` The diff adds no encryption, S3, Windows, GUI, or Android client code; backends are `local` and `ssh`.

## 15. Risks and mitigations

| Risk | Likelihood | Impact | Mitigation or test | Owner/step |
|---|---|---|---|---|
| A stale cache hides a new item | Medium | Low | Up to about a minute while `serve` runs; the freshness limit otherwise; `--nocache` and `R`; commands that connect refresh it | STEP-02 to 05 |
| Two writers race on the cache file | Medium | Low | Atomic rename, and each write is a complete list; the next refresh corrects a lost update | STEP-02 |
| A cache from another config is used | Low | Medium | The store identity must match | STEP-02 |
| Previews leak through the cache file | Low | Medium | Mode 0600 in the private state folder | STEP-02 |
| `load`/`cat`/`get` get slower on slow links | Medium | Low | One listing after the output is written; errors ignored | STEP-04 |
| The Homebrew build differs on macOS | Medium | Low | The tap's macOS workflow; the formula mirrors the working `cli-bot` one | STEP-06 |
| `brew audit --new` wants metadata passalong lacks | Low | Low | Stop condition; the crate already has a licence, description, and homepage | STEP-06 |

## 16. Builder hand-off

- **Start condition:** User approval and a clean repository, and a clean
  tap before STEP-06.
- **First step:** `PLAN-00007-STEP-01`.
- **Required sequence:** STEP-01 → … → STEP-09, one commit per step, and
  one commit in the tap at STEP-06.
- **Parallel-safe work:** None (single Builder).
- **Do not change:** approved scope, requirements, steps, acceptance
  criteria, or content outside Builder's permitted work-log area.
- **Escalate when:** a stop condition triggers, or a § 7 decision proves
  unworkable.
- **Completion hand-off:** the work log with evidence, the cache timings,
  CI results, coverage, and the tap branch to push. AC-10 completes after
  the user pushes the tap.

<!-- BUILDER_WORK_LOG_START -->
## 17. Builder Work Log

> [!warning] Builder-maintained section
> Delivery Planner creates this section. After approval, Builder may update only
> this delimited section and the Builder-maintained front-matter fields. Builder
> must preserve prior entries and use UTC timestamps.

### Step status

| Step | Status | Started (UTC) | Completed (UTC) | Evidence | Builder notes |
|---|---|---|---|---|---|
| PLAN-00007-STEP-01 | completed | 2026-09-14T18:19:15Z | 2026-09-14T18:21:10Z | Commit `build: complete PLAN-00007-STEP-01 - Timed write probe`; `just check` green, 92.70% lines | AC-08. store::PROBE_BYTES = 128 (new public constant); FsStore writes probe_content (128 bytes); check times the whole probe_write call and prints probe_line; WriteProbe unchanged. Usage (example and note on latency), architecture, CHANGELOG updated |
| PLAN-00007-STEP-02 | completed | 2026-09-14T18:21:28Z | 2026-09-14T18:25:33Z | Commit `build: complete PLAN-00007-STEP-02 - The list cache and its configuration`; `just check` green, 92.83% lines | AC-01, AC-02, AC-07. New public module passalong_core::cache; ServeConfig gains list_cache and list_cache_check_secs (as pull did in v0.1.2). Configuration docs (keys and the cache file) and CHANGELOG updated |
| PLAN-00007-STEP-03 | completed | 2026-09-14T18:26:03Z | 2026-09-14T18:34:16Z | Commit `build: complete PLAN-00007-STEP-03 - serve refreshes the cache`; refresh_loop tests (start, interval, kept file on error, reconnect, stop); StatePaths.cache tests on both platforms; binary test: no cache for local | The loop starts once serve reports ready, over its own connection; list_cache_identity gates it on the ssh backend and serve.list_cache. |
| PLAN-00007-STEP-04 | completed | 2026-09-14T18:34:27Z | 2026-09-14T18:47:03Z | Commit `build: complete PLAN-00007-STEP-04 - list uses the cache; commands keep it current`; list unit tests (fresh cache opens nothing; old or other-store cache reads and writes; --nocache); list_cache tests (apply, refresh, unwritable cache fails nothing); local binary no-connection test; Docker cache test | CacheFile and the Recording store wrapper live in crates/passalong-cli/src/list_cache.rs; commands are unchanged apart from list. |
| PLAN-00007-STEP-05 | completed | 2026-09-14T18:47:03Z | 2026-09-14T18:50:27Z | Commit `build: complete PLAN-00007-STEP-05 - choose opens from the cache; r and R`; run_picker tests with a Lister over a cache file and a ManualClock; state test for R; help lists R | d keeps reading the server after a delete, through the cache so it is rewritten too. |
| PLAN-00007-STEP-06 | completed | 2026-09-14T18:50:27Z | 2026-09-14T19:08:24Z | Commit `build: complete PLAN-00007-STEP-06 - Homebrew formula in the tap`; homebrew/brew install, test, and both audits exit 0; tap commit 7930f4d on add-passalong | AC-10 completes when the user pushes add-passalong and the tap's macOS workflow passes. |
| PLAN-00007-STEP-07 | not-started | — | — | — | — |
| PLAN-00007-STEP-08 | not-started | — | — | — | — |
| PLAN-00007-STEP-09 | not-started | — | — | — | — |

Allowed status values: `not-started`, `in-progress`, `blocked`, `completed`,
`skipped`. A skipped step requires explicit user approval recorded in Evidence.

### Execution log

| Timestamp (UTC) | Step | Event | Evidence or reference | Next action |
|---|---|---|---|---|
| 2026-09-14T18:19:15Z | PLAN-00007 | Plan approved (commit 4ea0c75); Builder starts on feature/00007-v0.1.6 | `docs(plan): approve PLAN-00007 - V0 1 6 List Cache And Homebrew` | Begin PLAN-00007-STEP-01 |
| 2026-09-14T18:19:15Z | PLAN-00007-STEP-01 | Started | — | Red phase |
| 2026-09-14T18:21:10Z | PLAN-00007-STEP-01 | Verified and committed (continuous execution authorised by the user) | `build: complete PLAN-00007-STEP-01 - Timed write probe` | Begin PLAN-00007-STEP-02 |
| 2026-09-14T18:21:28Z | PLAN-00007-STEP-02 | Started | — | Red phase |
| 2026-09-14T18:25:33Z | PLAN-00007-STEP-02 | Verified and committed (continuous execution authorised by the user) | `build: complete PLAN-00007-STEP-02 - The list cache and its configuration` | Begin PLAN-00007-STEP-03 |
| 2026-09-14T18:26:03Z | PLAN-00007-STEP-03 | Started | — | Red phase |
| 2026-09-14T18:34:16Z | PLAN-00007-STEP-03 | Verified and committed (continuous execution authorised by the user) | `build: complete PLAN-00007-STEP-03 - serve refreshes the cache` | Begin PLAN-00007-STEP-04 |
| 2026-09-14T18:34:27Z | PLAN-00007-STEP-04 | Started | — | Red phase |
| 2026-09-14T18:47:03Z | PLAN-00007-STEP-04 | Verified and committed (continuous execution authorised by the user) | `build: complete PLAN-00007-STEP-04 - list uses the cache; commands keep it current` | Begin PLAN-00007-STEP-05 |
| 2026-09-14T18:47:03Z | PLAN-00007-STEP-05 | Started | — | Red phase |
| 2026-09-14T18:50:27Z | PLAN-00007-STEP-05 | Verified and committed (continuous execution authorised by the user) | `build: complete PLAN-00007-STEP-05 - choose opens from the cache; r and R` | Begin PLAN-00007-STEP-06 |
| 2026-09-14T18:50:27Z | PLAN-00007-STEP-06 | Started | — | Red phase |
| 2026-09-14T19:08:24Z | PLAN-00007-STEP-06 | Verified and committed (continuous execution authorised by the user) | `build: complete PLAN-00007-STEP-06 - Homebrew formula in the tap` | Begin PLAN-00007-STEP-07 |

### Deviations and blockers

| Timestamp (UTC) | Step | Deviation or blocker | Impact | Decision required from |
|---|---|---|---|---|
| 2026-09-14T18:47:03Z | PLAN-00007-STEP-04 | The Docker test cannot stop the shared SSH container without breaking tests running beside it, so 'list works with no connection' is proved by a local binary test whose ssh config points at 127.0.0.1:1, where nothing listens, and where list --nocache fails. The Docker test proves the cache follows clipboard, delete, and get, and that list prints the same from the cache as with --nocache. | Stronger no-connection evidence (no server exists at all); the ssh round trip is still covered in Docker. | Builder |
| 2026-09-14T18:47:03Z | PLAN-00007-STEP-04 | An old cache of the same store is refreshed (one id listing plus new metadata) rather than read in full; --nocache reads every item's metadata. | Same output with fewer round trips, per D-01. | Builder |
| 2026-09-14T18:50:27Z | PLAN-00007-STEP-05 | choose still connects before it shows the list, because g, d, R, and the actions need the store; the cache saves the listing time (one metadata read per item) but not the connection time. | Most of the 3 to 5 s wait on a mobile connection is the listing; a lazy connection could follow in a later release. | Builder |
| 2026-09-14T19:08:24Z | PLAN-00007-STEP-06 | Homebrew 7.0.1 refuses formulae from untrusted taps, and a tap cloned from a local folder cannot be trusted by name (brew trust joelee/oss was recorded but ignored). The container validation and the tap's workflow set HOMEBREW_NO_REQUIRE_TAP_TRUST=1, which Homebrew calls deprecated but honours. Users of the GitHub tap run brew trust joelee/oss and brew tap joelee/oss once, verified in the container with cli-bot; the tap README and passalong's README say so. | The tap workflow keeps working for every formula on Homebrew 7; users need one extra command. The workflow change also affects cli-bot and gmail-tool. | Builder |

### Verification results

| Timestamp (UTC) | Step | Command or check | Result | Evidence |
|---|---|---|---|---|
| 2026-09-14T18:21:10Z | PLAN-00007-STEP-01 | Red: `cargo test -p passalong-core --all-features --lib store`; `cargo test -p passalong --no-run` | Exit 101 (expected) | core: probe_content and store::PROBE_BYTES not found; CLI: probe_line not found (4) |
| 2026-09-14T18:21:10Z | PLAN-00007-STEP-01 | `cargo test -p passalong-core --all-features --lib store`; `cargo test -p passalong` | Pass | the_probe_is_probe_bytes_long; the_probe_line_shows_the_time_and_the_rate (184 ms 696 B/s, 2.0 ms 62.5 KiB/s, 0.1 ms 1.2 MiB/s, zero duration); check unit and binary tests match "wrote and removed a 128-byte probe in " |
| 2026-09-14T18:21:10Z | PLAN-00007-STEP-01 | `just check` | Exit 0 | Lines 92.70% |
| 2026-09-14T18:25:33Z | PLAN-00007-STEP-02 | Red: `cargo test -p passalong-core --all-features --lib` | Exit 101 (expected) | 31 errors: ListCache, CACHE_VERSION, max_age and the list_cache fields not found |
| 2026-09-14T18:25:33Z | PLAN-00007-STEP-02 | `cargo test -p passalong-core --all-features --lib` | Pass | cache: ssh identity and none for local; save/load round trip with mode 0600 and no temporary file left; missing, invalid, newer-version files are no cache; usable only for its store within two intervals either way; refresh = 1 ReadDir + 1 OpenRead for 1 new item, drops the deleted one; failed refresh leaves the cache unchanged; apply_put/apply_delete keep newest first without duplicates; read makes a complete cache. config: defaults true/60, set false/30, 9 and 86401 rejected naming serve.list_cache_check_secs |
| 2026-09-14T18:25:33Z | PLAN-00007-STEP-02 | `just check` | Exit 0 | Lines 92.83%; cache.rs 98.35% |
| 2026-09-14T18:34:16Z | PLAN-00007-STEP-03 | cargo test -p passalong-core --all-features cache | pass | 10 passed, including the two refresh_loop tests under paused time |
| 2026-09-14T18:34:16Z | PLAN-00007-STEP-03 | cargo test -p passalong serve | pass | 6 unit + 4 binary serve tests passed; serve --daemon with a local store writes no list-cache.json |
| 2026-09-14T18:34:16Z | PLAN-00007-STEP-03 | just check | pass | exit 0; line coverage 92.28 %, cache.rs 97.68 % |
| 2026-09-14T18:47:03Z | PLAN-00007-STEP-04 | cargo test -p passalong | pass | 161 unit + 31 local binary tests passed, including list_prints_a_fresh_cache_without_connecting_and_nocache_connects |
| 2026-09-14T18:47:03Z | PLAN-00007-STEP-04 | just test-integration | pass | the_list_cache_follows_commands_and_lists_what_the_server_does passed against the Docker SSH server |
| 2026-09-14T18:47:03Z | PLAN-00007-STEP-04 | just check | pass | exit 0; line coverage 92.44 %, list_cache.rs 98.19 %, commands/list.rs 96.89 % |
| 2026-09-14T18:50:26Z | PLAN-00007-STEP-05 | cargo test -p passalong choose | pass | choose tests passed, including a fresh cache opening with 'cached 30 s ago', r reloading the file, R reading the server and saving the cache, and an old cache showing Loading... |
| 2026-09-14T18:50:26Z | PLAN-00007-STEP-05 | just check | pass | exit 0; line coverage 92.48 %, choose/mod.rs 90.46 %, choose/view.rs 100 % |
| 2026-09-14T19:08:24Z | PLAN-00007-STEP-06 | brew audit --strict --new / brew install --build-from-source joelee/oss/passalong in homebrew/brew, tap without the formula | pass | fails first as expected: both exit 1, 'No available formula or cask with the name joelee/oss/passalong' |
| 2026-09-14T19:08:24Z | PLAN-00007-STEP-06 | homebrew/brew (Homebrew 7.0.1): brew install --build-from-source, brew test, brew audit --strict --new, brew audit --strict for joelee/oss/passalong | pass | all exit 0; the test runs --version, then stores and prints text through a local store |
| 2026-09-14T19:08:24Z | PLAN-00007-STEP-06 | git -C ../homebrew-oss log -1 | pass | 7930f4d 'Add the passalong formula' on add-passalong, not pushed |
| 2026-09-14T19:08:24Z | PLAN-00007-STEP-06 | just check | pass | exit 0; passalong changes only in the work log for this step |

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
| 2026-09-14T17:12:31Z | draft | Created with 13 requirements, 9 steps, 15 acceptance criteria, and decisions D-01 to D-11; D-01 to D-04 were answered by the user before drafting (all as recommended), so none blocks approval | User request to plan v0.1.6 from the backlog road map | User |
| 2026-09-14T18:18:32Z | approved | Approved; `plan_status`, `approved_at`, and `build_ready` set | User approval after committing the draft (060aa5c) | User |

## 19. External references

None. The planning research used the repository, the local tap
`../homebrew-oss`, the crates.io API for `passalong` 0.1.5, and Docker
availability on this machine.

## 20. Confidence

**Medium.** The slow path (one SFTP round trip per item after a new
connection) and the building blocks (`list_ids`, `get_meta`, serve's state
folder) are known from the code. The Homebrew formula follows a working
one in the same tap. The main uncertainties are keeping the cache current
across many commands without surprising results, and Homebrew behaviour on
macOS, which only the tap's workflow will show.
