# Command Reference

Use these commands exactly unless the user explicitly overrides tag or backend.

## 1. Inspect Current Production State

```bash
kubectl config current-context
kubectl -n tolba-raiko2-host get deploy raiko2 -o jsonpath='{.metadata.annotations.deployment\.kubernetes\.io/revision} {.spec.template.spec.containers[0].image}{"\n"}'
kubectl -n tolba-raiko2-host get pods -l app=raiko2 -o wide
git status --short
```

## 2. Check Whether Register Is Needed

```bash
cargo run -r -p xtask -- register-image --profile hoodi-shasta --backend all
```

Apply only when explicitly requested and when `PRIVATE_KEY` is present:

```bash
PRIVATE_KEY=0x... cargo run -r -p xtask -- register-image --profile hoodi-shasta --backend all --apply
```

## 3. Generate Default Tag

```bash
date +tolba-%Y%m%d-%H%M
```

## 4. Build And Push The Runtime Image

```bash
cargo run -r -p xtask -- release-image all \
  --tag <tag> \
  --repository us-docker.pkg.dev/evmchain/images/raiko2 \
  --namespace tolba-raiko2-host \
  --deployment raiko2 \
  --container raiko2
```

Capture the pushed digest from the command output.

## 5. Roll Out The New Digest

```bash
kubectl set image deployment/raiko2 -n tolba-raiko2-host \
  raiko2=us-docker.pkg.dev/evmchain/images/raiko2@sha256:<digest>

kubectl rollout status deployment/raiko2 -n tolba-raiko2-host
```

## 6. Confirm Live Pod Digest

```bash
kubectl -n tolba-raiko2-host get deploy raiko2 -o jsonpath='{.metadata.annotations.deployment\.kubernetes\.io/revision} {.spec.template.spec.containers[0].image}{"\n"}'
kubectl -n tolba-raiko2-host get pods -l app=raiko2 -o wide
kubectl -n tolba-raiko2-host get pod <pod-name> -o jsonpath='{.status.containerStatuses[0].imageID}{"\n"}'
```

## 7. Passive Smoke

```bash
curl -sf http://34.87.10.238:8080/ready
curl -sf http://34.87.10.238:8080/metrics | head -n 30
```

Optional metric-family check:

```bash
curl -sf http://34.87.10.238:8080/metrics | rg 'raiko2_request_registrations_total|raiko2_stage_task_duration_seconds|raiko2_stage_tasks_inflight|raiko2_external_submission_total'
```

If the optional metric-family check is empty but `/metrics` itself is reachable, explain that the
new pod may not have seen request traffic yet.

## 8. Rollout Failure Diagnostics

```bash
kubectl -n tolba-raiko2-host get pods -l app=raiko2 -o wide
kubectl -n tolba-raiko2-host describe deployment raiko2
kubectl -n tolba-raiko2-host describe pod <pod-name>
kubectl -n tolba-raiko2-host logs deploy/raiko2 --since=5m
```
