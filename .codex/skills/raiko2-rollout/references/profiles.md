# Profiles

## `tolba-prod`

This skill currently supports one production profile only.

### Rollout Target

- Kubernetes context:
  `gke_evmchain_asia-southeast1_raiko2-zk-sg-cluster`
- Namespace: `tolba-raiko2-host`
- Deployment: `raiko2`
- Container: `raiko2`
- Runtime config secret: `raiko2-risc0-boundless-config`
- Image repository:
  `us-docker.pkg.dev/evmchain/images/raiko2`

### Key Identity Target

- Boundless signer source:
  `tolba-raiko2-host/raiko2-risc0-boundless-config:config.toml [prover.boundless].signer_key`
- Expected signer source:
  `tolba-raiko2-zk-agent/raiko2-agent-secrets:boundlessSignerKey`
- Network key source:
  deployment env `NETWORK_PRIVATE_KEY`
- The Boundless signer address must match the old agent signer address.
- The Boundless signer address must not match the host network/SP1 key address.

### Register Target

- `xtask register-image` profile: `hoodi-shasta`
- Default backend: `all`
- Preferred private-key source: repo-root `.env`
- Fallback private-key env var: `PRIVATE_KEY`

### Passive Smoke Target

Passive smoke hits `raiko2` over HTTP (`/ready`, `/metrics`). Treat this as an **internal-only**
check: real hosts, ports, and any IPv4/IPv6 literals live **only** in your internal runbook—this
repo keeps placeholders only (no public customer-facing DNS names here).

- Base URL: `http://<internal-smoke-host>:<internal-smoke-port>`
- Readiness URL: `http://<internal-smoke-host>:<internal-smoke-port>/ready`
- Metrics URL: `http://<internal-smoke-host>:<internal-smoke-port>/metrics`

When following `references/commands.md`, set `RAIKO2_SMOKE_BASE_URL` to the internal base URL from
your runbook (often after `kubectl port-forward` or an equivalent private path).

### Operational Defaults

- Build from the current worktree unless the user explicitly says otherwise.
- Use digest-based rollout, not mutable tags.
- Run `register-image` dry-run **after** `release-image` succeeds (against the ELFs packaged into
  that image build).
- Default smoke depth is passive only.

### Non-Goals For This Version

- No staging profile
- No dev profile
- No automatic active proof smoke
- No automatic Grafana import
