# Usage

Every command reads the configuration described in
[configuration](configuration.md). Set up the server first, as described in
the [README](../README.md#server-setup).

## Global options

| Option | Meaning |
|---|---|
| `--config <PATH>` | Use this config file instead of the lookup order in [configuration](configuration.md) |
| `--log-level <LEVEL>` | `error`, `warning`, `info`, `verbose`, or `debug`; overrides `PASSALONG_LOG_LEVEL` and `client.log_level` |
| `-h`, `--help` | Show help |
| `-V`, `--version` | Show the version |

Global options may come before or after the subcommand.

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
```

The key is written only after you confirm its fingerprint. `init` writes to
`--config` when given, otherwise to the standard location, and refuses to
replace an existing file without `--force`. After writing, it logs in and
lists the server to prove the settings work.

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

## Output and exit codes

Results go to standard output; logs and errors go to standard error. A
failure prints one line starting with `error:`.

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

## `passalong clipboard`

Sends the clipboard's text and prints the new item's id.

```text
$ passalong clipboard
6aa52107-2cf24dba5fb0
$ echo "from a script" | passalong clipboard --stdin
6aa5210c-91d1e2a7c4b3
```

`--stdin` reads the text from standard input instead, which also works
without a desktop session. Empty or whitespace-only text is refused with
`error: clipboard is empty`. Sending text that is already stored prints the
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
| `passalong load 2cf2` | Text items go to the clipboard. File items are refused with `destination required for file items`. |
| `passalong load 2cf2 ~/Downloads` | Into an existing directory, under the item's file name, or `<id>.txt` for text. |
| `passalong load 2cf2 ./copy.pdf` | To exactly that file. |

When writing a file, the path written is printed. An existing file is not
replaced unless `--force` is given. The content is written to a temporary
`.passalong-part` file next to the target and checked against the item's
SHA-256 first, so a corrupted or truncated download never replaces
anything. File names stored on the server are reduced to their last
component, so a name like `../../etc/passwd` is written as `passwd` inside
the destination directory.

A prefix that matches several items is refused, and the error lists them.

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
that cannot be read is skipped until `serve` restarts.

`serve` stops cleanly on Ctrl-C or SIGTERM. It exits with code 1 only when
it cannot start: invalid configuration, an unreachable server at start-up,
or a drop folder that cannot be created or watched. Without a desktop
clipboard, for example over SSH or in a container, it keeps watching the
drop folder and logs `clipboard unavailable`.

### Running `serve` in the background

`serve` runs in the foreground by design; let the operating system's
service manager keep it running.

- **Linux (systemd):** install `docs/service/passalong-serve.service` as a
  user unit. Its header shows the commands.
- **macOS (launchd):** install `docs/service/com.passalong.serve.plist` as a
  launch agent. Its header shows the commands.

## Clipboard support

Clipboard text works on macOS and on Linux under X11 or a Wayland compositor
that supports the `wlr-data-control` protocol, such as Hyprland or Sway.

On Linux the clipboard's content belongs to the program that set it. When
`passalong load` copies text to the clipboard and exits, the text survives
only if a clipboard manager takes it over; passalong waits up to 2 seconds
for one. Without a clipboard manager, load into a file instead.
