# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

# ── Build arguments ──────────────────────────────────────────────────
ARG CARGO_CHEF_VERSION=0.1.75

# ── Stage 1: Dependency planner ─────────────────────────────────────
FROM rust:1.93-bookworm AS chef
RUN cargo install cargo-chef@${CARGO_CHEF_VERSION} --locked
WORKDIR /build

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ── Stage 2: Builder ────────────────────────────────────────────────
FROM chef AS builder

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        gnupg \
        protobuf-compiler && \
    install -d -m 0755 /etc/apt/keyrings && \
    curl -fsSL https://deb.nodesource.com/gpgkey/nodesource-repo.gpg.key | \
        gpg --dearmor -o /etc/apt/keyrings/nodesource.gpg && \
    chmod 0644 /etc/apt/keyrings/nodesource.gpg && \
    echo "deb [signed-by=/etc/apt/keyrings/nodesource.gpg] https://deb.nodesource.com/node_22.x nodistro main" \
        > /etc/apt/sources.list.d/nodesource.list && \
    apt-get update && \
    apt-get install -y --no-install-recommends nodejs && \
    corepack enable && \
    rm -rf /var/lib/apt/lists/*

COPY --from=planner /build/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

COPY . .
RUN pnpm install --frozen-lockfile && \
    cd examples/app/commerce && pnpm build

RUN cargo build --release -p marketplace-api

# ── Stage 3: Runtime ────────────────────────────────────────────────
FROM debian:bookworm-slim

LABEL org.opencontainers.image.source="https://github.com/microsoft/webui"
LABEL org.opencontainers.image.description="Commerce Demo using Microsoft WebUI Framework"
LABEL org.opencontainers.image.licenses="MIT"

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/* && \
    groupadd --system app && \
    useradd --system --gid app --create-home app

WORKDIR /app

COPY --from=builder /build/target/release/marketplace-api /usr/local/bin/marketplace-api
COPY --from=builder --chown=app:app /build/examples/app/commerce/src/ ./src/
COPY --from=builder --chown=app:app /build/examples/app/commerce/dist/ ./dist/

USER app
EXPOSE 3004

CMD ["marketplace-api", "--port", "3004"]
