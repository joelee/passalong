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

## Keys

Unknown keys are rejected, and every error names the offending key or line.
`~` at the start of a local path expands to `$HOME`.

### `[client]`

| Key | Type | Default | Description |
|---|---|---|---|
| `device_name` | string | host name | Name recorded on every item this device sends |
| `log_level` | string | `info` | `error`, `warning`, `info`, `verbose`, or `debug` (see Logging) |

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
| `host_key` | string | required | The server's public host key, pinned: the `ssh-ed25519 AAAA...` part of `ssh-keyscan -t ed25519 <host>` |
| `identity_file` | path | required | Private key used to log in; `~` is expanded |
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
`PASSALONG_LOG_LEVEL`, then `client.log_level`, then `info`.
