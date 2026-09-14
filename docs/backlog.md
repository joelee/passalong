# Backlog

Future work not covered by an active plan. Completed items are removed.

## @joelee road map for next releases

### v0.2.0
- **Windows support**
- Add Homebrew package on `https://github.com/joelee/homebrew-oss`


## Agent suggested next steps

### Features

- **`serve` keeps the item list cached.** Requested by @joelee: over a
  mobile connection, `list` and `choose` can take 3 to 5 seconds. Each
  one-shot command opens its own SSH connection, then `Store::list` reads
  every item's `meta.json`, one SFTP round trip per item, so the time grows
  with the store and the latency. `serve` already reads the ids at every
  pull interval and keeps a connection open. To discuss: `serve` keeps the
  metadata of every item in memory, updated from `list_ids` and `get_meta`
  for new ids and from its own sends and deletes, and answers `list`,
  `choose`, `get`, and id lookups over a local socket in its state
  directory, which only the user can open. Commands fall back to reading
  the store when `serve` is not running, or when its answer is older than
  an agreed age. Related options: reading the `meta.json` files in
  parallel, which helps without `serve`, and "Connection reuse" below.

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
