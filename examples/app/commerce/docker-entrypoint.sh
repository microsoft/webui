#!/bin/sh
# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

set -eu

cleanup() {
    status=$?
    trap - INT TERM EXIT

    if [ -n "${PID_LINK:-}" ]; then
        kill "${PID_LINK}" 2>/dev/null || true
    fi
    if [ -n "${PID_MODULE:-}" ]; then
        kill "${PID_MODULE}" 2>/dev/null || true
    fi
    if [ -n "${PID_STYLE:-}" ]; then
        kill "${PID_STYLE}" 2>/dev/null || true
    fi

    if [ -n "${PID_LINK:-}" ]; then
        wait "${PID_LINK}" 2>/dev/null || true
    fi
    if [ -n "${PID_MODULE:-}" ]; then
        wait "${PID_MODULE}" 2>/dev/null || true
    fi
    if [ -n "${PID_STYLE:-}" ]; then
        wait "${PID_STYLE}" 2>/dev/null || true
    fi

    exit "${status}"
}

trap cleanup INT TERM EXIT

# Run both commerce variants in the same container:
# - 3004: link CSS strategy
# - 3003: module CSS strategy
marketplace-api --port 3004 --css link --no-tls &
PID_LINK=$!

marketplace-api --port 3003 --css module --no-tls &
PID_MODULE=$!

marketplace-api --port 3002 --css style --no-tls &
PID_STYLE=$!

status=0
while kill -0 "${PID_LINK}" 2>/dev/null && kill -0 "${PID_MODULE}" 2>/dev/null && kill -0 "${PID_STYLE}" 2>/dev/null; do
    sleep 1
done

if ! kill -0 "${PID_LINK}" 2>/dev/null; then
    wait "${PID_LINK}" || status=$?
fi

if ! kill -0 "${PID_MODULE}" 2>/dev/null; then
    wait "${PID_MODULE}" || status=$?
fi

if ! kill -0 "${PID_STYLE}" 2>/dev/null; then
    wait "${PID_STYLE}" || status=$?
fi

exit "${status}"
