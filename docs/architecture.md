# Architecture

## Crates

| Crate | Kind | Responsibility |
|---|---|---|
| `passalong-core` | library | Configuration, item model, storage traits, clipboard trait, `serve` loop, telemetry. No CLI or terminal dependencies. |
| `passalong-ssh` | library | SSH/SFTP storage backend (`russh`), host-key pinning. |
| `passalong` (in `crates/passalong-cli/`) | binary `passalong` | Argument parsing, command handlers, output formatting, and the `choose` terminal UI (ratatui). |

Future GUI and Android front-ends depend on `passalong-core` and
`passalong-ssh` only. `passalong-core` keeps the desktop clipboard behind its
`desktop` feature, so it also builds with `--no-default-features`.

```mermaid
flowchart LR
  CLI["passalong (CLI)<br/>commands and output"] --> REG["BackendRegistry"]
  CLI --> SERVE["serve loop<br/>passalong-core"]
  SERVE --> REG
  CLI --> CLIP["Clipboard trait<br/>ArboardClipboard"]
  SERVE --> CLIP
  REG --> LOCAL["FsStore over LocalFs<br/>kind = local"]
  REG --> SSH["FsStore over SftpFs<br/>passalong-ssh, kind = ssh"]
  SSH --> SERVER[("SSH server<br/>remote_path")]
```

## Command flow

Every invocation goes through the same start-up:

1. Load `./.env` if it exists, without overriding the environment.
2. Parse the command line; usage errors exit with code 2.
3. Find and validate `config.toml` (see [configuration](configuration.md)).
   `init`, which writes that file, `service-install` and `service-remove`,
   which need none, `check`, which reports a config problem as its first
   result, and the hidden clipboard holder described below run before this
   step.
4. Choose the log level and start logging to standard error. With
   `--quiet`, standard output is discarded for every command but `cat`, and
   the level is `error` unless one is set explicitly.
5. Open a correlation span, so every log line of this run shares one `op`
   id.
6. Build the backend registry (`local`, `ssh`) and run the command.

| Command | What it does after start-up |
|---|---|
| `clipboard` | Reads the clipboard or standard input, then `Store::put` |
| `file` | Streams the file into `Store::put` |
| `list` | `Store::list`, then renders a table or JSON |
| `load` | `Store::resolve`, `Store::get`, verifies SHA-256, then writes a file or the clipboard |
| `cat` | `Store::resolve`, `Store::get`, streams the content to standard output while verifying SHA-256 |
| `get` | `Store::resolve`, then `Store::get_meta`; prints fields or JSON |
| `choose` | `Store::list`, then a full-screen list (ratatui over crossterm); `d` runs `Store::delete` and `r` lists again; the chosen action runs the code of `load`, `cat`, or `get` after the terminal is restored |
| `serve` | Runs the loop below until stopped; `--daemon`, `--status`, and `--stop` manage a background copy |
| `delete` | Resolves every id first, then `Store::delete` for each |
| `prune` | `Store::list`, selects items older than `--older-than` beyond the newest `--keep`, confirms, deletes, then `Store::clean_staging` |
| `init` | Fetches the server host key without authenticating, asks you to confirm its fingerprint, writes the config file, then runs `Store::list` as a connection test |
| `check` | Loads the config, opens the backend, `Store::list_ids`, then `Store::probe_write`, printing one line per step, then reads `serve`'s pid lock |
| `service-install` | Writes a systemd user unit or launchd agent for `serve` and loads it with `systemctl --user` or `launchctl` |
| `service-remove` | Stops the service and removes its unit |

Each one-shot command opens its own connection; `serve` keeps one and
reopens it when needed.

## Item schema (version 1)

### Identifiers

Every item has an id of the form `<ts>-<key>`, for example
`6aa52107-2cf24dba5fb0`:

| Part | Meaning |
|---|---|
| `ts` | Creation time in whole seconds since the Unix epoch, 8 lowercase hex digits. Valid until 2106-02-07. |
| `key` | The content key: the first 12 lowercase hex digits of the content's SHA-256. |

Because both parts have a fixed width, plain string order is chronological:
sorting item directory names in reverse lists the newest items first,
without reading any metadata. The content key identifies identical content
regardless of when it was sent, so stores can skip re-uploads and `serve`
does not re-send text that `load` just placed on the clipboard. Users can
type a unique prefix of the content key, such as `2cf2`, instead of the
whole id.

The timestamp comes from the sending device's clock. Devices with skewed
clocks can therefore list slightly out of true send order; this affects
ordering only, never correctness or deduplication.

An id is parsed only by `ItemId::parse`, which accepts nothing but hex
digits and one `-`. That makes every id safe to use as a path component on
the server.

### `meta.json`

Each item directory holds its content and a `meta.json`:

```json
{
  "schema": 1,
  "id": "6aa52107-2cf24dba5fb0",
  "kind": "file",
  "name": "a.txt",
  "mime": "text/plain",
  "size": 5,
  "sha256": "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
  "created_at": "2026-09-12T09:53:11Z",
  "device": "box",
  "preview": null,
  "origin": "clipboard"
}
```

| Field | Meaning |
|---|---|
| `schema` | Schema version, `1` |
| `id` | The item id |
| `kind` | `text` or `file` |
| `name` | Original file name for files, `null` for text |
| `mime` | `text/plain; charset=utf-8` for text; guessed from the file extension otherwise, falling back to `application/octet-stream` |
| `size` | Content length in bytes |
| `sha256` | Full SHA-256 of the content, used to verify downloads |
| `created_at` | Creation time, equal to the id's timestamp |
| `device` | `client.device_name` of the sender |
| `preview` | For text, the first 80 characters with whitespace collapsed; `null` for files |
| `origin` | Optional. `clipboard` for a clipboard image, stored as a PNG file named `clipboard-YYYYMMDD-HHMMSS.png`; absent otherwise. Clients older than v0.1.2 ignore it and see an ordinary PNG file |

Readers ignore fields they do not know, so items written by newer clients
stay readable. A change that older readers cannot handle must increase
`schema`.

## Storage layout

The `local` and `ssh` backends share one layout, implemented once by
`FsStore` on top of the small `RemoteFs` filesystem trait:

```text
<root>/
├── items/
│   └── <id>/
│       ├── content      the item's bytes (UTF-8 for text)
│       └── meta.json    its metadata (see Item schema)
└── tmp/
    └── <random>/        staging for uploads in progress
```

Storing an item works like this:

1. Stream the content into `tmp/<random>/content`, hashing it on the way.
2. If an item with the same content key already exists, delete the staging
   directory and return the existing item. Nothing new is stored.
3. Otherwise write `meta.json` next to the content.
4. Rename `tmp/<random>` to `items/<id>` in one step. The rename never
   replaces an existing directory.
5. Remove the staging directory if anything failed.

An item directory therefore appears only when it is complete, and an
interrupted upload leaves nothing under `items/`.

`Store::list_after(id)` lists only the items newer than `id`, newest
first, reading `meta.json` for those items alone. Ids start with their
creation time, so comparing ids is enough. `Store::list_ids` returns the
ids alone, from one directory listing, which is how pull mode polls a large
store cheaply. `Store::get_meta(id)` reads one item's `meta.json`
without opening its content; the choice prompt for an ambiguous id uses it
for the candidates it shows, at most 9, instead of listing the store.
`Store::probe_write` writes a small file in `tmp/probe-<random>/` and
removes that directory, which is how `passalong check` tests write access
without storing an item. A backend without a probe reports
`WriteProbe::NotSupported`, the trait's default.

Deleting an item renames `items/<id>` to `tmp/deleted-<id>-<random>` and
then removes it, so the item disappears from every listing in one step.
If the removal fails, only a staging leftover remains. Staging directories
older than a threshold, left by interrupted uploads or deletions, are
removed by `clean_staging`, which `prune` runs.

Listing sorts the item directory names in reverse, which is newest first
because ids are time-sortable, then reads each `meta.json`. Directories that
are not ids, lack `meta.json`, or hold metadata that is unreadable or
describes a different id are skipped; corrupt ones are logged as warnings.

Users identify an item by typing at least 4 characters. Input without a `-`
matches the start of the content key, so `2cf2` finds
`6aa52107-2cf24dba5fb0`; input with a `-` matches the start of the full id.
Case and surrounding spaces are ignored. More than one match is an error
that lists the candidates.

## `serve`

`serve` runs three cooperating tasks:

- **Clipboard watcher.** Reads the clipboard every poll interval and queues
  text whose SHA-256 differs from the last text seen. Blank text is ignored.
  When the clipboard holds no text, or only the link or
  `<img>` tag a browser adds when copying an image, and
  `serve.clipboard_images` is on, it
  reads the image instead and queues it when its pixels differ from the last
  image seen; the uploader stores it as a PNG clipboard image. Clipboard
  access runs on a blocking thread, off the async runtime.
- **Drop watcher.** Scans the drop folder whenever the operating system
  reports a change, and at least every 5 seconds in case events are missed.
  Only events that may change files count: creating, writing, renaming, or
  removing them. Opening and reading do not, because every scan opens the
  folder and would otherwise trigger the next scan.
  A file is queued once two scans at least `file_stable_wait_ms` apart show
  the same size and modification time.
- **Pull loop** (only with `serve.pull = true`). At start-up, before
  `serve` reports ready, it records the ids of every stored item with
  `Store::list_ids`, one directory listing. Every pull interval it lists the
  ids again, forgets those no longer stored, reads `meta.json` only for ids
  it has not seen, ignores this device's own items, and handles the rest
  oldest id first: files are downloaded, verified, into
  `client.download_dir` when it exists, and the newest text or clipboard
  image is handed to the clipboard task. Like `load`, each download is written and verified in its own part file
  and only then linked into place under a free name, so it appears only when
  complete and two downloads never share a name. The clipboard task writes pulled content and marks
  it as seen, so it is not sent back. Each item is marked as handled once
  done, so a store error retries only what is left, and an item from a
  device whose clock runs behind is still applied.
- **Uploader.** Sends queued jobs one at a time. Before uploading text it
  checks whether that content key is already stored, which is how text that
  `load` just put on the clipboard is not sent back. After a file is sent,
  it moves to `sent/` or is deleted.

A failed upload is retried after 1, 2, 4 … seconds, capped at 60, and the
store is reopened before each retry so a dropped SSH connection recovers.
Local problems, such as a file that cannot be read, skip the job instead;
the drop watcher offers a skipped file again once its size or modification
time changes.
Shutdown on Ctrl-C or SIGTERM stops the watchers and abandons any upload in
progress; the atomic publish means an abandoned upload never appears under
`items/`.

## Background `serve`

`serve` holds an exclusive lock on a pid file for as long as it runs, so a
second `serve` on the same machine is refused whether it runs in the
foreground, as a daemon, or under a service manager. File locations are in
[configuration](configuration.md#serve-files).

`serve --daemon` starts a detached copy of the same binary in its own
process group, with standard error appended to the log file. The copy writes
its pid to the pid file, opens the store, and then appends a `ready` line;
the parent waits up to 5 seconds for that line and otherwise prints the end
of the log. `serve --status` reads the pid file and exits 3 when nothing is
running. `serve --stop` sends SIGTERM and waits for the pid file to be
released.

`service-install` renders a systemd user unit or a launchd agent from the
templates in `crates/passalong-cli/src/service.rs`, running the same binary's
`serve` from the home directory, and loads it with `systemctl --user` or
`launchctl`. It does not start a service while the pid lock shows a running
`serve`. `docs/service/` holds the same units with placeholder paths, and a
test keeps them identical to the templates. `service-remove` stops the
service and removes the unit.

## Clipboard on Linux

On X11 and Wayland the clipboard belongs to a running process, so text set
by a short-lived command vanishes when it exits. On Linux, `load` therefore
starts a detached helper, the same binary with a hidden command, which takes
ownership of the text and exits once another program replaces the
clipboard. `load` first checks that a clipboard is available at all, so on
a machine without one it fails instead of starting a helper that cannot
work. macOS keeps clipboard contents itself and needs no helper.

## Release pipeline

Pushing a tag `vX.Y.Z` runs `.github/workflows/release.yml`:

1. `scripts/check-release-tag.sh` rejects a tag that does not match the
   workspace version, and `cargo publish --dry-run` packages and builds all
   three crates.
2. Release binaries are built for Linux x86_64 and macOS arm64 and packed
   as `.tar.gz` files with SHA-256 checksums.
3. After a maintainer approves the `release` environment, the crates are
   published to crates.io in dependency order.
4. The GitHub release is created from `docs/release/vX.Y.Z.md` and the
   archives are attached.

The jobs that package crates remove `target/package` before the cache
action saves the build, because its cleanup fails on the unpacked crates.

CI audits dependencies with `cargo deny` (`deny.toml`) on every push and
runs the desktop clipboard tests under a virtual X server.

## Security model

- **Server identity.** The server's host key is pinned in the
  configuration. Any other key is refused with both fingerprints in the
  error. There is no trust-on-first-use and no `known_hosts` fallback.
- **Client identity.** Public-key authentication only. The optional key
  passphrase comes from the environment or `./.env` and is redacted from
  debug output.
- **Paths from the server.** Item ids are parsed strictly (hex digits and
  one `-`), so they are safe path components. File names from the server
  are reduced to their last component before `load` writes them.
- **Integrity.** `load` verifies the SHA-256 and size of the content before
  renaming it into place.
- **Logs.** Only an allow-list of fields is written. Clipboard text, file
  contents, and secrets are never logged.
- **Not covered.** The server operator can read every item. Encryption at
  rest is on the [backlog](backlog.md).

## Adding a backend

`server.kind` selects the backend through a `BackendRegistry`: a map from
kind name to an opener function, `fn(&Config) -> BackendFuture`.
`passalong-core` registers `local`, `passalong-ssh` provides `register` to
add `ssh`, and the CLI builds the registry at start-up. The core never
depends on a backend crate.

There are two ways to add a backend:

1. **File-like storage**, such as WebDAV or SMB: implement the seven-method
   `RemoteFs` trait and wrap it in `FsStore`. The layout, atomic publish,
   deduplication, listing, and id resolution come for free.
2. **Anything else**, such as an HTTP API or an S3 bucket: implement `Store`
   directly.

Then add a `[server.<kind>]` section to the configuration module in
`passalong-core` (unknown keys are rejected, so new sections must be
declared there), write an opener, and register it where the CLI builds its
registry.
