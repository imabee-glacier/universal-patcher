# Publisher CLI for in-cluster jobs (publishing, renewal, expiry and health checks).
# Build context: the patcher/ directory.
#   docker build -f deploy/Dockerfile -t <registry>/patcher-cli:<version> .
FROM rust:1.99.1-slim-trixie@sha256:f6c4a35d21dbb3c89d7a8d056c193e94fc09037f2b58dd858b09c39a47dd02d7 AS build
WORKDIR /src
# Dependencies first, against empty sources, so this layer is rebuilt only when Cargo.lock or a
# manifest changes. Source edits then recompile this crate alone.
COPY Cargo.toml Cargo.lock ./
COPY core/Cargo.toml core/build.rs ./core/
RUN mkdir core/src \
    && echo 'fn main() {}' > core/src/main.rs \
    && : > core/src/lib.rs \
    && cargo build --locked --release --bin patcher \
    && rm -r core/src
COPY core ./core
COPY distribution ./distribution
# Docker keeps the copied files' original times; touching them makes cargo see new sources.
RUN touch core/src/main.rs core/src/lib.rs \
    && cargo build --locked --release --bin patcher \
    && ./target/release/patcher --version

FROM debian:trixie-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2
COPY --from=build /src/target/release/patcher /usr/local/bin/patcher
# CA bundle for the health probe's HTTPS, from the digest-pinned build image.
COPY --from=build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
USER 65532:65532
