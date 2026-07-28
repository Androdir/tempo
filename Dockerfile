# syntax=docker/dockerfile:1
#
# Tempo Hub image for a Raspberry Pi (arm64) or any Docker host. Multi-stage:
#   1. build the React dashboard (Node)
#   2. build the headless hub binary (Rust) — only tempo-core + tempo-hub, so the
#      Tauri/webkit/X11 desktop deps are never pulled
#   3. tiny runtime that serves the dashboard + REST API
#
# Build & run on the Pi with: docker compose up -d --build

# ---- 1. React dashboard -------------------------------------------------------
FROM node:20-slim AS web
WORKDIR /app
COPY package.json package-lock.json* ./
RUN npm ci || npm install
COPY . .
RUN npm run build           # → /app/dist

# ---- 2. Hub binary ------------------------------------------------------------
FROM rust:1-bookworm AS build
WORKDIR /build
COPY crates/ ./crates/
COPY Cargo.hub.toml ./Cargo.toml
COPY Cargo.hub.lock ./Cargo.lock
RUN cargo build --release --locked -p tempo-hub

# ---- 3. Runtime ---------------------------------------------------------------
FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd -r -u 10001 tempo \
 && mkdir -p /data /app/web \
 && chown -R tempo /data
COPY --from=build /build/target/release/tempo-hub /usr/local/bin/tempo-hub
COPY --from=web   /app/dist                       /app/web
# Inside the container we bind to 0.0.0.0; the host port mapping + your LAN
# firewall / Tailscale control real exposure. NEVER publish this to the internet.
ENV TEMPO_PORT=7700 \
    TEMPO_BIND=0.0.0.0 \
    TEMPO_DB=/data/tempo.db \
    TEMPO_STATIC_DIR=/app/web
EXPOSE 7700
VOLUME ["/data"]
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
  CMD curl --fail --silent --show-error http://127.0.0.1:7700/api/health || exit 1
USER tempo
WORKDIR /app
# TEMPO_PAIRING_SECRET must be supplied at runtime (compose / -e), never baked in.
CMD ["tempo-hub"]
