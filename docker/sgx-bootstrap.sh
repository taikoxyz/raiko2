#!/usr/bin/env bash

exec 2>&1
set -euo pipefail

exec /opt/raiko2-sgx/bin/entrypoint.sh --init "$@"
