# passalong

A lightweight, cross-platform clipboard and file sharing tool for macOS, Linux,
and (later) Android.

One machine you already own runs a plain SSH server with a storage directory.
Every other device pushes clipboard text and files there, lists what is stored,
and pulls items back — no cloud service, no account, no custom server daemon.

> **Status: under construction.** The project is being built in the open
> against [Delivery Plan 00001](docs/plans/00001-Initial_Plan.md). The commands
> below describe the v0.1.0 target; they are not usable yet.

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
