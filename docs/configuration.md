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
