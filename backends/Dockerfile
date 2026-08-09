# Static musl builds: no glibc, no shell, no package manager in the runtime image.
# See docs/adr/0004-musl-mimalloc.md.

FROM rust:1.85-alpine AS builder
RUN apk add --no-cache musl-dev pkgconfig
WORKDIR /build
COPY Cargo.toml Cargo.lock clippy.toml ./
COPY .cargo ./.cargo
COPY .xtask ./.xtask
COPY backends ./backends
RUN rustup target add x86_64-unknown-linux-musl \
 && cargo build --profile dist --target x86_64-unknown-linux-musl \
      -p vpay-server -p vpay-worker-bin

FROM scratch AS server
COPY --from=builder /build/target/x86_64-unknown-linux-musl/dist/vpay-server /vpay-server
EXPOSE 8080
ENTRYPOINT ["/vpay-server"]

FROM scratch AS worker
COPY --from=builder /build/target/x86_64-unknown-linux-musl/dist/vpay-worker-bin /vpay-worker
ENTRYPOINT ["/vpay-worker"]
