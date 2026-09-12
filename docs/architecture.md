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

## Item schema

To be written (STEP-04).

## Storage layout

To be written (STEP-06).

## Adding a backend

To be written (STEP-12).
