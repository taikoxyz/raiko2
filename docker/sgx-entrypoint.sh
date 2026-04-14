#!/usr/bin/env bash

exec 2>&1
set -euo pipefail

RAIKO2_SGX_BIN=${RAIKO2_SGX_BIN:-/opt/raiko2-sgx/bin/raiko2-sgx-prover}
RAIKO2_SGX_APP_DIR=${RAIKO2_SGX_APP_DIR:-/opt/raiko2-sgx/bin}
RAIKO2_SGX_MANIFEST_TEMPLATE=${RAIKO2_SGX_MANIFEST_TEMPLATE:-/opt/raiko2-sgx/docker/raiko2-sgx-prover.manifest.template}
RAIKO2_SGX_CONFIG_DIR=${RAIKO2_SGX_CONFIG_DIR:-/var/lib/raiko2/sgx/config}
RAIKO2_SGX_SECRET_DIR=${RAIKO2_SGX_SECRET_DIR:-/var/lib/raiko2/sgx/secrets}
RAIKO2_SGX_LISTEN_ADDR=${RAIKO2_SGX_LISTEN_ADDR:-0.0.0.0:8080}
RAIKO2_SGX_MODE=${RAIKO2_SGX_MODE:-tee}
RAIKO2_SGX_FORK=${RAIKO2_SGX_FORK:-shasta}
RAIKO2_SGX_INSTANCE_ID=${RAIKO2_SGX_INSTANCE_ID:-}
GRAMINE_ENCLAVE_KEY=${GRAMINE_ENCLAVE_KEY:-/root/.config/gramine/enclave-key.pem}
GRAMINE_LOG_LEVEL=${GRAMINE_LOG_LEVEL:-error}
PCCS_HOST=${PCCS_HOST:-host.docker.internal:8081}

mkdir -p "$RAIKO2_SGX_CONFIG_DIR" "$RAIKO2_SGX_SECRET_DIR"

if [[ -f /etc/sgx_default_qcnl.conf ]]; then
    sed -i "s#https://localhost:8081#https://${PCCS_HOST}#g" /etc/sgx_default_qcnl.conf || true
fi

if [[ -x /restart_aesm.sh ]]; then
    /restart_aesm.sh
fi

prepare_gramine_app() {
    if [[ "$RAIKO2_SGX_MODE" == "native" ]]; then
        return
    fi

    if [[ ! -f "$GRAMINE_ENCLAVE_KEY" ]]; then
        echo "missing Gramine enclave key: $GRAMINE_ENCLAVE_KEY" >&2
        exit 1
    fi

    cd "$RAIKO2_SGX_APP_DIR"
    gramine-manifest \
        -Dlog_level="$GRAMINE_LOG_LEVEL" \
        -Darch_libdir=/lib/x86_64-linux-gnu \
        "$RAIKO2_SGX_MANIFEST_TEMPLATE" \
        raiko2-sgx-prover.manifest
    gramine-sgx-sign \
        --key "$GRAMINE_ENCLAVE_KEY" \
        --manifest raiko2-sgx-prover.manifest \
        --output raiko2-sgx-prover.manifest.sgx
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
    prepare_gramine_app
    if [[ -n "$RAIKO2_SGX_INSTANCE_ID" ]]; then
        run_prover serve --listen-addr "$RAIKO2_SGX_LISTEN_ADDR" --fork "$RAIKO2_SGX_FORK" --instance-id "$RAIKO2_SGX_INSTANCE_ID"
    else
        run_prover serve --listen-addr "$RAIKO2_SGX_LISTEN_ADDR" --fork "$RAIKO2_SGX_FORK"
    fi
fi

case "$1" in
--init|init|bootstrap)
    shift
    prepare_gramine_app
    run_prover bootstrap "$@"
    ;;
--check|check)
    shift
    prepare_gramine_app
    run_prover check "$@"
    ;;
serve|server)
    shift
    prepare_gramine_app
    if [[ -n "$RAIKO2_SGX_INSTANCE_ID" ]]; then
        run_prover serve --listen-addr "$RAIKO2_SGX_LISTEN_ADDR" --fork "$RAIKO2_SGX_FORK" --instance-id "$RAIKO2_SGX_INSTANCE_ID" "$@"
    else
        run_prover serve --listen-addr "$RAIKO2_SGX_LISTEN_ADDR" --fork "$RAIKO2_SGX_FORK" "$@"
    fi
    ;;
*)
    prepare_gramine_app
    run_prover "$@"
    ;;
esac
