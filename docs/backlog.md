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
- **Targeted metadata reads for ambiguous ids** (Code Review 00001,
  REV-00001-MED-01). When a prefix matches several items, the choice prompt
  calls `Store::list`, which reads every item's `meta.json`: one SFTP read
  per stored item. Add a `Store::get_meta` for the candidates alone, with a
  `FaultyFs` call-count test like the one for `list_after`.
- **Atomic download names** (Code Review 00001, REV-00001-LOW-01). Choosing
  a free name in the download directory and writing the file are separate
  steps, so two downloads of the same name at the same moment can pick the
  same name and one file is lost. Reserve the part file with an exclusive
  create and try the next number on a collision.
