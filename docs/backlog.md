# Backlog

Future work not covered by an active plan. Completed items are removed.

## Agent suggested next steps

### Features

- **Image clipboard.** Send and load images. The item schema already
  records `kind` and `mime`, so no storage change is needed.
- **Pull mode for `serve`.** Apply items sent from other devices to the
  local clipboard automatically, turning passalong into two-way sync.
- **Encryption at rest.** Encrypt content before upload, for example with
  `age`, so the server operator cannot read items.
- **ssh-agent authentication.** Use keys held by an agent instead of an
  identity file.
- **Interactive disambiguation.** Let `load` offer a choice when a prefix
  matches several items.
- **Connection reuse.** One-shot commands open a new SSH connection each
  time; reuse or multiplex connections.
- **More backends.** S3 and HTTP API backends behind the existing registry.
  Their config sections must be added to the core configuration module.
- **Android client and desktop GUI** on top of `passalong-core` and
  `passalong-ssh`.
- **Windows support.**

### Engineering

- **Android cross-compile check in CI.** Build `passalong-core` and
  `passalong-ssh` for an Android target without default features.
- **`rsa` advisory exception.** `deny.toml` ignores RUSTSEC-2023-0071,
  which reaches passalong only for RSA identity files. Remove the exception
  once `rsa` publishes a constant-time release, or drop RSA identity support.
- **Duplicate dependency versions.** `cargo deny` warns about crates
  present in two versions; trim them as upstream releases allow.
