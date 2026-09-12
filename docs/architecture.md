# Architecture

> Draft. Completed in PLAN-00001 STEP-14; sections are filled in as the
> components land.

## Crates

| Crate | Kind | Responsibility |
|---|---|---|
| `passalong-core` | library | Configuration, item model, storage traits, clipboard trait, `serve` loop, telemetry. No CLI or terminal dependencies. |
| `passalong-ssh` | library | SSH/SFTP storage backend (`russh`), host-key pinning. |
| `passalong-cli` | binary `passalong` | Argument parsing, command handlers, output formatting. |

Future GUI and Android front-ends depend on `passalong-core` and
`passalong-ssh` only.

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
  "preview": null
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

Listing sorts the item directory names in reverse, which is newest first
because ids are time-sortable, then reads each `meta.json`. Directories that
are not ids, lack `meta.json`, or hold metadata that is unreadable or
describes a different id are skipped; corrupt ones are logged as warnings.

Users identify an item by typing at least 4 characters. Input without a `-`
matches the start of the content key, so `2cf2` finds
`6aa52107-2cf24dba5fb0`; input with a `-` matches the start of the full id.
Case and surrounding spaces are ignored. More than one match is an error
that lists the candidates.

## Adding a backend

To be written (STEP-12).
