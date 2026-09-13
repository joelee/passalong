# Backlog

Future work not covered by an active plan. Completed items are removed.

## Agent suggested next steps

### Features

- **Encryption at rest.** Encrypt content before upload, for example with
  `age`, so the server operator cannot read items.
- **ssh-agent authentication.** Use keys held by an agent instead of an
  identity file.
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
