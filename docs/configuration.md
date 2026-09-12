# Configuration

> Draft. Completed in PLAN-00001 STEP-14.

## Lookup order

The first existing file wins:

1. `--config <path>`
2. `PASSALONG_CONFIG_FILE`
3. `$XDG_CONFIG_HOME/passalong/config.toml`
4. `$HOME/.config/passalong/config.toml`
5. `/etc/passalong/config.toml`
6. `./config.toml`

See `config.sample.toml` for an annotated example.

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
