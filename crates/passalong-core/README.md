# passalong-core

Core library of [passalong](https://github.com/joelee/passalong), a
lightweight clipboard and file sharing tool that stores items on your own
SSH server.

It provides everything a front-end needs that does not depend on the
command line:

- configuration discovery and validation;
- the item model, with time-sortable ids and content hashing;
- the `Store` trait and `FsStore`, an atomic storage layout on top of any
  `RemoteFs`, plus the `local` backend;
- the `Clipboard` trait, with a desktop implementation behind the `desktop`
  feature;
- the `serve` loop and syslog-style logging.

The SSH backend lives in
[`passalong-ssh`](https://crates.io/crates/passalong-ssh), and the CLI in
[`passalong`](https://crates.io/crates/passalong).

Licensed under the Apache License, Version 2.0.
