# passalong-ssh

SSH/SFTP storage backend of [passalong](https://github.com/joelee/passalong),
a lightweight clipboard and file sharing tool that stores items on your own
SSH server.

It connects with public-key authentication, refuses any server whose host
key differs from the pinned one, and exposes the server directory as a
`passalong_core::fs::RemoteFs`, so `passalong_core::store::FsStore` can store
items there. `register` adds the `ssh` kind to a `BackendRegistry`.

Built on [`russh`](https://crates.io/crates/russh) with the `ring` crypto
backend.

Licensed under the Apache License, Version 2.0.
