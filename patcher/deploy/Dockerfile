# Publisher CLI for in-cluster jobs (publishing, renewal, expiry and health checks).
# Build context: the patcher/ directory.
#   docker build -f deploy/Dockerfile -t <registry>/patcher-cli:<version> .
FROM rust:1.99.1-slim-trixie@sha256:f6c4a35d21dbb3c89d7a8d056c193e94fc09037f2b58dd858b09c39a47dd02d7 AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY core ./core
COPY distribution ./distribution
RUN cargo build --locked --release --bin patcher

FROM debian:trixie-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2
COPY --from=build /src/target/release/patcher /usr/local/bin/patcher
# CA bundle for the health probe's HTTPS, from the digest-pinned build image.
COPY --from=build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
USER 65532:65532
