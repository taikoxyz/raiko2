#!/usr/bin/env bash

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
helper="${repo_root}/docker/write-attestation-metadata.sh"
input_json="${repo_root}/docker/testdata/gramine-sigstruct-view.sample.json"
tmp_dir=$(mktemp -d)
output_json="${tmp_dir}/attestation.raiko2.json"

cleanup() {
  rm -rf "${tmp_dir}"
}
trap cleanup EXIT

[[ -x "${helper}" ]] || {
  echo "missing helper script: ${helper}" >&2
  exit 1
}

"${helper}" "${input_json}" "${output_json}"

[[ -f "${output_json}" ]] || {
  echo "missing output file: ${output_json}" >&2
  exit 1
}

python3 - "${output_json}" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
data = json.loads(path.read_text())

assert data["mr_enclave"] == "81a675e9a408818b430be4b259f3e11e6f8cacdb4c971c3114ee79fe53076893"
assert data["mr_signer"] == "0dedbe47afb6955e5f6109637c1fbd9cc4b4e073e1396da8ce2091075e5b0a3b"
assert data["isv_prod_id"] == 0
assert data["isv_svn"] == 2
assert data["debug_enclave"] is False
assert data["date"] == "2026-05-11"
PY

echo "attestation metadata helper test passed"
