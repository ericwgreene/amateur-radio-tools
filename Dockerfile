# syntax=docker/dockerfile:1

# Multi-stage build with cargo-chef for dependency caching.
# Build deps: only a C compiler (for `ring` + bundled `libsqlite3-sys`). No cmake/openssl —
# the TLS stack is rustls + ring, and trust roots are compiled in via webpki-roots.

########## chef base ##########
# `rust:1-slim-bookworm` tracks the latest stable (>= 1.85, required by edition 2024).
# Pin to an exact minor (e.g. rust:1.90-slim-bookworm) for fully reproducible builds.
FROM rust:1-slim-bookworm AS chef
RUN apt-get update \
    && apt-get install -y --no-install-recommends gcc libc6-dev \
    && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef --locked
WORKDIR /app

########## plan: capture the dependency graph only ##########
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

########## build ##########
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Compile dependencies only — this layer is cached until recipe.json changes.
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
# Build the web server and the standalone migration runner (reused by the migration Job).
RUN cargo build --release --bin amateur-radio-tools --bin migration

########## runtime ##########
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends wget \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -u 10001 -g nogroup app
WORKDIR /app
COPY --from=builder /app/target/release/amateur-radio-tools /app/amateur-radio-tools
COPY --from=builder /app/target/release/migration          /app/migration
COPY crates/web/static /app/static
# Production defaults; secrets and BASE_URL are injected by the platform, never baked in.
ENV BIND_ADDRESS=0.0.0.0:8080 \
    STATIC_DIR=/app/static \
    COOKIE_SECURE=true \
    RUST_LOG=info,web=info
EXPOSE 8080
USER 10001
# Local convenience; Azure Container Apps uses its own probes (defined in Bicep).
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD wget -qO- http://127.0.0.1:8080/health || exit 1
ENTRYPOINT ["/app/amateur-radio-tools"]
