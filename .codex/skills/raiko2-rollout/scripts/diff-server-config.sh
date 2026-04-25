#!/usr/bin/env bash
set -euo pipefail

CONTEXT="gke_evmchain_asia-southeast1_raiko2-zk-sg-cluster"
NAMESPACE="tolba-raiko2-host"
DEPLOYMENT="raiko2"
CONTAINER="raiko2"
SECRET="raiko2-risc0-boundless-config"

usage() {
  cat <<'USAGE'
Usage:
  diff-server-config.sh snapshot
  diff-server-config.sh diff <before-snapshot> <after-snapshot>

Prints a redacted server configuration snapshot or a unified diff between
two redacted snapshots. Sensitive values are replaced with short sha256
fingerprints so equality and drift are visible without exposing secrets.
USAGE
}

redact_toml() {
  perl -MDigest::SHA=sha256_hex -pe '
    my $terms = qr/key|secret|token|password|private|credential|rpc|url|dsn|auth|bearer|api|cookie|session/i;
    my $bare_key = qr/[A-Za-z0-9_.-]*(?:$terms)[A-Za-z0-9_.-]*/;
    my $quoted_key = qr/"(?:[^"\\]|\\.)*(?:$terms)(?:[^"\\]|\\.)*"/;
    my $sensitive = qr/(?:$bare_key|$quoted_key)/;
    s~
      (
        (?:^\s*|[,{]\s*)
        $sensitive
        \s*=\s*
      )
      (
        "(?:[^"\\]|\\.)*"
        |
        [^,\]\}\n#]+
      )
    ~
      $1 . "\"<redacted:sha256=" . substr(sha256_hex($2), 0, 12) . ">\""
    ~gex;
  '
}

redact_env() {
  jq -r '
    .spec.template.spec.containers[]
    | select(.name == "'"$CONTAINER"'")
    | .env[]?
    | if has("value") then
        if (.name | test("KEY|SECRET|TOKEN|PASSWORD|PRIVATE|CREDENTIAL|RPC|URL|DSN|AUTH|BEARER|API|COOKIE|SESSION"; "i")) then
          "\(.name)=<redacted:sha256=\(.value | @base64)>"
        else
          "\(.name)=\(.value)"
        end
      else
        "\(.name)=<valueFrom>"
      end
  ' | while IFS= read -r line; do
    if [[ "$line" =~ ^([^=]+)=\<redacted:sha256=([^[:space:]]+)\>$ ]]; then
      name="${BASH_REMATCH[1]}"
      encoded="${BASH_REMATCH[2]}"
      hash=$(printf '%s' "$encoded" | base64 -d | sha256sum | awk '{print substr($1, 1, 12)}')
      printf '%s=<redacted:sha256=%s>\n' "$name" "$hash"
    else
      printf '%s\n' "$line"
    fi
  done
}

snapshot() {
  current_context=$(kubectl config current-context)
  if [[ "$current_context" != "$CONTEXT" ]]; then
    echo "unexpected kubernetes context: $current_context" >&2
    exit 1
  fi

  echo "# deployment"
  kubectl --context "$CONTEXT" -n "$NAMESPACE" get deploy "$DEPLOYMENT" -o json \
    | jq -r '
      "revision=\(.metadata.annotations["deployment.kubernetes.io/revision"] // "unknown")",
      "image=\(.spec.template.spec.containers[] | select(.name == "'"$CONTAINER"'") | .image)",
      "replicas=\(.spec.replicas // 0)"
    '

  echo
  echo "# container env"
  kubectl --context "$CONTEXT" -n "$NAMESPACE" get deploy "$DEPLOYMENT" -o json | redact_env

  echo
  echo "# config.toml"
  kubectl --context "$CONTEXT" -n "$NAMESPACE" get secret "$SECRET" \
    -o jsonpath='{.data.config\.toml}' \
    | base64 -d \
    | redact_toml
}

case "${1:-}" in
  snapshot)
    snapshot
    ;;
  diff)
    if [[ $# -ne 3 ]]; then
      usage >&2
      exit 2
    fi
    diff -u "$2" "$3" || true
    ;;
  -h|--help|help|"")
    usage
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac
