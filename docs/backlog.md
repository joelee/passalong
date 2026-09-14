# Backlog

Future work not covered by an active plan. Completed items are removed.

## @joelee road map for next releases

### v0.1.6
- Add Homebrew package on `https://github.com/joelee/homebrew-oss`
- `passalong check` on storage write test, track and report the time to write a 128 bytes (calculate bytes/second) file. The local path to the repo is `../homebrew-oss/`
- **`serve` keeps the item list cached.** (disable for local path):
  - every write will update a new timestamp file
  - `serve` will keep a cache of the item list and the timestamp file
  - `serve` will check every minute (configurable) the remote timestamp file to see if it has the latest list
  - `serve` will also check the remote timestamp file on every connection to the remote for tasks like `cat`, `load`, etc.
  - Introduce `--nocache` to the `list`. 
  - In `choose` TUI, `r` will reload from cache, `R` will reload from remote
- **Encryption at rest.** Implement a optional client secret key to encrypt the content stored on the server and `passalong encrypt` prompting password prompts for old password and new password (twice) to encrypt or re-encrypt (to change secret key)

### v0.2.0
- **Windows support**
- **Amazon S3 support** for `serve`


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
