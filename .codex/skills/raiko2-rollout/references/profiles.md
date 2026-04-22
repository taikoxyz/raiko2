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

- Base URL: `http://34.87.10.238:8080`
- Readiness URL: `http://34.87.10.238:8080/ready`
- Metrics URL: `http://34.87.10.238:8080/metrics`

### Operational Defaults

- Build from the current worktree unless the user explicitly says otherwise.
- Use digest-based rollout, not mutable tags.
- Run `register-image` dry-run before `release-image`.
- Default smoke depth is passive only.

### Non-Goals For This Version

- No staging profile
- No dev profile
- No automatic active proof smoke
- No automatic Grafana import
