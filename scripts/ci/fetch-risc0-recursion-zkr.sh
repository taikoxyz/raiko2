#!/usr/bin/env bash
# Fetch the recursion programs that risc0-circuit-recursion's build script embeds, and point the
# build script at them through RECURSION_SRC_PATH.
#
# risc0-circuit-recursion 4.0.4 downloads recursion_zkr.zip from risc0-artifacts.s3.us-west-2,
# which RISC Zero retired (HTTP 403 since 2026-11). The same file is served by their current
# artifact host and by Git LFS in the risc0 repo. The build script re-checks the sha256 below, so
# a bad copy cannot be embedded.
#
# Do not "fix" this by bumping to risc0-circuit-recursion 4.1.0: that release ships different
# recursion programs, so ALLOWED_CONTROL_ROOT and BN254_IDENTITY_CONTROL_ID change and Groth16
# proofs stop matching the deployed RISC Zero verifier.
#
# Keep ZKR_SHA256 in sync with the `ADD --checksum` lines in Dockerfile and Dockerfile.cuda.
#
# Usage: scripts/ci/fetch-risc0-recursion-zkr.sh <destination-file>
# In GitHub Actions the script also exports RECURSION_SRC_PATH through $GITHUB_ENV.
# CARGO_LOCK selects the lockfile to check (default: this checkout's Cargo.lock), for workflows that
# run this copy of the script against another checkout.
set -euo pipefail

readonly CRATE_VERSION="4.0.4"
readonly ZKR_SHA256="744b999f0a35b3c86753311c7efb2a0054be21727095cf105af6ee7d3f4d8849"
# risc0 commit that published risc0-circuit-recursion 4.0.4 (its .cargo_vcs_info.json).
readonly RISC0_COMMIT="ba889fc049830b777de63fddbeea5f9776618d7d"
readonly SOURCES=(
  "https://artifacts.risczero.net/zkr/${ZKR_SHA256}.zip"
  "https://media.githubusercontent.com/media/risc0/risc0/${RISC0_COMMIT}/risc0/circuit/recursion/src/recursion_zkr.zip"
)

if [ "$#" -ne 1 ]; then
  echo "usage: $0 <destination-file>" >&2
  exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cargo_lock="${CARGO_LOCK:-$repo_root/Cargo.lock}"
dest="$1"
mkdir -p "$(dirname "$dest")"
dest="$(cd "$(dirname "$dest")" && pwd)/$(basename "$dest")"

locked="$(awk '$0 == "name = \"risc0-circuit-recursion\"" { getline; gsub(/version = |"/, ""); print; exit }' "$cargo_lock")"
if [ -z "$locked" ]; then
  echo "risc0-circuit-recursion is not in $cargo_lock; nothing to fetch"
  exit 0
fi
if [ "$locked" != "$CRATE_VERSION" ]; then
  echo "$cargo_lock has risc0-circuit-recursion $locked, but this script pins the $CRATE_VERSION" \
    "recursion_zkr.zip. Update CRATE_VERSION/ZKR_SHA256/RISC0_COMMIT here and in the Dockerfiles." >&2
  exit 1
fi

verified() {
  [ -f "$1" ] && echo "${ZKR_SHA256}  $1" | sha256sum --check --status -
}

if ! verified "$dest"; then
  tmp="$dest.partial"
  for url in "${SOURCES[@]}"; do
    echo "Downloading $url"
    if curl --fail --silent --show-error --location --retry 3 --retry-delay 5 --output "$tmp" "$url" &&
      verified "$tmp"; then
      mv "$tmp" "$dest"
      break
    fi
    echo "::warning::recursion_zkr.zip from $url failed to download or verify" >&2
    rm -f "$tmp"
  done
fi

if ! verified "$dest"; then
  echo "::error::Could not fetch a recursion_zkr.zip with sha256 $ZKR_SHA256" >&2
  exit 1
fi
echo "recursion_zkr.zip verified at $dest"

if [ -n "${GITHUB_ENV:-}" ]; then
  echo "RECURSION_SRC_PATH=$dest" >>"$GITHUB_ENV"
fi
