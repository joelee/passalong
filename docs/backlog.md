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
- **Pull mode and clock skew.** Pull mode finds new items by comparing ids,
  which start with the sender's clock, so an item from a device whose clock
  runs behind can be missed. Remembering the ids already seen, instead of a
  position, would remove the dependence on clocks.
- **Protected `release` environment.** Create a `release` environment with
  a required reviewer and only `v*` tags allowed, move `CARGO_REGISTRY_TOKEN`
  into it, delete the repository secret, and add `environment: release` to
  the release workflow's crates.io job, so publishing waits for approval.
  GitHub's settings page returned HTTP 500 when this was tried on
  2026-09-13. crates.io trusted publishing from GitHub Actions is an
  alternative that removes the stored token.
- **Dependabot alerts and security updates.** Enable both under Settings,
  Advanced Security. The page returned HTTP 500 when this was tried on
  2026-09-13. Meanwhile `cargo deny` checks advisories on every CI run.
