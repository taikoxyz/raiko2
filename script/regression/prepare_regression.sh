#!/usr/bin/env bash
set -euo pipefail

cargo build --release -p preflight -p guest-launcher
