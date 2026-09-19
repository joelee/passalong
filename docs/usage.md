# Usage

Every command reads the configuration described in
[configuration](configuration.md). Set up the server first, as described in
the [README](../README.md#server-setup).

## Global options

| Option | Meaning |
|---|---|
| `--config <PATH>` | Use this config file instead of the lookup order in [configuration](configuration.md) |
| `--log-level <LEVEL>` | `error`, `warning`, `info`, `verbose`, or `debug`; overrides `PASSALONG_LOG_LEVEL` and `client.log_level` |
| `-q`, `--quiet` | Print nothing but errors and prompts; `cat` still prints the item. Logging drops to `error` unless `--log-level` or `PASSALONG_LOG_LEVEL` sets a level |
| `-h`, `--help` | Show help |
| `-V`, `--version` | Show the version |

Global options may come before or after the subcommand.

## Using a passalong-server

Besides an SSH server or a shared folder, the store can be a workspace on a
[passalong-server](https://github.com/joelee/passalong-server): `kind =
"https"`, its URL, and, for a self-signed server, the pin its operator gives
you (see [configuration](configuration.md#serverhttps-required-when-kind--https)).
The server's operator also gives you an API key, which goes in
`server.https.api_key_file`, readable by you only; `passalong` refuses it
otherwise.

Every command below works the same with a server. What differs:

- **Limits.** A server may cap the size of one item, and every workspace
  has a quota. An item over the cap is refused before anything is sent,
  with a message that names the limit.
- **Read-only keys.** A key the operator made read-only lists and loads,
  and is refused when it sends or deletes (`FORBIDDEN_ROLE`).
- **Interrupted downloads resume** from the last byte received, up to three
  times, and the item is still checked whole before it is written.
- **The server cannot read an encrypted workspace**, nor change an item
  without its device noticing: sealing and checking happen on the device.
  A device that holds a key never sends plaintext to a workspace, whatever
  the server says about it.

## `passalong init`

Writes a config file for your SSH server and pins its host key. Run it once
per device after [preparing the server](../README.md#server-setup):

```text
$ passalong init
Server host name or address: nas.local
SSH port [22]:
User on the server [passalong]:
Private key for logging in [~/.ssh/id_ed25519]:
Storage directory on the server [/srv/passalong]:
Name for this device [laptop]:
The server at nas.local:22 presented this ssh-ed25519 host key:
  SHA256:5Si4lWKPwa0+I2wCQf3eOtcF8jWo30BWybHoXLTxABo
Compare it with the server's own key, for example by running there:
  ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub
Does the fingerprint match? [y/N] y
wrote /home/you/.config/passalong/config.toml
Add this device's public key to ~passalong/.ssh/authorized_keys on the server:
  /home/you/.ssh/id_ed25519.pub
connected: 0 items on the server
Encrypting keeps what the server stores unreadable without this store's six words.
...
Encrypt this store? [y/N] n
nothing was changed
```

The key is written only after you confirm its fingerprint. `init` writes to
`--config` when given, otherwise to the standard location, and refuses to
replace an existing file without `--force`. After writing, it logs in and
lists the server to prove the settings work. Then it looks at the store:
an empty store can be encrypted on the spot (see
[`passalong encrypt`](#passalong-encrypt)), an encrypted store is joined by
typing its six words, and a store with items gets the command to run. With
`--yes` or `--no-test`, `init` only says what to run.

| Option | Meaning |
|---|---|
| `--host <HOST>` | Server host name or address |
| `--port <PORT>` | SSH port, default 22 |
| `--user <USER>` | Login user, default `passalong` |
| `--identity-file <PATH>` | Private key, default `~/.ssh/id_ed25519` |
| `--remote-path <PATH>` | Storage directory on the server, default `/srv/passalong` |
| `--device-name <NAME>` | Name recorded on items, default the host name |
| `--host-key <KEY>` | Pin this OpenSSH key line instead of fetching one |
| `--fingerprint <SHA256:...>` | Accept the fetched key only if it has this fingerprint |
| `--yes` | Take defaults instead of asking; requires `--host-key` or `--fingerprint` |
| `--force` | Replace an existing config file |
| `--no-test` | Skip the connection test |

For scripts, `--yes` alone is refused so a key is never trusted blindly:

```sh
passalong init --host nas.local --fingerprint SHA256:5Si4lWKPwa0+I2wCQf3eOtcF8jWo30BWybHoXLTxABo --yes
```

## `passalong encrypt`

Encrypts the store, or changes its words. It needs a terminal: new words
are shown there, and words are typed there without being echoed. Words
never appear in the results or the logs.

| Store | `passalong encrypt` does |
|---|---|
| Plaintext, empty | Shows six new words, has them typed back, encrypts the store, and saves this device's key in `client.key_file` |
| Plaintext, with items | The same, after asking whether to migrate the items (the default: each is downloaded, re-encrypted, uploaded again, and checked) or to start fresh (they stay unencrypted in `plain/` until `prune --plain`) |
| Encrypted | Asks for the current words, shows new ones, and replaces the words; the key stays, nothing is re-encrypted, and every device that joined keeps working |

| Option | Meaning |
|---|---|
| `--join` | Give this device the key of an encrypted store, by typing its words |
| `--rotate` | Replace the store's key and words and re-encrypt every item; every other device must run `--join` again. Use it after losing a device |
| `--recover` | Finish or undo a migration or rotation that was interrupted |

```text
$ passalong encrypt
Encrypting keeps what the server stores unreadable without this store's six words.
Every device then needs passalong 0.2.0 or later, and joins once with `passalong encrypt --join`; older versions stop working with this store.
If the words and every device's key file are lost, the items cannot be recovered.
The 12 items stored now can be migrated, which re-encrypts each one by downloading and uploading it again, or left unencrypted in plain/ on the server until you remove them with `passalong prune --plain` (a fresh start).
Encrypt this store? [y/N] y
Migrate the 12 items or start fresh? [migrate/fresh] [migrate]:

The store's six words:

    abacus doorman quilt refinish tidy unwired

Write them down or keep them in a password manager. Every device types them to join, and nothing else can recover the store.

Type the six words to confirm:
encrypted the store: key 3f9a2c1d
migrated 12 items
```

The words are the only way to recover the store: keep them in a password
manager. A migration or rotation holds a lock on the server while it runs;
other devices wait, and if it is interrupted, `passalong encrypt --recover`
shows what it was doing and asks whether to finish or undo it.

## `passalong check`

Checks the setup in four steps, printing one line for each, then reports
whether `serve` is running:

```text
config         ok    /home/me/.config/passalong/config.toml
server         ok    ssh passalong@192.168.1.10:22, /srv/passalong
encryption     off   not encrypted
storage read   ok    12 items
storage write  ok    wrote and removed a 128-byte probe in 184 ms (696 B/s)
serve          ok    running (pid 4242)
```

- `config`: the config file is found and valid.
- `server`: passalong connects. For `ssh`, the server must present the
  pinned host key and accept the login.
- `storage read`: the storage directory can be listed.
- `storage write`: a 128-byte probe file is written under the store's
  `tmp/` folder and removed again. Listings and other devices never see
  it, so it does not reach pull mode. The line reports how long that took
  and the rate it makes; with so few bytes the time is mostly network round
  trips rather than bandwidth.
- `serve`: whether `serve` is running on this machine, as `serve --status`
  reports it. It is informational: `off` is not a failure, and the line is
  shown even when a check failed.

The first failure is shown as `FAIL` with the reason, the remaining checks
as `skip`, and `check` exits with 1 and an `error: check failed: ...` line.
A backend that cannot test writes shows `n/a` for the last check, which is
not a failure.

The `encryption` line says `off` for a plaintext store, and `on (key …)`
when this device holds the store's key, with the number of unencrypted
items a fresh start left. It fails, naming the command that fixes it, when
this device has no key or another key, has a key for a plaintext store, or
when a re-encryption is in progress.

## Output and exit codes

Results go to standard output; logs and errors go to standard error. A
failure prints one line starting with `error:`.

With `--quiet`, a successful command prints nothing, except `cat`, whose
output is the item itself, so scripts can rely on the exit code. Errors are
still printed. What a question asks about is shown with the question on
standard error: `init`'s host-key fingerprint and `prune`'s list of items.

| Exit code | Meaning |
|---|---|
| 0 | Success |
| 1 | The command failed, for example a missing config file or an unknown item |
| 2 | Invalid command-line usage |

## `passalong list`

Lists stored items, newest first.

```text
$ passalong list
ID                     KIND  NAME        SIZE     DEVICE  CREATED
6aa52143-8f3a1c2b9d0e  file  report.pdf  1.5 KiB  laptop  2026-09-12 11:54
6aa52107-2cf24dba5fb0  text  hello       5 B      box     2026-09-12 11:53
```

`NAME` shows the file name for files and the start of the text for text,
cut to 40 characters. `CREATED` is in local time. An empty store prints
`no items`.

`--json` prints the items' full metadata as a JSON array instead, using the
fields described in [architecture](architecture.md#metajson).

With the ssh backend, `list` prints the list cache that `serve` keeps (see
[configuration](configuration.md#serve-files)) when it was checked within
twice `serve.list_cache_check_secs`, 2 minutes by default, and makes no
connection. Otherwise it reads the server and writes the cache. `--nocache`
always reads the server and rewrites the cache. The output is the same
either way; `--log-level verbose` says which was used. `file`, `clipboard`,
`delete`, and `prune` add their own changes to the cache, and `load`,
`cat`, and `get` refresh it after their output. A cache problem never fails
a command.

On an encrypted store that still holds unencrypted items from a fresh start,
`list` warns about them on standard error.

## `passalong clipboard`

Sends the clipboard's text and prints the new item's id. When the
clipboard holds no text but an image, such as a screenshot, it sends the
image instead, stored as a PNG file named `clipboard-YYYYMMDD-HHMMSS.png`
and listed with kind `image`. A browser's "Copy image" also puts the
image's link, or an `<img>` tag, on the clipboard; that does not count as
text, so the image is sent.

```text
$ passalong clipboard
6aa52107-2cf24dba5fb0
$ echo "from a script" | passalong clipboard --stdin
6aa5210c-91d1e2a7c4b3
```

`--stdin` reads the text from standard input instead, which also works
without a desktop session. In PowerShell the same is
`"from a script" | passalong clipboard --stdin`. A clipboard with neither text nor an image, or
only whitespace, is refused with `error: clipboard is empty`. Sending text that is already stored prints the
existing item's id and stores nothing new.

## `passalong file <PATH>`

Sends a file and prints the new item's id. The file is streamed, so large
files are never read into memory. The item keeps the file's name, and its
type is guessed from the extension. Directories are refused. Sending a file
whose content is already stored prints the existing item's id.

## `passalong load <ID> [DEST]`

Copies an item out of the store. `ID` is the full id or at least 4
characters of it: the start of the content key (`2cf2`), or the start of the
full id when it contains a `-`.

| Form | Result |
|---|---|
| `passalong load 2cf2` | Text items go to the clipboard. File items are downloaded into `client.download_dir` (`~/Downloads` by default), which is created if missing. |
| `passalong load 2cf2 ~/Downloads` | Into an existing directory, under the item's file name, or `<id>.txt` for text. |
| `passalong load 2cf2 ./copy.pdf` | To exactly that file. |

When writing a file, the path written is printed. An existing file is not
replaced unless `--force` is given. The content is written to a temporary
`.passalong-part` file next to the target and checked against the item's
SHA-256 first, so a corrupted or truncated download never replaces
anything. File names stored on the server are reduced to their last
component, so a name like `../../etc/passwd` is written as `passwd` inside
the destination directory.

A download into `client.download_dir` never replaces a file: if the name is
taken, `load` writes `report (1).pdf`, then `report (2).pdf`, and so on, and
prints the name it used. `--force` overwrites the original name instead.

When a prefix matches several items and you are at a terminal, `load`,
`cat`, and `delete` list up to 9 of them, newest first, and ask which one
you mean:

```text
`68c3a1b2-` matches 3 items:
  1  68c3a1b2-9f1c02d4e5a6  text  meeting notes for Friday    laptop  2 min ago
  2  68c3a1b2-2cf2a8b17c3d  file  report.pdf                  laptop  2 min ago
  3  68c3a1b2-0b7e44c21a90  text  https://example.com/a-link  phone   2 min ago
Choose 1-3, or press Enter to cancel:
```

Pressing Enter, or three answers that are not a listed number, cancels
without changing anything; `delete` asks about every ambiguous id before it
deletes any item. With more than 9 matches, type more characters of the id.
Without a terminal, as in scripts, the command fails with exit code 1 and
the error lists every match.

## `passalong cat <ID>`

Prints an item's content to standard output exactly as stored, with
nothing added, so it can be piped or redirected:

```sh
passalong cat 2cf2 | wc -l
passalong cat 8f3a > report.pdf
```

In PowerShell, redirecting or piping binary content like this needs
PowerShell 7.4 or later. Windows PowerShell 5.1 re-encodes what passes
through `|` and `>`, which corrupts anything that is not text; there, use
`passalong load 8f3a report.pdf` instead. Text is fine in either.

At the default log level nothing is written to standard error on success;
the "item printed" record appears from `--log-level verbose`.

The content is checked against the item's SHA-256 as it streams. A
mismatch is reported with `item <ID> failed verification` and exit code 1
after the output has been written, as `curl` does, so treat that output
as damaged.

On a terminal, items that are not text are refused, because binary data
can garble the terminal:

```text
error: item 8f3a9c0d-... is binary (application/pdf); redirect the output or use --force
```

| Option | Meaning |
|---|---|
| `--force` | Print a binary item to the terminal anyway |

## `passalong get <ID>`

Prints an item's metadata, one field per line, without reading its
content:

```text
id:      6aa52107-2cf24dba5fb0
kind:    file
name:    report.pdf
mime:    application/pdf
size:    1.5 KiB (1536 bytes)
sha256:  2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
device:  laptop
created: 2026-09-12 11:53:11 +02:00 (2026-09-12T09:53:11Z)
```

Text items show a `preview` line instead of `name`. Clipboard images have
kind `image` and an `origin: clipboard` line. The creation time is shown in
local time and in UTC. The id works as for `load`.

| Option | Meaning |
|---|---|
| `--json` | Print the metadata as a JSON object, the same as the item's entry in `list --json` |

## `passalong choose`

Opens a full-screen list of the stored items, newest first, to pick one and
act on it. It needs a terminal.

| Key | Action |
|---|---|
| Up, Down, `k`, `j`, Page Up, Page Down, Home, End | Move |
| `/` | Filter: type to match the id, name or preview, device, or kind, ignoring case; Enter keeps the filter, Esc clears it |
| Enter | Load the item, as `passalong load <ID>` does: text and images go to the clipboard, files to the download directory |
| `c` | Print the item, as `passalong cat <ID>` does |
| `g` | Show its metadata, as `passalong get <ID>` does, in a dialog over the list; Up, Down, Page Up, Page Down, Home, and End scroll it, and Esc, `q`, `g`, or Enter close it |
| `d` | Delete it after `y`, then show the list read again from the store |
| `r` | Reload the list: from the list cache when it is fresh, otherwise from the server |
| `R` | Read the list from the server and rewrite the list cache |
| `?` | Show the passalong version, what it is, and these keys; any key closes it |
| `q`, Esc, Ctrl-C | Quit without doing anything |

With the ssh backend, the list opens from a fresh list cache, as `list`
does, and the bottom line says how old it is, such as `cached 30 s ago`.
The connection is still made first, for the actions.

Loading and printing close the list first, so what they print stays in the
terminal. With `--quiet`, nothing is printed after the list closes. While
the list is being read or an item deleted, the bottom line says
`Loading...`, `Reloading...`, or `Deleting <ID>...`, and keys wait until it
is done. Log lines are held while the list is open and printed when it
closes.

## `passalong delete <ID>...`

Deletes items from the store and prints each deleted id. Each `ID` is a full
id or at least 4 characters of it, as for `load`.

Every id is resolved before anything is deleted, so an unknown or ambiguous
id deletes nothing and exits with code 1. An item named twice is deleted
once. There is no confirmation prompt, as with `rm`; use `list` first if
in doubt.

## `passalong prune`

Deletes items by age or count. At least one of `--older-than` and `--keep`
is required:

| Option | Meaning |
|---|---|
| `--older-than <AGE>` | Delete items created at least this long ago. `AGE` is a whole number followed by `m`, `h`, `d`, or `w`, such as `90m` or `30d`. |
| `--keep <N>` | Always keep the newest `N` items. |
| `--dry-run` | Show what would be deleted, then stop. |
| `--yes` | Delete without asking. Required when not running in a terminal. |
| `--plain` | Prune the unencrypted items a fresh start left in `plain/`, instead of the store's items; `plain/` is removed once empty. It also removes what passalong 0.2.0 left behind when it encrypted a store: uploads and deletions cut short before encryption, still unencrypted in `tmp/`, and unused journals. `check` and `list` report both. |

With both options, an item survives if either protects it:
`prune --older-than 30d --keep 20` deletes items older than 30 days but
never leaves fewer than the newest 20.

`prune` shows the items it will delete and asks `Delete N items? [y/N]`.
When standard input is not a terminal, as in a cron job, it refuses unless
`--yes` is given. A real run also removes staging directories older than
one hour that interrupted uploads left on the server.

```text
$ passalong prune --older-than 30d --dry-run
2 items to delete:
ID                     KIND  NAME     SIZE     DEVICE  CREATED
6a8a1c07-9f3b2e11aa04  file  old.pdf  2.0 MiB  laptop  2026-08-01 10:12
6a8a0a55-c7d0e4f19b20  text  hello    5 B      box     2026-08-01 09:40
dry run: nothing deleted
```

## `passalong serve`

Keeps running and sends:

- every new clipboard text, checked every `serve.clipboard_poll_interval_ms`;
- every file dropped into `serve.drop_folder`, once its size and modification
  time have stayed the same for `serve.file_stable_wait_ms`.

Files already in the drop folder when `serve` starts are sent too. Hidden
files, subfolders, symbolic links, and names ending in `.part`,
`.crdownload`, `.tmp`, or `.passalong-part` are ignored. After a file is
sent it moves to `<drop_folder>/sent/` (a number is added if the name is
taken), or is deleted when `serve.after_send = "delete"`.

Text that is already stored, for example text that `passalong load` just
put on the clipboard, is not sent again.

When the server cannot be reached, `serve` logs a warning and retries the
same item after 1, 2, 4 … seconds, up to one minute apart, reconnecting each
time. It never gives up on an item because of a network problem. A file
that cannot be read, for example because of its permissions, is skipped
and sent once it changes, such as after you fix its permissions.

`serve` stops cleanly on Ctrl-C or SIGTERM. It exits with code 1 only when
it cannot start: invalid configuration, an unreachable server at start-up,
or a drop folder that cannot be created or watched. Without a desktop
clipboard, for example over SSH or in a container, it keeps watching the
drop folder and logs `clipboard unavailable`.

### Pull mode

With `pull = true` in `[serve]`, `serve` also applies items that other
devices send, which turns passalong into two-way sync:

- The newest text or clipboard image sent since the last check goes onto
  this device's clipboard. Older ones in the same check are skipped.
- Every file, including PNG files sent with `passalong file`, is downloaded
  into `client.download_dir`, but only if that directory exists. It is
  never created, and an existing name is kept: the download is numbered, as
  in `report (1).pdf`. Files that arrive while the directory is missing are
  skipped for good.
- Items sent by this device (same `client.device_name`) and items that
  existed before `serve` started are ignored. Pulled content is never sent
  back.

`serve` checks every `pull_interval_ms`, 5 seconds by default. A server
that cannot be reached is retried at the next check without skipping
anything. Pull mode remembers which items it has already handled, so an
item from a device whose clock runs behind is still applied, once.

In an encrypted store, an item that does not open yet is tried again at
the next check while it is less than five minutes old, since its files may
still be arriving through a synced folder; the items after it are applied
meanwhile. An older item that does not open is damaged: it is logged,
skipped for good, and never written to the clipboard or the download
folder.

### Running `serve` in the background

Only one `serve` runs at a time: a second one exits with
`serve is already running (pid N)`.

| Option | Meaning |
|---|---|
| `--daemon` | Start `serve` in the background and return. Prints `serve started (pid N, log PATH)`, or the start-up error and exit code 1. |
| `--status` | Print `running (pid N, log PATH)`, or `not running` with exit code 3. |
| `--stop` | Stop the running `serve` and wait up to 10 seconds for it to exit. |

The background process keeps running after you close the terminal and logs
to a file (see [configuration](configuration.md#serve-files)). On Windows it
has no console window. Windows has no stop signal, so there `--stop` leaves
a `serve.stop` file beside `serve.pid`, which `serve` checks every second.
For start at login and restarts after crashes, use
`passalong service-install`.

## `passalong service-install`

Installs `serve` so it starts at login: a systemd user unit on Linux and a
launchd agent on macOS, both restarting `serve` after a crash. On Windows
it uses the Run key, or a Task Scheduler task with `--scheduler`. Run
`passalong check` first to make sure the setup works.

```sh
passalong service-install   # write the unit, enable it, start it
passalong service-remove    # stop it and remove the unit
```

| Platform | Unit | Loaded with |
|---|---|---|
| Linux | `${XDG_CONFIG_HOME:-~/.config}/systemd/user/passalong-serve.service`, logging to the journal | `systemctl --user daemon-reload`, then `systemctl --user enable --now passalong-serve.service` |
| macOS | `~/Library/LaunchAgents/com.passalong.serve.plist`, logging to `~/Library/Logs/passalong/serve.log` | `launchctl bootstrap gui/<uid>` |
| Windows | The value `passalong-serve` in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, running `serve --daemon` at log-in and logging to `%LOCALAPPDATA%\passalong\serve.log` | `reg add`, then `passalong serve --daemon` to start it now |
| Windows, `--scheduler` | The scheduled task `passalong-serve`, running `serve` at your log-in and restarting it up to 3 times, a minute apart, if it fails | `schtasks /Create /XML`, then `schtasks /Run` |

The unit runs this `passalong` binary by its full path, with `--config` as
an absolute path when you give one. On Linux and macOS it runs from your
home directory, so a `~/.env` holding `PASSALONG_SSH_KEY_PASSPHRASE` is
found.

Which Windows option to use:

- **The Run key** (the default) needs no administrator. Windows runs it
  once at log-in and does not restart it after a crash. A command over 260
  characters is refused: use a shorter config path, or `--scheduler`.
- **`--scheduler`** restarts `serve` after a failure, but Windows lets only
  an administrator create a log-on task: run it from an administrator
  prompt. The task still runs as you, in your session, without
  administrator rights, so it can reach your clipboard. Its console window
  stays open while `serve` runs.

Neither option is a Windows service: services run in a separate session
that has no clipboard.

A unit with the same content is left alone; one that differs is replaced
only with `--force`. On Windows an existing Run key value is compared the
same way, and an existing scheduled task is replaced only with `--force`.
The service is not started while another `serve` runs: stop it first with
`passalong serve --stop`. If `systemctl` or `launchctl` fails, the unit is
left in place and the error names the command. Other platforms get
`service-install and service-remove support Linux (systemd), macOS
(launchd), and Windows (the Run key or Task Scheduler) only`.

| Option | Meaning |
|---|---|
| `--no-start` | Write the unit without enabling or starting it, and print the command that would |
| `--force` | Replace an installed unit that differs |
| `--scheduler` | Windows only: register a Task Scheduler task instead of the Run key; needs an administrator prompt |

The files in `docs/service/` are the same units with placeholder paths, for
installing by hand.

## `passalong service-remove`

Stops, disables, and removes the unit `service-install` wrote. On Linux it
runs `systemctl --user disable --now passalong-serve.service`, removes the
file, and reloads systemd; on macOS it boots the agent out with `launchctl
bootout` (an agent that is not loaded is fine) and removes the file. On
Windows it deletes the Run key value and the scheduled task, whichever
exist; it ends the task first. A `serve` started from the Run key keeps
running until `passalong serve --stop`. Without an installed unit it prints
`not installed: …` and succeeds.

## Clipboard support

Clipboard text and images work on macOS and on Linux under X11 or a
Wayland compositor that supports the `wlr-data-control` protocol, such as
Hyprland or Sway.

Images are exchanged as RGBA pixels and stored as PNG. `load` puts a
clipboard image back on the clipboard; with a destination it writes the
PNG file, and `cat` prints the PNG bytes. PNG files sent with
`passalong file` stay files and are downloaded like any other. Images
larger than 64 megapixels are refused. Clients older than v0.1.2 see
clipboard images as ordinary PNG files.

On Linux the clipboard's content belongs to the program that set it, so
`passalong load` hands the text or image to a small background process
that keeps it available until something else is copied, as `wl-copy` and
`xclip` do. `load` itself returns immediately.
