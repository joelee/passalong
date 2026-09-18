---
title: "Code Review 00002: v0.2.0 Release Review"
aliases:
  - "Review 00002"
tags:
  - code-review
  - software-quality
  - opencode
type: code-review
status: open
review_id: "00002"
reviewed_at: "2026-09-15T19:16:27Z"
reviewer_agent: review
review_model: "openai/gpt-6-astra"
triggered_by: "user"
review_kind: initial
previous_review: null
repository: "joelee/passalong"
branch: "main"
review_mode: range
pr_reference: null
commit: "7bf861af7d2f6b3d43e8169b59ac15d49fd87039"
base_ref: "v0.1.6"
base_commit: "4e72760b40c5fcaf9b726c02e176d548b3a981c5"
head_ref: "v0.2.0"
head_commit: "7bf861af7d2f6b3d43e8169b59ac15d49fd87039"
scope: "v0.1.6..v0.2.0; release implementation, affected callers, tests, tooling, and release documentation"
related_plan: "docs/plans/00008-V0_1_7_Encryption_At_Rest.md"
files_changed: 62
files_reviewed: 57
diff_additions: 19596
diff_deletions: 202
blocking_issues: 4
issues:
  critical: 0
  major: 4
  medium: 5
  low: 2
  info: 0
  total: 11
categories:
  security: 3
  data-integrity: 1
  reliability: 3
  correctness: 2
  tests: 1
  documentation: 1
verdict: request-changes
review_complete: true
web_research_used: true
confidence: medium
sources:
  - https://www.ietf.org/archive/id/draft-ietf-secsh-filexfer-02.txt
  - https://github.com/joelee/passalong/pull/10
  - https://github.com/joelee/passalong/actions/runs/34997376939
  - https://github.com/joelee/passalong/actions/runs/34997645834
---

# Code Review 00002: v0.2.0 Release Review

> [!abstract] Verdict: `request-changes`
> The encryption primitives have useful safeguards, but migration/rotation coordination, fail-closed opening, plaintext cleanup, and header recovery need corrections before the release's encryption guarantees can be relied upon.

## Review target

| Field | Value |
|---|---|
| Review mode | Release-to-release range |
| PR or commit | Tagged release commit `7bf861af7d2f6b3d43e8169b59ac15d49fd87039`; the merge of PR #10 |
| Base | `v0.1.6`, peeled commit `4e72760b40c5fcaf9b726c02e176d548b3a981c5` |
| Head | `v0.2.0`, peeled commit `7bf861af7d2f6b3d43e8169b59ac15d49fd87039` |
| Branch | Local `main`; release developed on `feature/00008-v0.2.0` |
| Triggered by | user |
| Related plan | `docs/plans/00008-V0_1_7_Encryption_At_Rest.md`, PLAN-00008 |
| Worktree | Clean before review publication; no staged, unstaged, or untracked application changes |

**Scope inference:** “this v0.2.0 release” is interpreted as the complete changes since the preceding release, not just the release-record finalisation commit. Both tags exist locally. Local HEAD is `797710132c906214c44813fe2d0c0d393604e005`, one documentation update beyond the release merge. Its four changed files contain no application changes. The later AC-21 deferral is used as supplemental acceptance evidence, not included in the release diff. Application line references below therefore match the tagged target.

PR #10 is merged, with head `f3625c0f209dad3b319f8d9b89de40a6ecac0680` and merge commit equal to the peeled v0.2.0 tag. This report reviews the tag range; the PR metadata and checks are supporting evidence only.

## Executive summary

v0.2.0 adds optional client-side encryption to both filesystem backends: wrapped data keys, generated words, sealed metadata and streaming content, key files, encryption administration, migration/rotation/recovery, and key-aware background operation. The API and version changes are documented. The release diff contains 62 files, including a 7,776-line word list and historical design documents.

The most consequential defects are at the storage state-transition boundaries rather than in the AES-GCM record construction:

1. An upload already in progress can publish under the old key after rotation, with a successful result.
2. An incomplete encryption directory can be mistaken for an ordinary plaintext store by a keyless client.
3. Encryption leaves earlier plaintext upload/deletion staging outside the cleanup and warning paths.
4. An interrupted passphrase-header swap leaves a store that the prescribed recovery command cannot repair.

Five further Medium findings concern interrupted journal creation, header change detection, background cache identity, pull-mode corruption handling, and symlinked key-file locations. Two Low findings correct a vacuous compatibility assertion and an unsupported release-note testing claim.

`gh pr checks 10` reports successful Linux, macOS, Xvfb, and Android checks in both listed runs. The release records report 93.54% / 94.82% line coverage. Those results do not cover the interleavings and partial-write states identified here. No project code was executed by this reviewer.

## Issue summary

| Severity | Count | Merge impact |
|---|---:|---|
| Critical | 0 | Blocks merge/release |
| Major | 4 | Blocks merge/release |
| Medium | 5 | Changes requested |
| Low | 2 | Non-blocking |
| **Total** | **11** | |

## Findings

### Major

#### REV-00002-MAJ-01 — Rotation does not fence an upload that already passed its guard

> [!warning] Blocking
> - **Confidence:** High
> - **Category:** Data integrity
> - **Location:** `crates/passalong-core/src/store/fs_store.rs:499-514` — `FsStore::put`; `crates/passalong-core/src/store/fs_store.rs:448-485` — `stage_and_publish`; `crates/passalong-core/src/encryption/rewrite.rs:381-427` — `run`.
> - **Evidence:** `put` calls `check_key` only before creating staging and streaming the entire input. Publication subsequently renames staging into the fixed `v2/items` path without acquiring a shared exclusion mechanism or checking the generation at commit. Rotation moves the old `v2/items` aside, enumerates that source once, creates a new `v2/items`, and removes the source after verification. `.rewrite` is a marker checked by new operations, not an exclusion mechanism for operations already running.
> - **Failure scenario:** Device A begins a large file upload under key K1 and passes `check_key`. While A streams into `v2/tmp`, device B rotates the existing items to K2 and completes. A finishes, sees no matching K1 content key in the K2 items, and successfully publishes K1 ciphertext into the new `v2/items`. This item was never part of B's source snapshot and is unreadable after A joins K2. With `serve.after_send = "delete"`, the successful outcome also deletes A's local original (`serve/upload.rs:201-233`).
> - **Impact:** A reported-successful send can become inaccessible, and deleting the local original followed by replacing the old key can lose the only usable copy. The promised stop-writing-on-rotation property is not held.
> - **Recommendation:** Coordinate publication and rewrites with a protocol that actually excludes or fences old-generation commits, such as generation-specific destinations plus a validated commit protocol, or mutually respected writer/admin locks. A second uncoordinated stat immediately before rename only narrows the race. Keep the upload retryable and retain the local original when its generation changes.
> - **Suggested verification:** Pause an upload after its first guard and during streaming; complete a rotation on another connection; resume it. Assert either successful publication under the current key or an explicit retryable failure, never successful old-key publication. Cover the `after_send = "delete"` caller and both LocalFs and SFTP.
> - **References:** Repository evidence; PLAN-00008 REQ-06 and AC-09.

#### REV-00002-MAJ-02 — An encryption directory without its header can open as plaintext

> [!warning] Blocking
> - **Confidence:** High
> - **Category:** Security
> - **Location:** `crates/passalong-core/src/encryption/open.rs:63-86` — `open_with_key`; `crates/passalong-core/src/encryption/admin.rs:67-90` — `inspect`.
> - **Evidence:** Opening checks `encryption/header.json`, not the presence of `encryption/`. With no header, no `.rewrite`, no regular `items` file, and no device key, it returns `FsStore::new`. `inspect` similarly reports `Plain`. PLAN-00008's fail-closed rule explicitly forbids plaintext writes when the root contains `encryption/`.
> - **Failure scenario:** A keyless device sees a partially delivered synced store: `encryption/` exists, but `header.json` and the `items` stop file have not arrived yet, or the stop file still appears as the old directory. Its running client or a one-shot send opens the store as plaintext and writes clipboard/file contents under `items` and `tmp`.
> - **Trust boundary:** The device must not infer permission to disclose plaintext from incomplete state on the storage side. This case needs no cryptographic break or malicious process; out-of-order folder delivery is sufficient.
> - **Impact:** New content can be disclosed to the storage provider despite visible evidence that the store is being encrypted. The stop-file compatibility test only covers a fully installed stop file and cannot prevent this path.
> - **Recommendation:** Use one conservative state classifier for opening and administration. An existing encryption directory without a valid readable header must be broken/incomplete, never plaintext. Apply the same rule before plaintext mutations rather than relying solely on initial opening.
> - **Suggested verification:** Build the directory-only state and the directory-plus-old-items state; open without a key and attempt a send. Both must fail closed and write no marker bytes. Add staged-arrival tests for header, stop file, and encryption directory ordering.
> - **References:** Repository evidence; PLAN-00008 §5 fail-closed constraint and D-14.

#### REV-00002-MAJ-03 — Encryption silently retains plaintext in the old staging tree

> [!warning] Blocking
> - **Confidence:** High
> - **Category:** Security
> - **Location:** `crates/passalong-core/src/encryption/rewrite.rs:76-80,381-387,483-486` — migration source selection and cleanup; `crates/passalong-core/src/store/fs_store.rs:240-246,713-735` — encrypted staging selection and cleanup.
> - **Evidence:** Migration moves only `items`, and completion removes only `.rewrite/source` and `.rewrite`. Earlier plaintext `tmp` is untouched. Sealed stores clean `v2/tmp`, while `prune --plain` works on `plain/`. The new test at `fs_store.rs:1936-1944` explicitly asserts that `tmp/plain-leftover` survives sealed cleanup. Leftover warnings/counts inspect only `plain/items`.
> - **Failure scenario:** An earlier interrupted upload, cancelled background upload, or failed deletion leaves plaintext content and possibly metadata in `tmp/<token>` or `tmp/deleted-*`. A user migrates the store successfully, or starts fresh and prunes all `plain` items. The old staging bytes remain readable indefinitely, while `check` and `list` report no plaintext leftovers and neither pruning route removes them.
> - **Trust boundary:** The storage operator can read these ordinary files without the data key. This concerns current files in the live store, not snapshots or backups retained by a provider.
> - **Impact:** Encryption completion gives a false confidentiality boundary: clipboard/file contents, names, and unkeyed identifiers can remain exposed in a supported, application-created location.
> - **Recommendation:** Include legacy staging in the conversion inventory. Once old writers are safely quiesced, remove it, migrate it into an explicitly reported plaintext quarantine, or refuse completion until the user handles it. Warnings and `prune --plain` must account for all retained plaintext locations, including the empty-store setup path.
> - **Suggested verification:** Seed plaintext upload and deletion leftovers, then exercise migration, fresh start, and empty-store setup. Recursively inspect the entire root for markers and verify that any intentionally retained plaintext is reported and removable through the documented command.
> - **References:** Repository evidence; PLAN-00008 AC-03, REQ-09, and release Security guarantees.

#### REV-00002-MAJ-04 — Interrupted header replacement has no working recovery path

> [!warning] Blocking
> - **Confidence:** High
> - **Category:** Reliability
> - **Location:** `crates/passalong-core/src/encryption/header.rs:262-279` — `replace_header`; `crates/passalong-cli/src/commands/encrypt.rs:253-270` — `recover`; `crates/passalong-core/src/encryption/admin.rs:144-145,178` — setup/header publication.
> - **Evidence:** Changing the words moves `encryption` into `v2/tmp/old-header-*`, then renames the staged new header into place. No `.rewrite` plan records this operation. If the second rename fails, `HeaderMissing` tells every client to run `encrypt --recover`. That command finds no plan; `undo` finds no `.rewrite`; the CLI prints `no re-encryption to recover` and succeeds. No recovery code searches or restores the staged header folders. The header test at `header.rs:388-417` asserts the missing-header state, but does not recover it.
> - **Failure scenario:** An SFTP disconnect or process interruption occurs between the two header renames during a word change. All ordinary store opens now fail, including clients with valid data keys. The named remediation does nothing. Setup/fresh-start interruption after installing the stop file but before publishing the header produces another unjournalled state with the same dead-end recovery guidance.
> - **Impact:** A routine administrative operation can render the whole store unavailable until someone manually reconstructs its layout. The keys/items may still exist, but the supported recovery interface cannot restore access.
> - **Recommendation:** Journal every multi-step encryption-layout/header transition before removing the live header, and make recovery explicitly handle those transaction types. Persist enough state to select and authenticate the intended old/new header safely. Report an unrecovered broken store as an error rather than successful “nothing to recover.”
> - **Suggested verification:** Inject failures and process-stop states before/after each header rename, then recover through the CLI to a usable store with unchanged item contents. Include setup and fresh start after the stop file is written, not only migration/rotation.
> - **References:** Repository evidence; PLAN-00008 STEP-05 and its header-swap risk/mitigation.

### Medium

#### REV-00002-MED-01 — A partially written rewrite plan prevents both finish and undo

> [!warning] Changes requested

- **Confidence / Category:** High / Reliability.
- **Location:** `crates/passalong-core/src/encryption/rewrite.rs:115-125` — `lock`; `rewrite.rs:89-103,235,297` — plan parsing and recovery entry points.
- **What:** `plan.json` is created in place and then written. Termination after the create, or a short/failed write, leaves an empty/truncated file. `read_plan` returns an error rather than `None`; both `finish` and `undo` propagate it. Thus even an interruption before any source item moves leaves a lock the supported recovery command cannot clear.
- **Evidence:**
  ```rust
  let mut writer = fs.open_write(&path).await?;
  writer.write_all(&json).await
      .map_err(|err| FsError::from_io(&path, err))?;
  ```
  The plan-less cleanup at `rewrite.rs:297-303` is never reached for an existing malformed plan. `FaultyFs::open_write` fails before opening and does not wrap `AsyncWrite` (`testing.rs:289-292`), so the exhaustive call-failure tests omit this state.
- **Why it matters:** Interrupted migrations and rotations remain blocked even when their items are untouched, contrary to the recoverable-operation contract. This is distinct from the unjournalled header transactions in MAJ-04: it affects the existing rewrite journal itself.
- **Suggested fix:** Publish the initial plan atomically from a complete staged file, and make pre-plan recovery distinguish safe abandoned initialization from evidence of a later transaction. Do not blindly delete an invalid journal if source data may already have moved.
- **Suggested verification:** Simulate successful file creation followed by a failed write, truncated JSON, and termination immediately after open. Verify finish/undo provide a safe supported recovery without manual filesystem edits.

#### REV-00002-MED-02 — Header size and mtime are not a reliable key-generation identity

> [!warning] Changes requested

- **Confidence / Category:** High / Correctness.
- **Location:** `crates/passalong-core/src/store/fs_store.rs:117-131` — `check_key`; `crates/passalong-ssh/src/sftp_fs.rs:233-239` — `stat`.
- **What:** Equal `(size, modified)` skips reading the key id indefinitely. Headers with the same KDF settings have the same length: key ids, salts, nonces, and wrapped data keys are fixed-width hex. SFTP v3 modification times have whole-second precision; they are not a content version.
- **Evidence:**
  ```rust
  let seen = (found.size, found.modified);
  if *guard.lock().unwrap_or_else(PoisonError::into_inner) == seen {
      return Ok(());
  }
  ```
- **Failure scenario / Impact:** Two rapid header generations through the public encryption API, or a backend preserving the timestamp during replacement, can leave an already-open K1 store with exactly the same cached stat tuple after a K2 rotation. If it did not observe the transient `.rewrite`, subsequent sends are accepted under K1 instead of forcing a rejoin. This is a post-rotation guard failure even when no upload was in flight during rotation.
- **Suggested fix:** Verify the actual key id/generation for safety decisions, or use a generation identifier whose change is guaranteed by the protocol. Treat stat data only as a performance hint, not proof of key identity.
- **Suggested verification:** Replace the header with a different key while preserving size and mtime in a filesystem double. The old store must reject put/delete/list_ids. Add a second-resolution SFTP test rather than relying on LocalFs subsecond timestamps.
- **References:** IETF's SFTP v3 draft, §5: `atime` and `mtime` are seconds since the Unix epoch. The application consequence follows from the repository code.

#### REV-00002-MED-03 — A running cache refresher keeps its old key identity after rejoining

> [!warning] Changes requested

- **Confidence / Category:** High / Correctness.
- **Location:** `crates/passalong-core/src/cache.rs:68-83` — new key-aware identity; `cache.rs:191-211,242-262` — `refresh_loop`/`refresh_once`; `crates/passalong-cli/src/commands/serve.rs:103-117` — initial identity capture.
- **What:** The identity includes the key loaded at `serve` startup, but remains an immutable string for the lifetime of its cache task. Reopening the store loads the updated key file; it does not replace the cache's `store` identity.
- **Evidence:**
  ```rust
  Some(cache) => cache.refresh(connected.as_ref(), now).await,
  None => ListCache::read(connected.as_ref(), identity.to_owned(), now)
      .await
      .map(|fresh| *cache = Some(fresh)),
  ```
- **Failure scenario / Impact:** Leave `serve` running, rotate the store, then join with the new words on that machine. Its refreshed connection reads K2 items but keeps saving them under the K1 identity. New `list` invocations correctly expect K2 and reject this cache. Even a correct cache created by a one-shot list is overwritten by the background task at the next interval, so caching remains broken until `serve` restarts. Migration from plaintext has the same issue.
- **Suggested fix:** Recompute the identity from the successfully opened store's key generation and rebuild/relabel the cache only after validating that generation; do not keep the startup key suffix across reconnections.
- **Suggested verification:** Keep `refresh_loop` alive across migration and rotation/rejoin, then assert its saved identity matches the new key and a one-shot list consumes the cache without reconnecting.

#### REV-00002-MED-04 — A corrupt encrypted file can block all later pull-mode work forever

> [!warning] Changes requested

- **Confidence / Category:** High / Reliability.
- **Location:** `crates/passalong-core/src/crypto/stream.rs:243-256` — authenticated read failures; `crates/passalong-core/src/serve/pull.rs:112-120,191-199,209-224` — application/download failure classification.
- **What:** The new decryption reader exposes authentication/truncation failures as `io::ErrorKind::InvalidData`. Download helpers wrap these in `DownloadError::Read`; pull mode treats every such read error as a retryable backend failure. A corrupt content file with valid metadata is never marked handled, and the poll returns before later items are processed.
- **Evidence:**
  ```rust
  Err(DownloadError::Read(err)) => return Err(read_failed(meta, &err)),
  ```
- **Failure scenario / Impact:** A foreign file has valid sealed metadata but a flipped ciphertext byte. It is the oldest unseen file. Every poll reaches the same file, fails authentication, reconnects, and retries it. New valid files and clipboard updates after it are starved indefinitely. The five-minute grace does not help: full-length ciphertext authentication errors occur inside the reader, bypassing `FsStore::unreadable` entirely.
- **Suggested fix:** Preserve the distinction between permanent authenticated-content damage, temporary incomplete synchronization, and transport errors. Defer incomplete items without starving independent work, and skip/quarantine/report persistently corrupt items according to the existing pull policy.
- **Suggested verification:** Corrupt one encrypted foreign file while leaving its metadata valid, append a valid file and clipboard item, and poll across the grace interval. Confirm valid items still arrive, damaged plaintext is never applied, and a genuine network read failure remains retryable.

#### REV-00002-MED-05 — Symlinked config directories bypass the key-file Git safeguard

> [!warning] Changes requested

- **Confidence / Category:** High / Security.
- **Location:** `crates/passalong-core/src/encryption/key_file.rs:156-167,260-277` — `work_tree`, `check_git`, and `save_key_file`.
- **What:** Worktree discovery walks the lexical parents of the configured path without resolving symlinked ancestors. The actual write follows those ancestors, so discovery and storage can refer to different trees.
- **Evidence:**
  ```rust
  path.parent()?
      .ancestors()
      .find(|dir| dir.join(".git").exists())
  ```
- **Failure scenario / Trust boundary:** A user manages dotfiles in Git and symlinks `~/.config/passalong` to a subdirectory of that worktree. The lexical config ancestors contain no `.git`, so `check_git` returns success without invoking Git. Setup writes `store.key` into the real, non-ignored repository directory. A subsequent ordinary dotfiles commit can publish the data key.
- **Impact:** The accidental-VCS-disclosure safeguard advertised for key files does not cover a common configuration-directory layout. Someone obtaining that key and the encrypted store can decrypt its items.
- **Suggested fix:** Resolve existing ancestors to their physical location before worktree/ignore checks, including when the final key file or some directories do not exist yet. Revalidate the actual destination for save/load, or explicitly refuse unsupported symlinked destinations. If a key was already published, rotate it and address the exposed copies; changing only the words is insufficient.
- **Suggested verification:** Use a temporary repository with a subdirectory reached through an external symlink. Saving must be refused without an ignore rule and allowed only when the actual destination is ignored. Also cover symlinked read paths and missing final components.
- **References:** Repository evidence; PLAN-00008 AC-07 and the documented Git worktree rule.

### Low

#### REV-00002-LOW-01 — The old-client prune compatibility check only tests an invalid flag

- **Confidence / Category:** High / Testing.
- **Location:** `crates/passalong-cli/tests/compat_v016.rs:143-152` — `assert_old_client_is_refused`.
- **What / Why:** The harness invokes v0.1.6 as `prune --older-than 1m --force`, then merely asserts a nonzero exit. The tagged v0.1.6 CLI defines `prune --yes`, not `--force`. That invocation fails argument parsing before it accesses any store, including a healthy plaintext store. It therefore supplies no evidence that the old prune path respects the stop file.
- **Suggested fix:** Invoke the supported `--yes` flag and assert a storage-level refusal rather than a usage error. Add a positive control using the same arguments against a plaintext store with an eligible item, so the compatibility assertion cannot pass without exercising prune.
- **Suggested verification:** The positive control must actually prune; the encrypted-layout case must reach the command and reject the store without modifying it.

#### REV-00002-LOW-02 — Release notes claim a cloud-sync check that the acceptance record says was not run

- **Confidence / Category:** High / Documentation.
- **Location:** `docs/release/v0.2.0.md:152` — Known limitations.
- **What / Why:** “Synced folders were checked by hand with one provider only” overstates verification. The release's PLAN-00008 work log at line 1071 leaves AC-21 to the user. The subsequent documentation commit explicitly records that the user deferred it for lack of a setup (`docs/plans/00008-V0_1_7_Encryption_At_Rest.md:1110` at local HEAD; `docs/backlog.md:10-15`). Users deciding whether to put sensitive data in a synced store are given the wrong evidence status.
- **Suggested fix:** Correct the maintained release notes and any published release description to say no provider was tested and identify the deferred check. Do not rewrite the historical tag. If a check was actually performed later, provide its provider, date, two-device procedure, and results before making that claim.
- **Suggested verification:** Reconcile release-facing testing statements with the accepted deferral and any subsequently supplied execution evidence. This finding is about inaccurate documentation, not a second finding for the already-deferred AC-21 work.

## Open questions

- **Concurrent administrative commands:** `change_words` unwraps through `join`, then replaces the header without acquiring the rewrite lock (`encryption/admin.rs:235-243`). The rewrite APIs also inspect state before taking their lock, and recovery has no separate recovery-owner exclusion. The locking redesign requested in MAJ-01 should explicitly define and test word-change-versus-rotation and simultaneous recovery behavior; no execution evidence for those interleavings was supplied.
- **Cloud-sync deployment conditions:** A local exclusive mkdir coordinates one filesystem, not two independently synced replicas. Historical IDEA-00001 already records this risk, and AC-21 is explicitly deferred. What enforced operating constraints will make destructive rewrites safe on those replicas? A documented single-admin/quiescence procedure plus multi-device synchronization tests would resolve the deployment question. This known design risk is not counted again as a new finding.
- **Crash durability beyond API failures:** The supplied fault matrix fails filesystem calls before execution; it does not model power loss, successful remote operations whose acknowledgements are lost, or failures midway through recursive deletion. Define the required durability boundary and supply crash/restart evidence before treating that matrix as proof against every interruption.

## Review coverage

### Files and areas reviewed

The 57 changed files counted in front matter cover the implementation and relevant contracts:

- **Core crypto:** `crypto/{mod,stream,words}.rs`: randomness, key derivation/wrapping, metadata binding, chunk/final-record authentication, bounds, secret wrappers, and tests.
- **Core encryption:** all six files in `encryption/`: `mod.rs`, `admin.rs`, `header.rs`, `key_file.rs`, `open.rs`, and `rewrite.rs`, including their unit/fault-injection tests.
- **Core storage and integration:** `fs/{mod,local}.rs`, `store/{mod,factory,fs_store}.rs`, `model.rs`, `config.rs`, `cache.rs`, `serve/{pull,upload}.rs`, `testing.rs`, `lib.rs`, and `tests/serve_local.rs`.
- **CLI:** `app.rs`, `cli.rs`, `prompt.rs`, `list_cache.rs`, `commands/{check,encrypt,init,mod,prune,serve}.rs`, and `tests/{cli_local_backend,cli_ssh_backend,compat_v016}.rs`.
- **SSH:** `src/{backend,connect,lib,sftp_fs}.rs` and `tests/sftp_docker.rs`, including the filesystem's stat, rename, create, and removal semantics.
- **Build/release records:** workspace `Cargo.toml`, `Cargo.lock`, CLI/core manifests, `.gitignore`, `justfile`, root/core `NOTICE`, README, CHANGELOG, PLAN-00008, and the changed architecture, configuration, usage, developer-guide, backlog, and v0.2.0 release documents.
- **Additional unchanged context, not added to the changed-file count:** root AGENTS and contributing rules, review-directory instructions and the earlier review, `.pre-commit-config.yaml`, `deny.toml`, CI/release workflows, and `download.rs`'s verification/error flow. The tagged v0.1.6 CLI was read to verify the compatibility-test arguments.

**Boundaries:** This is a comprehensive review of the release change, not an audit of every unchanged subsystem. The four historical idea revisions in the diff were not independently re-reviewed as deliverables; the approved plan is the acceptance baseline. The vendored word list was assessed through its embedding/selection logic, validation tests, attribution, and recorded checksum, not by independently rehashing or manually auditing all 7,776 entries. These five files are not included in `files_reviewed`. No credential files, private keys, or `.env` files were read.

### Checks performed

- Resolved both annotated tags to full commits, inspected the release's complete changed-file/stat inventory, and inspected the relevant implementation/test/tooling/documentation patches and surrounding code. Confirmed application sources in the local worktree match the tag.
- Checked staged/unstaged state and the post-release documentation diff so acceptance evidence was not confused with code in the tagged release.
- Traced key generation, sealed writes, file publication, rewrite snapshots/cleanup, header swaps, key loading, cache reopening, pull retries, and CLI error handling across callers.
- Cross-checked the new API defaults and `#[non_exhaustive]` changes against release notes and out-of-crate call sites.
- Inspected tests for tampering, refusal cases, old-client compatibility, local/SFTP integration, and operation-level fault injection; identified concrete missing failure states in the findings rather than treating coverage percentage as sufficient evidence.
- Read the SFTP v3 attribute definition to confirm timestamp granularity.
- Queried PR #10 metadata and checks read-only. Both listed CI runs report successful Linux, macOS, Xvfb, and Android jobs. Coverage percentages and benchmark numbers are repository-reported results, not measurements made in this review.
- Applied correctness, security/privacy, data integrity, reliability, performance, API compatibility, testing, and release-documentation lenses. No separate evidence-backed material performance regression was found. HTTP/web authorization and SQL migration concerns are not applicable to this filesystem-encryption change; cosmetic style preferences were excluded.

### Checks not performed

- Tests, builds, linters, formatters, scanners, hooks, package managers, containers, migrations, and all project/runtime execution were **not run** by this read-only agent.
- Findings' proposed regressions were validated by static control-flow/state reasoning, not by executing reproductions.
- No independent word-list checksum, crypto known-answer execution, benchmark, provider test, power-loss test, or published-binary inspection was performed.
- CI job statuses were retrieved, but full job logs and raw coverage artifacts were not audited. Cloud-provider behavior remains explicitly unverified, as recorded in the later acceptance deferral.
- The native skill tool was not exposed in this session. Repository scripts were not executed; report numbering used the review agent's permitted atomic directory lock.

## Positive notes

- The content format binds record order and finality into authenticated nonces, derives per-item content keys from random salts, and checks the metadata-recorded salt. The tests cover boundary lengths, record reordering, extension, truncation, and content swaps.
- Data keys and words have redacted Debug implementations, key files are created with private permissions through exclusive temporary files, and production key/word/nonces use OS randomness rather than the non-cryptographic staging RNG.
- The completed stop-file layout rejects new old-client sends before their content is written. The compatibility harness includes marker scans and an actual encrypted-store fixture; LOW-01 is a localized weakness in its prune case, not a dismissal of the whole harness.
- The rewrite engine keeps source items until new copies are read back and verified. Operation-level fault injection is a valuable foundation, although it does not substitute for partial-write and concurrency tests.
- The version bump communicates the intended library break, while default trait methods preserve ordinary external implementations for the additive interfaces.

## External references

1. **SSH File Transfer Protocol, draft-ietf-secsh-filexfer-02**, T. Ylonen and S. Lehtinen, IETF Internet-Draft, October 2001. Accessed 2026-09-15. §4 identifies protocol version 3; §5 defines modification times in whole seconds. This is the historical v3 protocol draft, not a claim that it is a current Standards Track RFC. <https://www.ietf.org/archive/id/draft-ietf-secsh-filexfer-02.txt>
2. **PR #10**, joelee/passalong on GitHub, metadata and check status accessed 2026-09-15 using `gh`. Confirms the merged release branch/head and merge commit. <https://github.com/joelee/passalong/pull/10>
3. **CI runs 34997376939 and 34997645834**, joelee/passalong, GitHub Actions. Check statuses accessed 2026-09-15; run publication timestamps were not separately retrieved. Both show passing Linux, macOS, Xvfb, and Android jobs. <https://github.com/joelee/passalong/actions/runs/34997376939> and <https://github.com/joelee/passalong/actions/runs/34997645834>.

## Recommended next actions

1. **Preserve data first.** Keep backups of existing stores and the necessary keys; avoid migration/rotation during active sends until the writer-fencing issue is fixed. Do not clear recovery directories indiscriminately: they may hold the only source copies or usable header.
2. **Fix the four blocking state-transition issues.** Introduce coordinated generation-aware publication, conservative layout classification, complete legacy-plaintext accounting, and recoverable header transactions. Include administrative concurrency in the protocol design.
3. **Make the journal and its verification realistic.** Atomically publish the plan and extend fault injection to returned readers/writers, partial writes, cancellation points, and interruption during recovery itself.
4. **Correct the remaining integration cases.** Test same-stat header replacement, cache identity changes in a live process, corrupt encrypted files in pull mode, and physical-path Git validation for key files.
5. **Correct the compatibility command and release evidence.** Run the valid old-client prune path with a positive control; remove the unsupported cloud-provider testing claim and retain the explicit deferred-check warning.
6. **Have the implementation owner run the gates.** Supply fresh format/lint/test/build/coverage, compatibility, SFTP, desktop, and relevant platform results, with the new regression cases. Re-review the corrective diff and the release upgrade guidance before calling encryption hardened.

## Handoff

The release tag already exists and PR #10 is merged. Treat this as a corrective-release review: do not move or overwrite the historical tag, and do not represent the current encryption implementation as cleared by this report. Hand the blocking findings to the implementation owner for a fix plan and a new reviewed release. If artifacts have already been published, assess affected users and communicate migration/rotation precautions and the plaintext-leftover risk.

This is an initial review of v0.2.0, not a replacement of [[00001-v0_1_2_Release_Review]]. The older report concerns a different release; its ambiguous-id and download-naming findings were not re-raised here. The existing AC-21 cloud-sync deferral is acknowledged rather than counted as a newly discovered test gap. Only this Markdown report was created; application code, tests, configuration, plans, backlog, release records, and earlier reviews were not modified.

## Confidence

**Medium.** The relevant release implementation and failure paths were available locally, and the reported defects follow from specific state transitions and caller contracts. The principal uncertainty is their runtime frequency on real SFTP and synced-folder deployments: reproductions, crash tests, and provider tests were not executed, and CI success alone does not establish those behaviors.
