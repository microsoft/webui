#!/bin/sh
# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

set -eu

# Usage: commerce-entrypoint.sh [--css <link|module|style>] [--port <port>]
#
# Each container runs a single marketplace-api instance with one CSS strategy.
# Defaults: --css link --port 3000

CSS_STRATEGY="${CSS_STRATEGY:-link}"
PORT="${PORT:-3000}"

while [ $# -gt 0 ]; do
    case "$1" in
        --css)  CSS_STRATEGY="$2"; shift 2 ;;
        --port) PORT="$2";         shift 2 ;;
        *)      echo "Unknown option: $1" >&2; exit 1 ;;
    esac
done

case "${CSS_STRATEGY}" in
    link|module|style) ;;
    *) echo "Invalid CSS strategy: ${CSS_STRATEGY} (expected link, module, or style)" >&2; exit 1 ;;
esac

exec marketplace-api --port "${PORT}" --css "${CSS_STRATEGY}" --no-tls
