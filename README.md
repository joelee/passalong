# passalong

A lightweight, cross-platform clipboard and file sharing tool for macOS, Linux,
and (later) Android.

One machine you already own runs a plain SSH server with a storage directory.
Every other device pushes clipboard text and files there, lists what is stored,
and pulls items back — no cloud service, no account, no custom server daemon.

> **Status: under construction.** The project is being built in the open
> against [Delivery Plan 00001](docs/plans/00001-Initial_Plan.md).
> `clipboard`, `file`, `list`, and `load` work with a local storage
> directory; the SSH backend is being wired in and `serve` is not finished.

## How it works

```text
 laptop ─┐                          ┌─ desktop
         │  SSH/SFTP, pinned host key│
 phone ──┼──────►  server:/srv/passalong  ◄──────┘
         │         items/<time>-<hash>/{content,meta.json}
```

- The server needs nothing but `sshd` and a directory.
- Each client pins the server's SSH host public key, so there is no
  trust-on-first-use.
- Items are identified by a time-sortable id (`<time>-<content hash>`), so the
  newest items list first and identical content is stored only once.
- The storage layer sits behind a trait, so other backends such as a web API or
  an S3 bucket can be added later.

## Commands (v0.1.0 target)

| Command | What it does |
|---|---|
| `passalong clipboard` | Send the current clipboard text (`--stdin` reads standard input instead) |
| `passalong file <path>` | Send a file |
| `passalong list` | List stored items, newest first (`--json` for scripts) |
| `passalong load <id> [dest]` | Copy an item to `dest`, or to the clipboard when `dest` is omitted |
| `passalong serve` | Run in the foreground, sending every new clipboard text and every file dropped into the drop folder |

`serve` runs in the foreground by design; run it in the background with the
systemd or launchd examples that will ship in `docs/service/`.

## Server setup

Any machine with an OpenSSH server can be the server. Do this once:

1. Create a user and a storage directory for passalong:

   ```sh
   sudo useradd --create-home passalong
   sudo install -d -o passalong -m 700 /srv/passalong
   ```

2. Append each client's public key, for example `~/.ssh/id_ed25519.pub`, to
   `~passalong/.ssh/authorized_keys` on the server.

3. On each client, fetch the server's host key:

   ```sh
   ssh-keyscan -t ed25519 192.168.1.10
   ```

   Copy the `ssh-ed25519 AAAA...` part into `server.ssh.host_key`. The whole
   line as printed works too. passalong refuses to connect if the server
   ever presents a different key.

4. Copy `config.sample.toml` to `~/.config/passalong/config.toml` and set
   `host`, `user`, `host_key`, `identity_file`, and `remote_path`. If the key
   has a passphrase, put it in `PASSALONG_SSH_KEY_PASSPHRASE` in a `.env`
   file, never in the config.

## Building from source

Requires Rust 1.98.1 (pinned in `rust-toolchain.toml`; `rustup` installs it
automatically) and [`just`](https://github.com/casey/just).

```sh
just setup    # one-time: coverage tooling
just build    # debug build of the whole workspace
just run -- --help
```

## Development

Development is test-driven and every change must pass `just check` (format,
clippy, tests, coverage ≥ 80 %, locked build). See
[docs/developer-guide.md](docs/developer-guide.md).

| Document | Contents |
|---|---|
| [docs/architecture.md](docs/architecture.md) | Crates, traits, storage layout |
| [docs/configuration.md](docs/configuration.md) | Every configuration key and environment variable |
| [docs/usage.md](docs/usage.md) | Command reference |
| [docs/developer-guide.md](docs/developer-guide.md) | Toolchain, `just` recipes, testing |
| [docs/backlog.md](docs/backlog.md) | Planned future work |
| [CHANGELOG.md](CHANGELOG.md) | Release notes |

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
