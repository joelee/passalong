# Backlog

Future work not covered by an active plan. Completed items are removed.

## Agent suggested next steps

### Features

- **Image clipboard.** Send and load images. The item schema already
  records `kind` and `mime`, so no storage change is needed.
- **Pull mode for `serve`.** Apply items sent from other devices to the
  local clipboard automatically, turning passalong into two-way sync.
- **`delete` and `prune`.** Remove items, and expire them by age or count.
- **Encryption at rest.** Encrypt content before upload, for example with
  `age`, so the server operator cannot read items.
- **`passalong init`.** Write a config interactively, and fetch and confirm
  the server's host key.
- **ssh-agent authentication.** Use keys held by an agent instead of an
  identity file.
- **Clipboard holder on Linux.** Keep text from `load` on the clipboard
  after the command exits when no clipboard manager is running, as
  `wl-copy` does with a background process.
- **Retry skipped files.** `serve` skips files it cannot read until it
  restarts; retry them when they change.
- **Interactive disambiguation.** Let `load` offer a choice when a prefix
  matches several items.
- **Connection reuse.** One-shot commands open a new SSH connection each
  time; reuse or multiplex connections.
- **`serve --daemon`.** Optional self-daemonising mode; decision D-01 chose
  service managers for v0.1.0.
- **More backends.** S3 and HTTP API backends behind the existing registry.
  Their config sections must be added to the core configuration module.
- **Android client and desktop GUI** on top of `passalong-core` and
  `passalong-ssh`.
- **Windows support.**

### Engineering

- **Supply-chain audit in CI.** Add `cargo deny` for advisories and
  licences, and review the `rsa` crate's timing side-channel advisory
  (RUSTSEC-2023-0071), which `russh`'s `rsa` feature pulls in for RSA
  identity files.
- **Android cross-compile check in CI.** Build `passalong-core` and
  `passalong-ssh` for an Android target without default features.
- **Desktop clipboard test in CI.** Run the ignored desktop test under a
  virtual X server such as Xvfb.
- **Restore the root `AGENTS.md`.** It is empty in the repository; the Rust
  rules it held survive only in Delivery Plan 00001.
