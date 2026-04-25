# Command Reference

Use these commands exactly unless the user explicitly overrides tag or backend.

## 1. Inspect Current Production State

```bash
kubectl config current-context
kubectl -n tolba-raiko2-host get deploy raiko2 -o jsonpath='{.metadata.annotations.deployment\.kubernetes\.io/revision} {.spec.template.spec.containers[0].image}{"\n"}'
kubectl -n tolba-raiko2-host get pods -l app=raiko2 -o wide
git status --short
```

## 2. Verify Config And Key Identity

Run this before building and again after the rollout. The script prints derived addresses and
source fields only; it must not print private keys.

```bash
bash .codex/skills/raiko2-rollout/scripts/verify-config-keys.sh
```

The check fails if:

- Kubernetes context is not the `tolba-prod` context.
- `[prover.boundless].signer_key` does not derive the old agent signer address.
- `[prover.boundless].signer_key` reuses the host `NETWORK_PRIVATE_KEY` address.

## 3. Capture Redacted Server Config Snapshot

Run this before rollout. Run it again after any live config change and after rollout status succeeds.

```bash
mkdir -p target/rollout-config
.codex/skills/raiko2-rollout/scripts/diff-server-config.sh snapshot \
  > target/rollout-config/server-config.before.txt
```

After config changes or rollout:

```bash
.codex/skills/raiko2-rollout/scripts/diff-server-config.sh snapshot \
  > target/rollout-config/server-config.after.txt
.codex/skills/raiko2-rollout/scripts/diff-server-config.sh diff \
  target/rollout-config/server-config.before.txt \
  target/rollout-config/server-config.after.txt
```

The diff is intentionally redacted. It must be used to confirm expected config changes or detect
unexpected drift. Do not decode and print raw Kubernetes secrets.

When a rollout includes queue config schema changes, the redacted config snapshot must not contain
the removed retry table or key, and the task timeout must be explicit:

```bash
if rg '^\s*\[queue\.retry\]|^\s*retry\s*=' target/rollout-config/server-config.after.txt; then
  echo "legacy queue retry config remains" >&2
  exit 1
fi
rg '^\s*task_timeout_secs\s*=' target/rollout-config/server-config.after.txt
```

## 4. Generate Default Tag

```bash
date +tolba-%Y%m%d-%H%M
```

## 5. Build And Push The Runtime Image

```bash
cargo run -r -p xtask -- release-image all \
  --tag <tag> \
  --repository us-docker.pkg.dev/evmchain/images/raiko2 \
  --namespace tolba-raiko2-host \
  --deployment raiko2 \
  --container raiko2
```

Capture the pushed digest from the command output.

## 6. Check Whether Register Is Needed

Run this after `release-image`, because `xtask release-image` refreshes the checked-in guest ELF
artifacts before packaging the runtime image.

```bash
cargo run -r -p xtask -- register-image --profile hoodi-shasta --backend all
```

Resolve `PRIVATE_KEY` from the repo-root `.env` first. If `.env` does not provide it, fall back to
the current process environment.

Preferred apply flow:

```bash
set -a
[ -f .env ] && . ./.env
set +a
cargo run -r -p xtask -- register-image --profile hoodi-shasta --backend all --apply
```

Fallback apply flow when `.env` does not define `PRIVATE_KEY`:

```bash
PRIVATE_KEY=0x... cargo run -r -p xtask -- register-image --profile hoodi-shasta --backend all --apply
```

## 7. Roll Out The New Digest

```bash
kubectl set image deployment/raiko2 -n tolba-raiko2-host \
  raiko2=us-docker.pkg.dev/evmchain/images/raiko2@sha256:<digest>

kubectl rollout status deployment/raiko2 -n tolba-raiko2-host
```

## 8. Confirm Live Pod Digest

```bash
kubectl -n tolba-raiko2-host get deploy raiko2 -o jsonpath='{.metadata.annotations.deployment\.kubernetes\.io/revision} {.spec.template.spec.containers[0].image}{"\n"}'
kubectl -n tolba-raiko2-host get pods -l app=raiko2 -o wide
kubectl -n tolba-raiko2-host get pod <pod-name> -o jsonpath='{.status.containerStatuses[0].imageID}{"\n"}'
```

## 9. Passive Smoke

Internal-only: set the base URL from your **internal runbook** (never commit live addresses here).
Typical access is `kubectl port-forward` into the cluster or another private path—see infra docs.

```bash
export RAIKO2_SMOKE_BASE_URL='http://<internal-smoke-host>:<internal-smoke-port>'
```

```bash
curl -sf "${RAIKO2_SMOKE_BASE_URL}/ready"
curl -sf "${RAIKO2_SMOKE_BASE_URL}/metrics" | head -n 30
```

Optional metric-family check:

```bash
curl -sf "${RAIKO2_SMOKE_BASE_URL}/metrics" | rg 'raiko2_request_registrations_total|raiko2_stage_task_duration_seconds|raiko2_stage_tasks_inflight|raiko2_external_submission_total'
```

If the optional metric-family check is empty but `/metrics` itself is reachable, explain that the
new pod may not have seen request traffic yet.

## 10. Rollout Failure Diagnostics

```bash
kubectl -n tolba-raiko2-host get pods -l app=raiko2 -o wide
kubectl -n tolba-raiko2-host describe deployment raiko2
kubectl -n tolba-raiko2-host describe pod <pod-name>
kubectl -n tolba-raiko2-host logs deploy/raiko2 --since=5m
```
