# Build from the repository root. No display server or WebKit is needed.
FROM rust:1-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends libdbus-1-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p wisp-server \
    && cp target/release/wisp-server /wisp-server

FROM ghcr.io/astral-sh/uv:0.9.3 AS uv
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libdbus-1-3 python3 python3-venv git openssh-client tini \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 10001 --create-home wisp \
    && mkdir -p /data /opt/wisp \
    && chown 10001:10001 /data
COPY --from=build /wisp-server /usr/local/bin/wisp-server
COPY --from=uv /uv /usr/local/bin/uv
COPY python/ /opt/wisp/python/
COPY r/ /opt/wisp/r/
COPY skills/ /opt/wisp/skills/
USER 10001:10001
WORKDIR /data
ENV WISP_SERVER_DATA=/data WISP_SERVER_WORKSPACE=/data/workspace \
    WISP_SERVER_BIND=0.0.0.0:8787 WISP_SKILLS_PATH=/opt/wisp/skills \
    WISP_KERNEL_WORKER=/opt/wisp/python/kernel_worker.py \
    WISP_R_KERNEL_WORKER=/opt/wisp/r/kernel_worker.R
VOLUME /data
EXPOSE 8787
ENTRYPOINT ["/usr/bin/tini", "--", "wisp-server"]
