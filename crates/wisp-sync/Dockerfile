# Self-hosted relay for project sync and remote web access (docs/remote-access.md).
# Build from the repository root:
#   docker build -f crates/wisp-sync/Dockerfile -t wisp-relay .

FROM rust:1-alpine AS build
# ring compiles C and assembly.
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY . .
# Alpine's Rust targets musl and links statically, so the binary needs no
# base image. The cache mounts keep rebuilds incremental.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p wisp-sync --bin wisp-relay \
    && cp target/release/wisp-relay /wisp-relay \
    && mkdir /data

FROM scratch
LABEL org.opencontainers.image.source="https://github.com/xuzhougeng/wisp-science" \
      org.opencontainers.image.licenses="AGPL-3.0-only"
COPY --from=build /wisp-relay /wisp-relay
# Owned by the relay's user, so a new volume mounted here is writable.
COPY --from=build --chown=10001:10001 /data /data
USER 10001:10001
# The default bind address is loopback, which nothing outside the container reaches.
ENV WISP_RELAY_ROOT=/data WISP_RELAY_BIND=0.0.0.0:8787
VOLUME /data
EXPOSE 8787
# The relay handles no signals and, as PID 1, would ignore SIGTERM until the
# stop timeout. It has nothing to flush: files are replaced atomically and
# desktops and browsers reconnect.
STOPSIGNAL SIGKILL
ENTRYPOINT ["/wisp-relay"]
