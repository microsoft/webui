# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

# Runtime-only image — the binary and assets are built on the CI runner
# using .github/actions/build and staged into the Docker context.
FROM debian:bookworm-slim

LABEL org.opencontainers.image.source="https://github.com/microsoft/webui"
LABEL org.opencontainers.image.description="WebUI Commerce DEMO"
LABEL org.opencontainers.image.licenses="MIT"

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/* && \
    groupadd --system app && \
    useradd --system --gid app --create-home app

WORKDIR /app

COPY marketplace-api /usr/local/bin/marketplace-api
COPY --chown=app:app src/ ./src/
COPY --chown=app:app dist/ ./dist/

USER app
EXPOSE 443

CMD ["marketplace-api", "--port", "443"]
