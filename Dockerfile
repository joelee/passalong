# syntax=docker/dockerfile:1

# Builder and runtime share Debian trixie so the binary's glibc matches.
FROM rust:1.98.1-slim-trixie AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --locked -p passalong

FROM debian:trixie-slim
# The home directory must be traversable (755) so the image also works with
# `--user "$(id -u):$(id -g)"`, which is needed to read a mounted SSH key.
RUN useradd --create-home --uid 10001 passalong && chmod 755 /home/passalong
COPY --from=builder /src/target/release/passalong /usr/local/bin/passalong
USER passalong
WORKDIR /home/passalong
ENTRYPOINT ["passalong"]
CMD ["--help"]
