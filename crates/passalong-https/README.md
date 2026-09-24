# passalong-https

Storage backend of [passalong](https://github.com/joelee/passalong), a
lightweight clipboard and file sharing tool, for a
[passalong-server](https://github.com/joelee/passalong-server) workspace.

It speaks the server's HTTPS API with an API key, trusting either the one
public key a `tls_pin` names or the operating system's trust store; nothing
turns verification off. Items are written in the same format as the `ssh`
and `local` backends, sealed on the device for an encrypted workspace.
`register` adds the `https` kind to a `BackendRegistry`.

Built on [`reqwest`](https://crates.io/crates/reqwest) and
[`rustls`](https://crates.io/crates/rustls) with the `ring` crypto backend.
This crate is written from the server's published API documents and shares
no code with the server.

Licensed under the Apache License, Version 2.0.
