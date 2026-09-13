# Backlog

Future work not covered by an active plan. Completed items are removed.

## @joelee road map for next releases

### v0.1.4

- **Clean up the Release workflow's annotations.**
- Scan and fix *.md on `https://crates.io/crates/passalong` and Github repo for broken links.
- Implement `passalong get <ID>`: Output metadata of the <ID>
- Implement `passlong install-service` for installing `passalong-serve.service` or equivilent for other OS.
- `passalong cat` prints '<content><log_line>'. `cat` will supress the <log_line>
- implement `--quiet` option to supress output (except `cat`)
- implement `passalong check` to check the configuration and server is accessible
- Beautify `How it works` ASCII diagram in `README.md` by tranforming it into Mermaid diagram.

### v0.1.5
- **Android cross-compile check in CI.**
- `passalong choose` - invoke a TUI to select 

### v0.2.0
- **Windows support**
- Add Homebrew package on `https://github.com/joelee/homebrew-oss`


## Agent suggested next steps

### Features

- **Encryption at rest.** Encrypt content before upload, for example with
  `age`, so the server operator cannot read items.
- **ssh-agent authentication.** Use keys held by an agent instead of an
  identity file.
- **Connection reuse.** One-shot commands open a new SSH connection each
  time; reuse or multiplex connections.
- **More backends.** S3 and HTTP API backends behind the existing registry.
  Their config sections must be added to the core configuration module.
- **Android client and desktop GUI** on top of `passalong-core` and
  `passalong-ssh`.
- **Windows support.**

### Engineering

- **Android cross-compile check in CI.** Build `passalong-core` and
  `passalong-ssh` for an Android target without default features.
- **Clean up the Release workflow's annotations.** The v0.1.3 Release run
  passed but showed ten error annotations in each of the "Check the tag and
  the packages" and "Publish to crates.io" jobs, such as `ENOENT: no such
  file or directory, opendir '.../target/package/passalong-core-0.1.3/tests/target'`.
  The job logs show the same errors. They come from the post step of
  `Swatinem/rust-cache@v2`, which walks
  `target/package/<crate>-<version>/`, the crates that `cargo publish`
  unpacks there. In each unpacked crate it finds a `tests` directory and
  looks for `tests/target` and `tests/trybuild` without awaiting the
  lookups, so its `try/catch` misses the errors (`cleanProfileTarget` in
  `src/cleanup.ts`). Fix: delete `target/package` in a final step of both
  jobs, before the post step runs; those unpacked crates are not worth
  caching. The runs also warn that `actions/upload-artifact@v4` and
  `actions/download-artifact@v4` target the deprecated Node.js 20; move
  them to their Node.js 24 releases. A `.github/dependabot.yml` with a
  `github-actions` entry would propose such updates automatically. The CI
  workflow shows none of these annotations.
