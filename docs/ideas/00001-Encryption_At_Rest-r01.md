---
title: "Idea 00001 r01: Encryption At Rest"
aliases:
  - "Idea 00001"
tags:
  - idea
  - discovery
  - brief
  - security
  - claude-code
type: idea-report
idea_id: "IDEA-00001"
revision: 1
revision_kind: initial
status: draft
created: 2026-09-15
updated: 2026-09-15
analysed_at: "2026-09-15T07:38:42Z"
agent: "user brief; front matter by Claude Code"
model: "anthropic/claude-opus-5"
triggered_by: user
previous_revision: null
root_revision: "[[00001-Encryption_At_Rest-r01]]"
related:
  - "docs/backlog.md"
  - "docs/architecture.md"
  - "crates/passalong-core/src/model.rs"
  - "crates/passalong-core/src/store/fs_store.rs"
  - "crates/passalong-core/src/serve/pull.rs"
  - "crates/passalong-cli/src/commands/init.rs"
idea_kind: feature
maturity: seed
recommendation: incomplete
confidence: low
fact_check_status: not-started
web_research_used: false
actionable_risks: 0
risks:
  critical: 0
  major: 0
  medium: 0
  low: 0
  info: 0
  total: 0
open_questions:
  blocking: 0
  non_blocking: 0
sources: []
---

Enable optional encryption on the server

User briefs:
1. A Secret Key stored on every clients, `~/.config/passalong/content_key` (suggest a better filename). Config can overwrite the key file location.
2. Use `AES-256`
3. Filename and Metadata like kind, hash of unencrypted content, and other other keys that does not impact operation process, like `id` are encrypted.
4. `passalong init` to include option to set up encryption. `init` will not run if there is already a config file.
5. Implement `passalong encrypt ` to allow setting up new encryption or changing secret key for existing encryption:
  - Ask for current secret key, if encryption is already set
  - Ask for new secret key twice
  - Use password prompt so that the key is not visible
  - Encryption process:
    - Warn the user with usual disclaimer and all the other clients will need to set to the new key,etc. User requires to acknowledge with `y`.
    - Write an encrypting in progress lock file
    - Move the existing content and meta to a working directory, e.g. `./.to-be-encrypted/`
    - Rebuild the content and meta with the new secret key
    - Check and verify
    - Remove the working directory
    - Update the secret key file 
    - remove the lock file
6. Implement recovery `passalong encrypt --recover` to restore from the working directory and remove the lock file.
7. Implement `passalong encrypt --change-key` to allow other clients to set the new key.
8. Ensure the secret key file is correctly `chmod` and `.gitignore`
