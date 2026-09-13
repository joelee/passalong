# Security policy

## Supported versions

Security fixes go into the newest release on crates.io and GitHub. Older
versions are not patched; upgrade with `cargo install --locked passalong`.

## Reporting a vulnerability

Report vulnerabilities privately through GitHub: open the repository's
**Security** tab and choose **Report a vulnerability**. Please do not open a
public issue or pull request for a security problem.

Include the passalong version, your platform, and the steps or input that
show the problem. The discussion stays private in the advisory until a fix
is released.

## Scope

Examples of what counts as a vulnerability in passalong:

- connecting to a server whose host key does not match
  `server.ssh.host_key`;
- item names or ids from the server writing outside the chosen directory;
- content that fails its SHA-256 check being written or put on the
  clipboard;
- pull mode writing outside `client.download_dir` or sending pulled content
  back;
- secrets such as the key passphrase reaching logs or error messages.

Weaknesses in the SSH server itself belong with its project. `cargo deny`
checks passalong's dependencies against the RustSec advisory database on
every CI run.
