---
title: "Idea 00001 r05: Encryption At Rest"
aliases:
  - "Idea 00001"
tags:
  - idea
  - discovery
  - security
  - claude-code
type: idea-report
idea_id: "IDEA-00001"
revision: 5
revision_kind: final
status: accepted
created: 2026-09-15
updated: 2026-09-15
analysed_at: "2026-09-15T17:25:45Z"
agent: "Claude Code"
model: "anthropic/claude-opus-5"
triggered_by: user
previous_revision: "[[00001-Encryption_At_Rest-r04]]"
root_revision: "[[00001-Encryption_At_Rest-r01]]"
related:
  - "[[00001-Encryption_At_Rest-r04]]"
  - "docs/plans/00008-V0_1_7_Encryption_At_Rest.md"
  - "docs/release/v0.2.0.md"
  - "docs/architecture.md"
  - "docs/backlog.md"
idea_kind: feature
maturity: decision-ready
recommendation: proceed-to-experiment
confidence: medium
fact_check_status: partial
web_research_used: true
actionable_risks: 1
risks:
  critical: 0
  major: 0
  medium: 1
  low: 0
  info: 3
  total: 4
open_questions:
  blocking: 0
  non_blocking: 1
sources:
  - "https://www.rfc-editor.org/rfc/rfc9106.html"
  - "https://nvlpubs.nist.gov/nistpubs/Legacy/SP/nistspecialpublication800-38d.pdf"
  - "https://www.eff.org/dice"
  - "https://www.eff.org/files/2016/07/18/eff_large_wordlist.txt"
  - "https://www.eff.org/copyright"
---

# Idea 00001 r05: Encryption At Rest

> [!abstract] Recommendation: `proceed-to-experiment`; status `accepted`
> The user accepted IDEA-00001 on 2026-09-15, after PLAN-00008 delivered it
> as passalong v0.2.0 (release commit `f3625c0`, merged as PR #10, tagged
> `v0.2.0`). The condition set for acceptance, that v0.1.6 clients cannot
> write plaintext into an encrypted store, was proved by `just test-compat`
> in PLAN-00008 STEP-01 and STEP-05. Every r04 finding but one is resolved
> by the delivered design and its tests. The one left, sync behaviour on a
> cloud-synced folder (PLAN-00008 AC-21), was deferred by the user to the
> backlog because no suitable setup was at hand.

## 1. Seed Idea

### Original proposition

Unchanged from r01 to r04: optional client-side encryption of everything a
store holds, a per-device key file, AES-256, encrypted metadata, set-up
from `init` and `encrypt`, recoverable rewrites, and a way for devices to
take the key.

### Motivation and timing

Whoever controlled the storage could read every item. v0.2.0 closes that
for stores that opt in.

## 2. Context and Intent

| Field | Detail |
|---|---|
| Intended outcome | The storage reveals no content, names, types, previews, or device names; a lost device can be locked out |
| Target users or beneficiaries | Users of SSH servers they do not fully trust, and of `local` stores in synced folders |
| Current stage | Delivered in v0.2.0; accepted |
| Known constraints | As in r04 |
| Non-negotiables | As in r04, all met |
| Related project context | PLAN-00008; `docs/architecture.md` Encrypted stores and Security model |

## 3. Problem or Opportunity

As in r04. The delivered design removes it for encrypted stores: the
storage now sees only creation times, item counts, approximate sizes,
which items share content, and access times.

## 4. Proposed Feature or Concept

### User-visible outcome

As delivered: `passalong encrypt` (set-up, migrate or fresh start, new
words), `encrypt --join`, `encrypt --rotate`, `encrypt --recover`,
`prune --plain`, encryption in `init` and `check`, and `serve` working on
encrypted stores.

### Principal use cases

As in r04, all implemented.

### Important edge cases

As in r04; all covered by tests except sync through a real cloud provider
(IDEA-00001-R05-MED-01).

## 5. Desired Outcomes and Success Measures

| Outcome | Measure | Baseline | Target | Result |
|---|---|---:|---:|---|
| Storage reveals no content or metadata | Plaintext fields readable from an encrypted store | All | Id times, counts, sizes | Met: leak-scan test over text, file, and image items |
| Short clipboard text not guessable from ids | Confirmable candidates | 10^6 in < 1 s | None without the key | Met: keyed content keys |
| Old, keyless, rotated-out clients cannot write | Items written | Possible | 0 | Met: `just test-compat`, open-table and guard tests |
| Passphrase strength | Entropy | n/a | ≈ 77.5 bits | Met: six EFF words |
| Speed kept | `list --nocache`, 10 and 100 items | 111 ms, 135 ms | within 10 % | Met: 115 ms, 136 ms; cached list 2 ms |
| Passphrase change is cheap | Items rewritten | n/a | 0 | Met |
| Rewrites are safe | Items lost after any cut and recovery | n/a | 0 | Met: fault injection at every call |
| Rotation locks out old keys | Operations accepted with an old key | n/a | 0 | Met |

## 6. Scope and Non-goals

### In scope

As in r04, all delivered in v0.2.0.

### Out of scope

As in r04: hiding times, counts, and sizes; decrypting back to plaintext;
configurable Argon2; typed passphrases; keychains and hardware keys;
Windows, GUI, Android, and S3.

## 7. Users and Stakeholders

| Stakeholder | Need or incentive | Impact | Involvement needed |
|---|---|---|---|
| Maintainer (user) | Safe v0.2.0 | Delivered | The deferred sync check |
| SSH-server users | Privacy from the operator | Delivered | Upgrade every device, keep the words |
| Cloud-folder users | Privacy from the provider | Delivered, sync unchecked by hand | Report sync problems |
| Users of v0.1.6 or earlier | Keep working | Refused by encrypted stores, as accepted | Upgrade |

## 8. Assumption Ledger

| ID | Statement | Classification | Impact if wrong | Evidence status | Confidence | Cheapest test |
|---|---|---|---|---|---|---|
| A1 | Users record the six words | Desirability | Lost store | Typed-back confirmation shipped | Medium | Support reports |
| A2 | Argon2id at 64 MiB is under 1 s | Feasibility | Slow join | Verified: 106 ms, release build | High | None |
| A3 | A file named `items` stops v0.1.6 | Feasibility | Leak | Verified: `just test-compat` | High | None |
| A4 | Directory rename is atomic on SFTP and local disks | Feasibility | Partial items | Relied on and tested | High | None |
| A5 | Cloud-sync clients deliver files eventually but not atomically | Feasibility | Spurious errors | Unverified; handled by "not complete yet" | Low | The deferred AC-21 check |
| A6 | Header re-checks cost little | Viability | Slow sends | Verified: `clipboard --stdin` 110-115 ms either way | High | None |
| A7 | Short prefixes keep working | Desirability | Usability | Verified | High | None |

## 9. Research and Fact Check

| Claim | Finding | Status | Evidence | Checked on |
|---|---|---|---|---|
| r04 claims | As recorded in r04 | Verified | r04 §9 | 2026-09-15 |
| v0.1.6 cannot write into an encrypted store | Every item command fails; nothing written | Verified | `just test-compat`, PLAN-00008 STEP-01 and STEP-05 | 2026-09-15 |
| Sealed format rejects tampering | Truncation, reordering, duplication, extension, swaps, bit flips all refused | Verified | crypto and sealed-store tests | 2026-09-15 |
| No item lost by an interrupted rewrite | Every cut recovered both ways | Verified | rewrite fault-injection tests | 2026-09-15 |
| Encryption keeps list speed | Within 4 % | Verified | v0.2.0 release notes, Timings | 2026-09-15 |
| Cloud-sync clients sync encrypted stores without spurious errors | Not tested | Unverified | PLAN-00008 AC-21 deferred | — |

### Evidence limitations

No cloud-sync provider was tested; see IDEA-00001-R05-MED-01.

## 10. Challenge Review

### Strongest version of the idea

As in r04, now shipped.

### Formal findings

#### IDEA-00001-R05-MED-01: Sync through a real cloud provider is unchecked

> [!warning] Medium
> - **Confidence:** Medium
> - **Category:** Reliability
> - **Evidence:** PLAN-00008 AC-21, a two-device check on one cloud-synced folder, was deferred by the user on 2026-09-15 for lack of a setup. Carried from IDEA-00001-R04-MED-03.
> - **Failure scenario:** A provider syncs `meta.json` long before `content`, or keeps a conflicted copy of `encryption/header.json`, so items look incomplete for longer than five minutes or a device cannot unwrap the header.
> - **Impact:** Spurious corrupt items or a refused store in synced folders.
> - **Mitigation or test:** Run the backlog's "Check encrypted stores on a cloud-synced folder" item; the backlog's "Cloud-sync hardening" item covers the follow-up.
> - **References:** `docs/backlog.md`; PLAN-00008 AC-21.

#### IDEA-00001-R05-INFO-01: Some metadata stays visible

> [!info] Info
> Creation times, item counts, approximate sizes, shared content, and
> access times; documented in the security model and release notes.

#### IDEA-00001-R05-INFO-02: Each client's list cache holds decrypted metadata

> [!info] Info
> Mode 0600 in the user's state folder; documented.

#### IDEA-00001-R05-INFO-03: No new dependencies were needed

> [!info] Info
> `Cargo.lock` gained no package names; the EFF list ships with its
> attribution in `NOTICE`.

### Failure modes and unintended consequences

As in r04: losing the words and every key file loses the store; every
rotation needs every device to join again.

### Conditions to revise, park, or reject

Revisit if the deferred sync check shows encrypted stores unreliable in a
synced folder.

## 11. Options and Trade-offs

As in r04; option A was delivered.

## 12. Recommended Concept

As delivered in v0.2.0 and described in `docs/architecture.md`, Encrypted
stores.

## 13. Dependencies, Risks, and Safeguards

| Item | Type | Likelihood | Impact | Mitigation, test, or owner |
|---|---|---|---|---|
| Cloud sync (MED-01) | Risk | Medium | Medium | Backlog check (user) |

## 14. Highest-value Next Experiment

- **Hypothesis:** An encrypted store in one cloud-synced folder works on two devices.
- **Method:** Two devices share a Dropbox, Google Drive, or OneDrive folder as a `local` store; encrypt on one, join on the other, send text and files both ways, wait for sync.
- **Inputs or participants:** The maintainer, two devices, one provider.
- **Success threshold:** Every item lists and loads on both devices; nothing reported corrupt once sync completes.
- **Failure threshold:** A corrupt item after sync, or a conflicted header copy.
- **Expected effort:** An hour.
- **Risks and safeguards:** A throwaway store.
- **Evidence to capture:** Commands, outputs, provider, timings.
- **Decision enabled:** Close IDEA-00001-R05-MED-01, or plan cloud-sync hardening.

## 15. Open Questions and Loose Ends

### Blocking

None.

### Important but non-blocking

- [ ] Does an encrypted store sync cleanly through a real cloud provider?
      (PLAN-00008 AC-21, on the backlog.)

### Later considerations

- [ ] Key import for GUI and Android, several stores per machine, stronger
      Argon2 settings: on the backlog.

## 16. Feedback Incorporated

| Feedback or prior finding | Disposition | Change in this revision | Rationale |
|---|---|---|---|
| User accepts IDEA-00001 (2026-09-15) | accepted | Status `accepted`, kind `final` | Explicit user decision |
| User defers AC-21 to the backlog | accepted | MED-01 stays open; backlog item added | No setup available |
| IDEA-00001-R04-MAJ-01: old and keyless clients | resolved | — | `just test-compat`; open-table tests (STEP-01, STEP-05) |
| IDEA-00001-R04-MED-01: AEAD construction | resolved | — | Tamper suite (STEP-02, STEP-04) |
| IDEA-00001-R04-MED-02: migration and rotation | resolved | — | Fault injection; pull reset; cache identity (STEP-07, STEP-08) |
| IDEA-00001-R04-MED-03: cloud sync | still-open | Now IDEA-00001-R05-MED-01 | Deferred check |
| IDEA-00001-R04-MED-04: fresh-start plaintext | resolved | — | `plain/`, warning, `prune --plain` (STEP-06) |
| IDEA-00001-R04-MED-05: old-key writers after rotation | resolved | — | Header guard (STEP-05) |
| IDEA-00001-R04-LOW-01: word-list attribution | resolved | — | NOTICE files and README (STEP-02, STEP-10) |
| IDEA-00001-R04-INFO-01 to INFO-03 | superseded | Now IDEA-00001-R05-INFO-01 to INFO-03 | Still relevant |

## 17. Decision Log

| Date | Decision or change | Rationale | Owner |
|---|---|---|---|
| 2026-09-15 | Decisions of r02 to r04 | See r04 §17 | User |
| 2026-09-15 | Release as v0.2.0, a breaking release; Windows and S3 to v0.2.1 | Layout and API break | User |
| 2026-09-15 | Accept IDEA-00001 | Delivered and verified in v0.2.0 | User |
| 2026-09-15 | Defer the cloud-sync check (AC-21) to the backlog | No setup available | User |

## 18. Recommended Next Actions

1. Run the deferred cloud-sync check when a setup is available.
2. Pick up the backlog's cloud-sync hardening if it shows problems.

## 19. Revision History

| Revision | Status | Kind | Supersedes | Summary |
|---|---|---|---|---|
| r01 | draft | initial | — | User brief; front matter added |
| r02 | revised | feedback | r01 | Decisions 1–4; concept, 14 findings |
| r03 | revised | feedback | r02 | Rotation, fresh start, words, git rule, 64 MiB |
| r04 | revised | feedback | r03 | Generated words only; git check-ignore rule |
| r05 | accepted | final | r04 | Delivered in v0.2.0; accepted; cloud-sync check deferred |

## References

1. IETF. "RFC 9106: Argon2 Memory-Hard Function for Password Hashing and
   Proof-of-Work Applications." September 2021. Accessed 2026-09-15.
   https://www.rfc-editor.org/rfc/rfc9106.html
2. NIST. "SP 800-38D: Recommendation for Block Cipher Modes of Operation:
   Galois/Counter Mode (GCM) and GMAC." November 2007. Accessed 2026-09-15.
   https://nvlpubs.nist.gov/nistpubs/Legacy/SP/nistspecialpublication800-38d.pdf
3. Electronic Frontier Foundation. "EFF Dice-Generated Passphrases."
   Accessed 2026-09-15. https://www.eff.org/dice
4. Electronic Frontier Foundation. "EFF large word list." July 2016.
   Accessed 2026-09-15.
   https://www.eff.org/files/2016/07/18/eff_large_wordlist.txt
5. Electronic Frontier Foundation. "Copyright." Accessed 2026-09-15.
   https://www.eff.org/copyright

## Confidence

**Medium.** The delivered feature is backed by strong tests, including
v0.1.6 against real encrypted stores, tamper suites, and fault injection at
every filesystem call. The remaining uncertainty is behaviour through real
cloud-sync providers, which was not tested.
