# syntax=docker/dockerfile:1

# Builder and runtime share Debian trixie so the binary's glibc matches.
FROM rust:1.98.1-slim-trixie AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --locked -p passalong-cli

FROM debian:trixie-slim
RUN useradd --create-home --uid 10001 passalong
COPY --from=builder /src/target/release/passalong /usr/local/bin/passalong
USER passalong
WORKDIR /home/passalong
ENTRYPOINT ["passalong"]
CMD ["--help"]
