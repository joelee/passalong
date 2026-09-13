# Contributing to passalong

Thank you for helping. passalong is a small project with a few firm rules,
all described below or in the documents they link to.

## Before you start

- Open an issue to discuss anything larger than a small fix, so the change
  can be planned before code is written. Larger changes follow a delivery
  plan in `docs/plans/`.
- The road map keeps v0.1.x an SSH-only command-line client. A GUI,
  Android, and Windows support are planned for v0.2 and later.

## Making a change

1. Fork the repository and branch from `main`.
2. Set up the toolchain and tools as the
   [developer guide](docs/developer-guide.md) describes.
3. Work test first: write a failing test, then the code that makes it pass.
4. Run `just check` before pushing. It must pass, with line coverage of at
   least 80 %. Changes to SSH code should also pass `just test-integration`,
   which needs Docker.
5. Update the documentation, and add a line under `Unreleased` in
   [CHANGELOG.md](CHANGELOG.md).
6. Open a pull request to `main`. It needs the Linux, macOS, and desktop CI
   checks to pass and a maintainer's approval. Changes to CI, release
   tooling, or dependencies also need the code owner's review.

## Rules

- Never commit secrets. The SSH key passphrase belongs in `.env`, which git
  ignores.
- Keep `passalong-core` and `passalong-ssh` free of terminal and process
  code; that belongs in the CLI crate.
- Report security problems privately, as [SECURITY.md](SECURITY.md)
  explains.

## Licence

By contributing, you agree that your contribution is licensed under the
[Apache License, Version 2.0](LICENSE), like the rest of the project.
