# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

# ── Build arguments ──────────────────────────────────────────────────
ARG CARGO_CHEF_VERSION=0.1.75

# ── Stage 1: Dependency planner ─────────────────────────────────────
FROM rust:1.93-bookworm AS chef
ARG CARGO_CHEF_VERSION
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

# Cache Rust dependencies via cargo-chef
COPY --from=planner /build/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Copy full source
COPY . .

# ── Build JS/TS assets ──────────────────────────────────────────────
# Install all npm dependencies
RUN pnpm install --frozen-lockfile

# Build framework libraries first (needed by apps with build:deps)
RUN pnpm --filter @microsoft/webui-framework --filter @microsoft/webui-router run build

# Build each app's client assets
RUN cd examples/app/hello-world && \
    mkdir -p dist && \
    npx esbuild src/index.ts --bundle --outfile=dist/index.js --format=esm --minify

RUN cd examples/app/todo-fast && pnpm build
RUN cd examples/app/calculator && \
    mkdir -p dist && \
    npx esbuild src/index.ts --bundle --outfile=dist/index.js --format=esm --minify

RUN cd examples/app/todo-webui && pnpm build
RUN cd examples/app/contact-book-manager && pnpm build
RUN cd examples/app/component-assets && pnpm build
RUN cd examples/app/routes && pnpm build
RUN cd examples/app/commerce && pnpm build

# Build the demo shell client (bundles src/index.ts → dist/index.js)
RUN cd examples/demo && pnpm build

# Build Node.js API servers
RUN cd examples/app/contact-book-manager && \
    npx esbuild server/api.ts --bundle --platform=node --format=esm \
    --outfile=dist/api.js --packages=external

RUN cd examples/app/routes/server && \
    npx esbuild src/index.ts --bundle --platform=node --format=esm \
    --outfile=dist/index.js --packages=external

# ── Build Rust binaries ─────────────────────────────────────────────
RUN cargo build --release -p microsoft-webui-cli -p marketplace-api -p demo-shell

# ── Stage 3: Runtime ────────────────────────────────────────────────
FROM debian:bookworm-slim

LABEL org.opencontainers.image.source="https://github.com/microsoft/webui"
LABEL org.opencontainers.image.description="WebUI Demo — all example apps in one container"
LABEL org.opencontainers.image.licenses="MIT"

# Install Node.js runtime (needed for contact-book-manager & routes APIs)
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        gnupg && \
    install -d -m 0755 /etc/apt/keyrings && \
    curl -fsSL https://deb.nodesource.com/gpgkey/nodesource-repo.gpg.key | \
        gpg --dearmor -o /etc/apt/keyrings/nodesource.gpg && \
    chmod 0644 /etc/apt/keyrings/nodesource.gpg && \
    echo "deb [signed-by=/etc/apt/keyrings/nodesource.gpg] https://deb.nodesource.com/node_22.x nodistro main" \
        > /etc/apt/sources.list.d/nodesource.list && \
    apt-get update && \
    apt-get install -y --no-install-recommends nodejs && \
    rm -rf /var/lib/apt/lists/* && \
    groupadd --system app && \
    useradd --system --gid app --create-home app

WORKDIR /app

# Copy Rust binaries
COPY --from=builder /build/target/release/webui /usr/local/bin/webui
COPY --from=builder /build/target/release/marketplace-api /usr/local/bin/marketplace-api
COPY --from=builder /build/target/release/demo-shell /usr/local/bin/demo-shell

# Copy entrypoint
COPY --from=builder /build/examples/demo/docker-entrypoint.sh /usr/local/bin/demo-entrypoint.sh
RUN chmod 0755 /usr/local/bin/demo-entrypoint.sh && \
    sed -i 's/\r$//' /usr/local/bin/demo-entrypoint.sh

# Copy each app's source + built assets into /app/apps/{slug}/
# hello-world
COPY --from=builder --chown=app:app /build/examples/app/hello-world/src/ ./apps/hello-world/src/
COPY --from=builder --chown=app:app /build/examples/app/hello-world/dist/ ./apps/hello-world/dist/
COPY --from=builder --chown=app:app /build/examples/app/hello-world/data/ ./apps/hello-world/data/
COPY --from=builder --chown=app:app /build/examples/app/hello-world/demo.toml ./apps/hello-world/demo.toml

# todo-fast
COPY --from=builder --chown=app:app /build/examples/app/todo-fast/src/ ./apps/todo-fast/src/
COPY --from=builder --chown=app:app /build/examples/app/todo-fast/dist/ ./apps/todo-fast/dist/
COPY --from=builder --chown=app:app /build/examples/app/todo-fast/data/ ./apps/todo-fast/data/
COPY --from=builder --chown=app:app /build/examples/app/todo-fast/demo.toml ./apps/todo-fast/demo.toml

# calculator
COPY --from=builder --chown=app:app /build/examples/app/calculator/src/ ./apps/calculator/src/
COPY --from=builder --chown=app:app /build/examples/app/calculator/dist/ ./apps/calculator/dist/
COPY --from=builder --chown=app:app /build/examples/app/calculator/data/ ./apps/calculator/data/
COPY --from=builder --chown=app:app /build/examples/app/calculator/demo.toml ./apps/calculator/demo.toml

# todo-webui
COPY --from=builder --chown=app:app /build/examples/app/todo-webui/src/ ./apps/todo-webui/src/
COPY --from=builder --chown=app:app /build/examples/app/todo-webui/dist/ ./apps/todo-webui/dist/
COPY --from=builder --chown=app:app /build/examples/app/todo-webui/data/ ./apps/todo-webui/data/
COPY --from=builder --chown=app:app /build/examples/app/todo-webui/demo.toml ./apps/todo-webui/demo.toml

# contact-book-manager (has Node API)
COPY --from=builder --chown=app:app /build/examples/app/contact-book-manager/src/ ./apps/contact-book-manager/src/
COPY --from=builder --chown=app:app /build/examples/app/contact-book-manager/dist/ ./apps/contact-book-manager/dist/
COPY --from=builder --chown=app:app /build/examples/app/contact-book-manager/data/ ./apps/contact-book-manager/data/
COPY --from=builder --chown=app:app /build/examples/app/contact-book-manager/demo.toml ./apps/contact-book-manager/demo.toml

# component-assets
COPY --from=builder --chown=app:app /build/examples/app/component-assets/src/ ./apps/component-assets/src/
COPY --from=builder --chown=app:app /build/examples/app/component-assets/dist/ ./apps/component-assets/dist/
COPY --from=builder --chown=app:app /build/examples/app/component-assets/data/ ./apps/component-assets/data/
COPY --from=builder --chown=app:app /build/examples/app/component-assets/demo.toml ./apps/component-assets/demo.toml

# routes (has Node API)
COPY --from=builder --chown=app:app /build/examples/app/routes/src/ ./apps/routes/src/
COPY --from=builder --chown=app:app /build/examples/app/routes/dist/ ./apps/routes/dist/
COPY --from=builder --chown=app:app /build/examples/app/routes/server/dist/ ./apps/routes/server/dist/
COPY --from=builder --chown=app:app /build/examples/app/routes/demo.toml ./apps/routes/demo.toml

# commerce (custom Rust server, has images)
COPY --from=builder --chown=app:app /build/examples/app/commerce/src/ ./apps/commerce/src/
COPY --from=builder --chown=app:app /build/examples/app/commerce/dist/ ./apps/commerce/dist/
COPY --from=builder --chown=app:app /build/examples/app/commerce/images/ ./apps/commerce/images/
COPY --from=builder --chown=app:app /build/examples/app/commerce/demo.toml ./apps/commerce/demo.toml

# Demo shell (the `/` UI itself — a real WebUI app)
COPY --from=builder --chown=app:app /build/examples/demo/src/  ./shell/src/
COPY --from=builder --chown=app:app /build/examples/demo/dist/ ./shell/dist/

# Shared npm theme used by webui-cli demo apps at runtime
COPY --from=builder --chown=app:app /build/packages/webui-examples-theme/ ./node_modules/@microsoft/webui-examples-theme/

# Create data directories for apps that use state files
RUN mkdir -p apps/hello-world/data apps/todo-fast/data apps/calculator/data \
    apps/todo-webui/data apps/contact-book-manager/data apps/component-assets/data && \
    chown -R app:app apps/

USER app
EXPOSE 8080

CMD ["/usr/local/bin/demo-entrypoint.sh"]
