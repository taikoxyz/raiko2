#!/usr/bin/env bash

exec 2>&1
set -euo pipefail

RAIKO2_SGX_BIN=${RAIKO2_SGX_BIN:-/opt/raiko2-sgx/bin/raiko2-sgx-prover}
RAIKO2_SGX_APP_DIR=${RAIKO2_SGX_APP_DIR:-/opt/raiko2-sgx/bin}
RAIKO2_SGX_CONFIG_DIR=${RAIKO2_SGX_CONFIG_DIR:-/var/lib/raiko2/sgx/config}
RAIKO2_SGX_SECRET_DIR=${RAIKO2_SGX_SECRET_DIR:-/var/lib/raiko2/sgx/secrets}
RAIKO2_SGX_LISTEN_ADDR=${RAIKO2_SGX_LISTEN_ADDR:-0.0.0.0:8080}
RAIKO2_SGX_MODE=${RAIKO2_SGX_MODE:-tee}
RAIKO2_SGX_FORK=${RAIKO2_SGX_FORK:-shasta}
RAIKO2_SGX_INSTANCE_ID=${RAIKO2_SGX_INSTANCE_ID:-}
PCCS_HOST=${PCCS_HOST:-host.docker.internal:8081}

mkdir -p "$RAIKO2_SGX_CONFIG_DIR" "$RAIKO2_SGX_SECRET_DIR"

if [[ -f /etc/sgx_default_qcnl.conf ]]; then
    sed -i "s#https://localhost:8081#https://${PCCS_HOST}#g" /etc/sgx_default_qcnl.conf || true
fi

if [[ -x /restart_aesm.sh ]]; then
    /restart_aesm.sh
fi

require_gramine_app() {
    if [[ "$RAIKO2_SGX_MODE" == "native" ]]; then
        return
    fi

    local required=(
        "${RAIKO2_SGX_APP_DIR}/raiko2-sgx-prover.manifest"
        "${RAIKO2_SGX_APP_DIR}/raiko2-sgx-prover.manifest.sgx"
        "${RAIKO2_SGX_APP_DIR}/raiko2-sgx-prover.sig"
        "/opt/raiko2-sgx/etc/attestation.raiko2.json"
    )
    local missing=0
    for path in "${required[@]}"; do
        if [[ ! -f "${path}" ]]; then
            echo "missing prebuilt SGX runtime artifact: ${path}" >&2
            missing=1
        fi
    done
    if [[ "${missing}" -ne 0 ]]; then
        echo "raiko2-sgx image must be signed at build time before tee startup" >&2
        exit 1
    fi
}

run_prover() {
    if [[ "$RAIKO2_SGX_MODE" == "native" ]]; then
        exec "$RAIKO2_SGX_BIN" \
            --mode native \
            --config-dir "$RAIKO2_SGX_CONFIG_DIR" \
            --secret-dir "$RAIKO2_SGX_SECRET_DIR" \
            "$@"
    fi

    cd "$RAIKO2_SGX_APP_DIR"
    exec gramine-sgx raiko2-sgx-prover \
        --mode tee \
        --config-dir "$RAIKO2_SGX_CONFIG_DIR" \
        --secret-dir "$RAIKO2_SGX_SECRET_DIR" \
        "$@"
}

if [[ $# -eq 0 ]]; then
    require_gramine_app
    if [[ -n "$RAIKO2_SGX_INSTANCE_ID" ]]; then
        run_prover serve --listen-addr "$RAIKO2_SGX_LISTEN_ADDR" --fork "$RAIKO2_SGX_FORK" --instance-id "$RAIKO2_SGX_INSTANCE_ID"
    else
        run_prover serve --listen-addr "$RAIKO2_SGX_LISTEN_ADDR" --fork "$RAIKO2_SGX_FORK"
    fi
fi

case "$1" in
--init|init|bootstrap)
    shift
    require_gramine_app
    run_prover bootstrap "$@"
    ;;
--check|check)
    shift
    require_gramine_app
    run_prover check "$@"
    ;;
serve|server)
    shift
    require_gramine_app
    if [[ -n "$RAIKO2_SGX_INSTANCE_ID" ]]; then
        run_prover serve --listen-addr "$RAIKO2_SGX_LISTEN_ADDR" --fork "$RAIKO2_SGX_FORK" --instance-id "$RAIKO2_SGX_INSTANCE_ID" "$@"
    else
        run_prover serve --listen-addr "$RAIKO2_SGX_LISTEN_ADDR" --fork "$RAIKO2_SGX_FORK" "$@"
    fi
    ;;
*)
    require_gramine_app
    run_prover "$@"
    ;;
esac
