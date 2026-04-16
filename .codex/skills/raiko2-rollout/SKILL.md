---
name: raiko2-rollout
description: Use when publishing or verifying the current raiko2 hosted service in the tolba production deployment from this repository
---

# Raiko2 Rollout

## Overview

Roll out the current `raiko2` tree to the canonical tolba production deployment with one fixed,
repeatable sequence:

1. confirm environment and current live state
2. show worktree risk
3. build and push the runtime image
4. check whether guest registration is needed
5. update the GKE deployment and wait for rollout
6. run passive smoke checks
7. report the exact outcome

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

### 2. Worktree Gate

Always show whether the worktree is dirty.

Default policy is `warn + allow`:

- do not block the rollout just because the worktree is dirty
- explicitly state that the image is built from the current worktree, not a single clean commit
- include that risk again in the final summary

Never hide unrelated local modifications.

### 3. Release Image

Generate a default tag in the format `tolba-YYYYMMDD-HHMM` unless the user provides one.

Use the canonical `xtask release-image` entrypoint with:

- backend `all`
- production repository
- production namespace/deployment/container

Capture the pushed digest from the command output. If image build or push fails, stop immediately
and do not attempt registration or `kubectl set image`.

### 4. Register Check

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

### 5. GKE Rollout

Use the pushed digest, not a mutable tag, for the deployment update.

Required rollout steps:

- `kubectl set image`
- `kubectl rollout status`
- re-read deployment revision and image
- re-read pod list and pod `imageID`

Success requires the live pod `imageID` to match the pushed digest.

If rollout fails, collect the failure diagnostics from `references/checklists.md` before reporting.

### 6. Passive Smoke

Default smoke depth is passive only. Run:

- deployment revision check
- pod readiness check
- `GET /ready`
- `GET /metrics`

Smoke does **not** send a proof request in this skill.

Interpret `/metrics` this way:

- if the endpoint is unreachable, smoke failed
- if it is reachable but custom `raiko2_*` families are absent, do not fail the rollout
- explain that a fresh pod may expose only process metrics until new request traffic materializes

### 7. Final Report

Always report:

- previous revision and image
- new revision, tag, and image digest
- whether register was checked or applied
- rollout result
- smoke result
- dirty-worktree warning if applicable

Use the corresponding template from `references/checklists.md` and fill in concrete values.

## Stop Conditions

Stop and escalate instead of improvising when any of these happen:

- cluster context does not match the production profile
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
