#!/usr/bin/env bash

set -euo pipefail

sigstruct_json_path="${1:?missing sigstruct json path}"
output_path="${2:?missing output path}"

python3 - "${sigstruct_json_path}" "${output_path}" <<'PY'
import json
import pathlib
import sys

input_path = pathlib.Path(sys.argv[1])
output_path = pathlib.Path(sys.argv[2])

source = json.loads(input_path.read_text())

required = ["mr_enclave", "mr_signer", "isv_prod_id", "isv_svn", "debug_enclave"]
missing = [key for key in required if key not in source]
if missing:
    raise SystemExit(f"missing required sigstruct fields: {', '.join(missing)}")

metadata = {
    "mr_enclave": source["mr_enclave"],
    "mr_signer": source["mr_signer"],
    "isv_prod_id": source["isv_prod_id"],
    "isv_svn": source["isv_svn"],
    "debug_enclave": source["debug_enclave"],
}

if "date" in source:
    metadata["date"] = source["date"]

output_path.parent.mkdir(parents=True, exist_ok=True)
output_path.write_text(json.dumps(metadata, indent=2, sort_keys=True) + "\n")
PY
