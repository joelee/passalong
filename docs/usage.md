# Usage

> Draft. Each command's section is written when the command lands
> (PLAN-00001 STEP-08 to STEP-13) and completed in STEP-14.

## Global options

| Option | Meaning |
|---|---|
| `--config <PATH>` | Use this config file instead of the lookup order in [configuration](configuration.md) |
| `--log-level <LEVEL>` | `error`, `warning`, `info`, `verbose`, or `debug`; overrides `PASSALONG_LOG_LEVEL` and `client.log_level` |
| `-h`, `--help` | Show help |
| `-V`, `--version` | Show the version |

Global options may come before or after the subcommand.

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

## Clipboard support

Clipboard text works on macOS and on Linux under X11 or a Wayland compositor
that supports the `wlr-data-control` protocol, such as Hyprland or Sway.

On Linux the clipboard's content belongs to the program that set it. When
`passalong load` copies text to the clipboard and exits, the text survives
only if a clipboard manager takes it over; passalong waits up to 2 seconds
for one. Without a clipboard manager, load into a file instead.
