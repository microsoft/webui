#!/bin/sh
# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

set -eu

# The demo-shell binary handles everything:
#   1. Scans ./apps/*/demo.toml to discover all example apps
#   2. Assigns ports dynamically starting from --base-port
#   3. Spawns all child processes (webui-cli, node APIs, custom binaries)
#   4. Starts the reverse proxy + shell UI on the exposed port
#   5. Forwards SIGTERM to all children on shutdown

PORT="${PORT:-8080}"

exec demo-shell --port "${PORT}" --apps-dir ./apps --shell-dir ./shell
