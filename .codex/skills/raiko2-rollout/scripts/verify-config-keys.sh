#!/usr/bin/env bash
set -euo pipefail

expected_context="${RAIKO2_EXPECTED_CONTEXT:-gke_evmchain_asia-southeast1_raiko2-zk-sg-cluster}"
host_namespace="${RAIKO2_HOST_NAMESPACE:-tolba-raiko2-host}"
host_deployment="${RAIKO2_HOST_DEPLOYMENT:-raiko2}"
host_config_secret="${RAIKO2_HOST_CONFIG_SECRET:-raiko2-risc0-boundless-config}"
agent_namespace="${RAIKO2_AGENT_NAMESPACE:-tolba-raiko2-zk-agent}"
agent_secret="${RAIKO2_AGENT_SECRET:-raiko2-agent-secrets}"
agent_signer_key="${RAIKO2_AGENT_SIGNER_KEY:-boundlessSignerKey}"

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 2
  fi
}

secret_value() {
  local namespace="$1"
  local secret="$2"
  local key="$3"

  kubectl -n "$namespace" get secret "$secret" -o "jsonpath={.data.${key}}" | base64 -d
}

wallet_address() {
  local private_key="$1"

  cast wallet address --private-key "$private_key"
}

boundless_signer_key() {
  secret_value "$host_namespace" "$host_config_secret" "config\\.toml" |
    perl -ne '
      if (/^\s*\[prover\.boundless\]\s*$/) { $in = 1; next }
      if (/^\s*\[/) { $in = 0 }
      if ($in && /^\s*signer_key\s*=\s*"([^"]+)"/) { print $1; exit }
    '
}

deployment_secret_key_ref() {
  local env_name="$1"

  kubectl -n "$host_namespace" get deploy "$host_deployment" \
    -o "jsonpath={range .spec.template.spec.containers[*].env[?(@.name==\"${env_name}\")]}{.valueFrom.secretKeyRef.name}:{.valueFrom.secretKeyRef.key}{\"\\n\"}{end}"
}

require_command kubectl
require_command cast
require_command base64
require_command perl

current_context="$(kubectl config current-context)"
if [[ "$current_context" != "$expected_context" ]]; then
  echo "context_mismatch expected=$expected_context actual=$current_context" >&2
  exit 1
fi

host_signer_key="$(boundless_signer_key)"
if [[ -z "$host_signer_key" ]]; then
  echo "missing [prover.boundless].signer_key in $host_namespace/$host_config_secret:config.toml" >&2
  exit 1
fi
host_signer_address="$(wallet_address "$host_signer_key")"

agent_key="$(secret_value "$agent_namespace" "$agent_secret" "$agent_signer_key")"
if [[ -z "$agent_key" ]]; then
  echo "missing agent signer key at $agent_namespace/$agent_secret:$agent_signer_key" >&2
  exit 1
fi
agent_signer_address="$(wallet_address "$agent_key")"

mapfile -t network_refs < <(deployment_secret_key_ref NETWORK_PRIVATE_KEY)
if [[ "${#network_refs[@]}" -ne 1 || -z "${network_refs[0]}" || "${network_refs[0]}" != *":"* ]]; then
  echo "missing NETWORK_PRIVATE_KEY secretKeyRef in $host_namespace/$host_deployment deployment env" >&2
  exit 1
fi
network_ref="${network_refs[0]}"
network_secret="${network_ref%%:*}"
network_key="${network_ref#*:}"
if [[ -z "$network_secret" || -z "$network_key" ]]; then
  echo "invalid NETWORK_PRIVATE_KEY secretKeyRef in $host_namespace/$host_deployment deployment env: $network_ref" >&2
  exit 1
fi
network_private_key="$(secret_value "$host_namespace" "$network_secret" "$network_key")"
if [[ -z "$network_private_key" ]]; then
  echo "missing NETWORK_PRIVATE_KEY value at $host_namespace/$network_secret:$network_key" >&2
  exit 1
fi
network_address="$(wallet_address "$network_private_key")"

echo "context=$current_context"
echo "boundless_signer_source=$host_namespace/$host_config_secret:config.toml [prover.boundless].signer_key"
echo "boundless_signer_address=$host_signer_address"
echo "agent_signer_source=$agent_namespace/$agent_secret:$agent_signer_key"
echo "agent_signer_address=$agent_signer_address"
echo "network_key_source=$host_namespace/$network_ref"
echo "network_key_address=$network_address"

if [[ "$host_signer_address" != "$agent_signer_address" ]]; then
  echo "boundless_signer_mismatch expected_agent=$agent_signer_address actual=$host_signer_address" >&2
  exit 1
fi

if [[ "$host_signer_address" == "$network_address" ]]; then
  echo "boundless_signer_reuses_network_key address=$host_signer_address" >&2
  exit 1
fi

echo "config_key_check=ok"
