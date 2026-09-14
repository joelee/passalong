# Backlog

Future work not covered by an active plan. Completed items are removed.

## @joelee road map for next releases

### v0.1.5
- **Android cross-compile check in CI.**
- **`serve` idle CPU, for discussion.** Reported by @joelee: an idle
  `serve` uses 9.8 MB of RAM and 4.8 % CPU. Measured on 2026-09-14 with
  v0.1.4 on Linux under Wayland, it used about 134 % CPU continuously
  (38 threads), with plain text on the clipboard and an empty drop folder.
  A sandboxed `serve` with no clipboard, a local store, and an empty drop
  folder used 139 %, so the clipboard is not the cause. The busy threads are
  the tokio workers and the `notify` watcher. Likely cause:
  `drop_watcher::watch_folder` forwards every inotify event, including the
  access events that `drop_loop`'s own `scan` causes by opening the
  folder, so each scan triggers the next. To discuss: react only to create,
  modify, remove, and rename events, with a test that an idle `serve` does
  not rescan between `rescan_interval`s; then measure the remaining idle
  cost (clipboard poll every 750 ms on a blocking thread, pull every 5 s,
  the runtime's thread count) and whether a current-thread runtime suits
  `serve`.
- `passalong choose` - invoke a TUI to select 

### v0.2.0
- **Windows support**
- Add Homebrew package on `https://github.com/joelee/homebrew-oss`


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
