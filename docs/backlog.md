# Backlog

Future work not covered by an active plan. Completed items are removed.

## @joelee road map for next releases

### v0.2.1
- **Windows support**
- Evaluate and action Review `00002`

### v0.2.2
- **Amazon S3 support** for `serve`
- **Check encrypted stores on a cloud-synced folder** (PLAN-00008 AC-21,
  deferred on 2026-09-15 for lack of a setup). Two devices share one
  Dropbox, Google Drive, or OneDrive folder as a `local` store: encrypt on
  one, join on the other, and send text and files both ways. Every item
  should list and load on both, and none be reported corrupt once syncing
  is done. See IDEA-00001-R05-MED-01.



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
- **Cloud-sync hardening.** Encrypted stores treat a recent item that does
  not open as not complete; check each provider (Dropbox, Google Drive,
  OneDrive) for conflicted copies of `encryption/header.json` and warn.
- **Key import for GUI and Android**, for example from a QR code shown by
  another device.
- **Several stores per machine**, each with its own key file.
- **Stronger Argon2 settings** when the words next change, since the
  header records them.
