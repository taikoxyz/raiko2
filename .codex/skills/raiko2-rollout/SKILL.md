---
name: raiko2-rollout
description: Use when publishing or verifying the current raiko2 hosted service in the tolba production deployment from this repository
---

# Raiko2 Rollout

## Overview

Roll out the current `raiko2` tree to the canonical tolba production deployment with one fixed,
repeatable sequence:

1. confirm environment and current live state
2. verify live config and key identity
3. capture safe server config state and decide whether config diffing is required
4. show worktree risk
5. build and push the runtime image
6. check whether guest registration is needed
7. update the GKE deployment and wait for rollout
8. re-verify config, key identity, and server config diff
9. run passive smoke checks
10. report the exact outcome

Load these references before acting:

- `references/profiles.md`
- `references/commands.md`
- `references/checklists.md`

## When to Use

Use this skill when the user asks to:

- roll out `raiko2`
- publish a new production image
- update the GKE deployment
- verify whether the current production rollout is healthy

Do not use this skill for:

- staging or ad-hoc environments
- local Docker testing
- code changes unrelated to deployment

This version supports exactly one environment profile: the current tolba production deployment.

## Defaults

Use the `tolba-prod` profile from `references/profiles.md`.

Default rollout parameters:

- backend: `all`
- register behavior: `check`
- smoke depth: `passive`
- worktree policy: `warn + allow`

If the user requests different namespace, deployment, cluster, or verifier profile, stop and ask
explicitly instead of guessing.

## Mandatory Flow

### 1. Preflight Context

Always gather these facts first:

- current repo path
- `kubectl config current-context`
- current deployment revision and image digest
- current pod list for the deployment
- `git status --short`

The cluster context must match the production profile. If it does not, stop before any build or
rollout action.

### 2. Config And Key Identity Gate

Run `.codex/skills/raiko2-rollout/scripts/verify-config-keys.sh` before building or registering.

This gate protects against the recurring failure mode where the host network/SP1 key is copied into
`[prover.boundless].signer_key`.

The check must pass with:

- current Kubernetes context equal to the production profile
- host `[prover.boundless].signer_key` deriving the old agent signer address
- host `[prover.boundless].signer_key` not deriving the same address as `NETWORK_PRIVATE_KEY`

If it fails, stop before build, register, or rollout and report the mismatch with derived addresses
only. Never print private keys.

### 3. Server Config Difference Gate

Always capture a redacted server config snapshot before rollout with:

```bash
.codex/skills/raiko2-rollout/scripts/diff-server-config.sh snapshot
```

Save the snapshot under `target/rollout-config/` or another ignored temporary path. The snapshot
includes deployment metadata, non-secret env values, and a redacted `config.toml` view. Sensitive
keys, secrets, private keys, tokens, RPCs, and URLs are replaced with short fingerprints; never print
or paste raw Kubernetes secret values.

Treat the rollout as config-related when any of these are true:

- local changes touch config schema, config defaults, config loading, CLI/env overrides, or
  `config*.toml`
- the operator edits the production config secret, deployment env, resource requests/limits,
  replica count, HPA, or mounted config
- the requested rollout is intended to change timeouts, workers, concurrency, RPC/provider
  settings, prover pricing, backend selection, or feature flags

For config-related rollouts:

- capture a before snapshot before any config mutation
- capture an after snapshot after the config mutation and again after rollout restart/status when a
  restart is required
- when queue schema changes are present, verify the live `config.toml` no longer contains
  `[queue.retry]` or `queue.retry`, and that `queue.task_timeout_secs` is set intentionally; the
  current server rejects unknown queue fields at startup
- run `diff-server-config.sh diff <before> <after>`
- explicitly report the redacted server config diff summary

For image-only rollouts:

- capture before and after snapshots
- confirm there was no unexpected server config diff beyond revision/image/pod-template metadata

If an unexpected config diff appears, stop before proceeding further unless the user confirms that
the diff is intended. Do not treat repo config examples as proof of production config; the live
server config comes from the Kubernetes secret and deployment env.

### 4. Worktree Gate

Always show whether the worktree is dirty.

Default policy is `warn + allow`:

- do not block the rollout just because the worktree is dirty
- explicitly state that the image is built from the current worktree, not a single clean commit
- include that risk again in the final summary

Never hide unrelated local modifications.

### 5. Release Image

Generate a default tag in the format `tolba-YYYYMMDD-HHMM` unless the user provides one.

Use the canonical `xtask release-image` entrypoint with:

- backend `all`
- production repository
- production namespace/deployment/container

Capture the pushed digest from the command output. If image build or push fails, stop immediately
and do not attempt registration or `kubectl set image`.

### 6. Register Check

After building the image, run the dry-run `register-image` command from the production profile.
`xtask release-image` prepares the checked-in guest ELFs first, so registration must be evaluated
against those final artifacts, not against a pre-build snapshot.

Interpret the result this way:

- no pending registrations: continue automatically
- pending registrations exist: stop the automatic flow before rollout and report exactly what needs
  registration

Default behavior is `check`, not `apply`.

For `apply`:

- require explicit user intent
- resolve `PRIVATE_KEY` from the repo-root `.env` first
- if `.env` does not provide it, fall back to the current process environment
- if neither source provides it, stop and report that `register-image --apply` is blocked

Never broadcast registration transactions by default.

### 7. GKE Rollout

Use the pushed digest, not a mutable tag, for the deployment update.

Required rollout steps:

- `kubectl set image`
- `kubectl rollout status`
- re-read deployment revision and image
- re-read pod list and pod `imageID`

Success requires the live pod `imageID` to match the pushed digest.

If rollout fails, collect the failure diagnostics from `references/checklists.md` before reporting.

### 8. Post-Rollout Config, Key Identity, And Config Diff Gate

Run `.codex/skills/raiko2-rollout/scripts/verify-config-keys.sh` again after rollout status succeeds
and after re-reading the new pod/image. This catches config secret drift or manual edits that happen
between the preflight check and the live pod start.

If it fails after rollout, collect smoke-failure diagnostics and report that rollout completed but
configuration validation failed.

Also capture a post-rollout redacted server config snapshot and compare it with the pre-rollout
snapshot. For config-related rollouts, verify that the diff matches the intended changes. For
image-only rollouts, verify that only expected deployment metadata changed. Report the diff result in
the final summary.

### 9. Passive Smoke

Default smoke depth is passive only. Run:

- deployment revision check
- pod readiness check
- `GET /ready` using the **internal** base URL from `references/profiles.md` (set
  `RAIKO2_SMOKE_BASE_URL` when using `references/commands.md`; repo text uses placeholders only—no
  literal IPs or public DNS names)
- `GET /metrics` on the same base URL

Smoke does **not** send a proof request in this skill.

Interpret `/metrics` this way:

- if the endpoint is unreachable, smoke failed
- if it is reachable but custom `raiko2_*` families are absent, do not fail the rollout
- explain that a fresh pod may expose only process metrics until new request traffic materializes

### 10. Final Report

Always report:

- previous revision and image
- new revision, tag, and image digest
- whether register was checked or applied
- config/key identity check result
- server config diff result
- rollout result
- smoke result
- dirty-worktree warning if applicable

Use the corresponding template from `references/checklists.md` and fill in concrete values.

## Stop Conditions

Stop and escalate instead of improvising when any of these happen:

- cluster context does not match the production profile
- config/key identity check fails before build
- unexpected server config diff appears during an image-only rollout
- `register-image` reports pending work and the user has not asked to apply it
- `PRIVATE_KEY` is unavailable from both the repo-root `.env` and the current environment for `register-image --apply`
- `xtask release-image` fails
- `kubectl rollout status` fails
- `/ready` fails
- `/metrics` is unreachable

When rollout or smoke fails, gather the exact diagnostics listed in `references/checklists.md`
before responding.

## Guardrails

- Prefer the documented `xtask` entrypoints over ad-hoc Docker or kubectl flows.
- Use immutable digests for rollout.
- Keep the flow production-only until this skill is extended with more profiles.
- Do not auto-run active proof smoke from this skill.
- Do not auto-commit, auto-push, or auto-create release notes.
