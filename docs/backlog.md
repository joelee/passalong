# Backlog

Future work not covered by an active plan. Completed items are removed.

## @joelee road map for next releases

### v0.1.7
- **Encryption at rest.** Implement a optional client secret key to encrypt the content stored on the server and `passalong encrypt` prompting password prompts for old password and new password (twice) to encrypt or re-encrypt (to change secret key)
  - Design questions for its plan: which metadata is encrypted, the content hash in the item id, deduplication, pull mode, and sharing the key across devices.

### v0.2.0
- **Windows support**
- **Amazon S3 support** for `serve`


## Agent suggested next steps

### Features

- **`choose` connects only when needed.** Since v0.1.6 `choose` shows the
  list cache at once, but it still connects first, because `g`, `d`, `R`,
  and the actions need the store. Connecting when one of them is first
  used would open the list without waiting for SSH.
- **Parallel metadata reads.** `list --nocache`, `R`, and a listing
  without a fresh cache still read each item's `meta.json` in turn, one
  SFTP round trip per item. Reading them concurrently would cut the wait
  over slow links.
- **ssh-agent authentication.** Use keys held by an agent instead of an
  identity file.
- **Connection reuse.** One-shot commands open a new SSH connection each
  time; reuse or multiplex connections.
- **More backends.** S3 and HTTP API backends behind the existing registry.
  Their config sections must be added to the core configuration module.
- **Android client and desktop GUI** on top of `passalong-core` and
  `passalong-ssh`.
- **Windows support.**
