# Configuration

Settings live in `config.toml`; the only secret, the SSH key passphrase,
comes from the environment. `passalong init` writes a complete file for an
SSH server, at `$XDG_CONFIG_HOME/passalong/config.toml` or
`~/.config/passalong/config.toml` unless `--config` says otherwise. [`config.sample.toml`](../config.sample.toml) is
an annotated example with every key.

## Lookup order

The first existing file wins:

1. `--config <path>`
2. `PASSALONG_CONFIG_FILE`
3. `$XDG_CONFIG_HOME/passalong/config.toml`
4. `$HOME/.config/passalong/config.toml`
5. `/etc/passalong/config.toml`
6. `./config.toml`

A path named by `--config` or `PASSALONG_CONFIG_FILE` must exist; passalong
reports the typo instead of falling back to another file. Relative paths
resolve against the working directory. Empty environment variables count as
unset, and a relative `XDG_CONFIG_HOME` is ignored.
A location that exists but cannot be read, for example because a directory
on its path may not be entered, is reported as
`cannot read config file <path>: permission denied` rather than skipped.

## Keys

Unknown keys are rejected, and every error names the offending key or line.
`~` at the start of a local path expands to `$HOME`.

### `[client]`

| Key | Type | Default | Description |
|---|---|---|---|
| `device_name` | string | host name | Name recorded on every item this device sends |
| `log_level` | string | `info` | `error`, `warning`, `info`, `verbose`, or `debug` (see Logging) |
| `key_file` | path | `store.key` beside the default config file | Where this device keeps an encrypted store's key: `$XDG_CONFIG_HOME/passalong/store.key`, or `~/.config/passalong/store.key`; there is no default when neither `XDG_CONFIG_HOME` nor `HOME` is set. Must be absolute after `~` expansion. It is written with mode 0600, and refused if other users can read it or if it is inside a git work tree that does not ignore it |
| `download_dir` | path | `~/Downloads` | Where `load` puts file items when no destination is given, created if missing; pull mode writes here only if it exists. Must be absolute after `~` expansion |

### `[server]`

| Key | Type | Default | Description |
|---|---|---|---|
| `kind` | string | required | Storage backend: `ssh` or `local` |

### `[server.ssh]` (required when `kind = "ssh"`)

| Key | Type | Default | Description |
|---|---|---|---|
| `host` | string | required | Server host name or IP address |
| `port` | integer | `22` | 1 to 65535 |
| `user` | string | required | Login user |
| `host_key` | string | required | The server's public host key, pinned: the `ssh-ed25519 AAAA...` part of `ssh-keyscan -t ed25519 <host>`. An `ssh-rsa` key needs a build with the `rsa` feature |
| `identity_file` | path | required | Private key used to log in; `~` is expanded. Ed25519 and ECDSA keys work in every build; RSA keys need a build with the `rsa` feature |
| `remote_path` | string | required | Storage directory on the server, absolute or relative to the login home; not `~`-expanded |
| `connect_timeout_secs` | integer | `10` | 1 to 3600 |

The key passphrase is never read from this file; see Environment variables.

### `[server.local]` (required when `kind = "local"`)

| Key | Type | Default | Description |
|---|---|---|---|
| `path` | path | required | Storage directory, for example a mounted network share; `~` is expanded |

### `[serve]`

| Key | Type | Default | Description |
|---|---|---|---|
| `drop_folder` | path | `~/PassAlong` | Folder watched for files to send |
| `clipboard_poll_interval_ms` | integer | `750` | 1 to 3600000 |
| `file_stable_wait_ms` | integer | `1000` | 0 to 3600000; how long a dropped file must stay unchanged before it is sent |
| `after_send` | string | `move` | `move` puts sent files in `<drop_folder>/sent/`; `delete` removes them |
| `clipboard_images` | boolean | `true` | Also send clipboard images; an image is read only when the clipboard holds no text |
| `pull` | boolean | `false` | Also apply items sent by other devices: text and images to the clipboard, files into `client.download_dir` when it exists. `client.download_dir` must then not be `drop_folder` or inside it |
| `pull_interval_ms` | integer | `5000` | 1000 to 3600000; how often pull mode checks for new items |
| `list_cache` | boolean | `true` | With the `ssh` backend, keep a local copy of the item list that `list` and `choose` read without connecting (see `serve` files) |
| `list_cache_check_secs` | integer | `60` | 10 to 86400; how often `serve` compares that copy with the server |

## Environment variables

| Variable | Purpose |
|---|---|
| `PASSALONG_CONFIG_FILE` | Config file path (lookup position 2) |
| `PASSALONG_SSH_KEY_PASSPHRASE` | Secret: passphrase for `identity_file`; the `passphrase` of the SSH settings comes only from here |
| `PASSALONG_LOG_LEVEL` | Overrides `client.log_level` |
| `XDG_CONFIG_HOME`, `HOME` | Lookup positions 3 and 4, and `~` expansion |

The CLI loads `./.env` at start-up when it exists. Values from `.env` never
override variables already set in the environment. Keep secrets only in
`.env`, which is git-ignored; `.env.sample` lists the supported names.

## `serve` files

`serve` records its pid, and `serve --daemon` writes its log, here:

| Platform | Pid file | Log file |
|---|---|---|
| Linux | `${XDG_STATE_HOME:-~/.local/state}/passalong/serve.pid` | `${XDG_STATE_HOME:-~/.local/state}/passalong/serve.log` |
| macOS | `~/Library/Application Support/passalong/serve.pid` | `~/Library/Logs/passalong/serve.log` |

With the `ssh` backend and `serve.list_cache` on, `serve` also keeps
`list-cache.json` in the pid file's folder: the item list, readable by you
only, since it holds text previews. `list` and `choose` use it while it was
checked within two `list_cache_check_secs`, and only for the server it was
made from.

The pid file is locked while `serve` runs, which is how a second copy is
refused. A pid file left behind by a crash is harmless and is reused. The
log file grows without rotation.

## Encrypted stores

`passalong encrypt` changes a store's layout. An encrypted store's root
holds:

| Path | What it is |
|---|---|
| `encryption/header.json` | The store's key, sealed under the key its six words derive (Argon2id, 64 MiB), and the key's id. No device name or time |
| `items` | A file, not a folder, that says the store is encrypted: clients before v0.2.0 fail on it instead of writing plaintext |
| `v2/items/<id>/` | Sealed items: `meta.json` shows only the schema and id, and `content` is sealed in 64 KiB chunks |
| `v2/tmp/` | Staging for uploads, and for the header while it is replaced |
| `plain/items/` | After a fresh start, the items stored before, unencrypted until `prune --plain` removes them |
| `.rewrite/` | Only while a migration or rotation runs: its lock and journal |

Each device keeps its copy of the store's key in `client.key_file`. A store
opens sealed only with that key; without it, with another key, or while
`.rewrite/` exists, commands refuse and name the command that fixes it.

## Logging

Logs go to standard error, one line per record:

```text
2026-09-12T09:53:11Z info passalong_core::store op=0123456789abcdef item stored id=... size=42
```

Each line has an RFC 3339 UTC timestamp, the syslog severity, the component,
the operation's correlation id when there is one, the message, and a fixed
set of fields (`id`, `size`, `name`, `path`, `kind`, `device`, `attempt`,
`error`). Clipboard text, file contents, and secrets are never logged.

| Level | Shows | syslog severity |
|---|---|---|
| `error` | Errors only | `err` |
| `warning` | Errors and warnings | `warning` |
| `info` (default) | Normal operation | `info` |
| `verbose` | Detailed progress | `notice` |
| `debug` | Everything, including the SSH library's debug output | `debug` |

Third-party libraries are limited to warnings and errors at every level
except `debug`. The level is chosen by `--log-level`, then
`PASSALONG_LOG_LEVEL`, then `error` when `--quiet` is given, then
`client.log_level`, then `info`.
