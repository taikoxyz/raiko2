# Checklists And Result Templates

## Successful Rollout

Confirm all of these before reporting success:

- Kubernetes context matches `tolba-prod`
- current deployment revision and image were captured before rollout
- config/key identity check passed before build
- redacted server config snapshot was captured before rollout
- queue config schema migration was checked when relevant: no `[queue.retry]` / `queue.retry`
  remains, and `queue.task_timeout_secs` is intentional
- dirty worktree state was shown
- pushed image digest was captured
- `register-image` dry-run was executed after `release-image`
- `kubectl set image` and `kubectl rollout status` both succeeded
- new deployment revision was captured
- at least one new pod is `Ready`
- pod `imageID` matches the pushed digest
- config/key identity check passed after rollout
- redacted server config snapshot was captured after rollout
- server config diff was checked and matched the intended change set, or had no unexpected diff for
  image-only rollout
- `/ready` returned `200`
- `/metrics` endpoint was reachable

Use this report shape:

```text
Rollout completed.

Previous revision/image: <old-revision> <old-image>
New revision/image: <new-revision> <new-digest>
Tag: <tag>
Register: checked only | applied
Config/key check: ok
Server config diff: none | expected changes only
Ready: ok
Metrics: reachable
Worktree: clean | dirty
```

If the worktree was dirty, append:

```text
The image was built from the current worktree, not a single clean commit.
```

## Register Required

When dry-run shows pending registrations:

- stop the automatic rollout flow
- keep the built image tag and digest in the report
- report the exact backend/object list that needs registration
- say whether `PRIVATE_KEY` is available from repo-root `.env` or the current environment
- do not apply automatically unless the user explicitly asks for it

Use this report shape:

```text
Rollout paused after image build and before rollout.

Built image: <tag> <digest>

Register check found pending registrations for:
- <item-1>
- <item-2>

Apply requested: no
PRIVATE_KEY source: .env | env | unavailable
Next step required: explicit confirmation before running register-image --apply
```

## Rollout Failed

If `kubectl rollout status` fails, collect all of these before responding:

- `kubectl -n tolba-raiko2-host get pods -l app=raiko2 -o wide`
- `kubectl -n tolba-raiko2-host describe deployment raiko2`
- `kubectl -n tolba-raiko2-host describe pod <new-pod>`
- `kubectl -n tolba-raiko2-host logs deploy/raiko2 --since=5m`

Use this report shape:

```text
Rollout failed.

Tag: <tag>
Digest: <digest>
Observed revision: <revision-or-unknown>
Failure point: kubectl rollout status
Primary signal: <short error>
Diagnostics collected:
- pod list
- deployment describe
- pod describe
- recent deployment logs
```

## Smoke Failed

If rollout succeeded but smoke failed:

- report whether `/ready` or `/metrics` failed
- include the response body or curl failure
- include current pod readiness state
- include recent deployment logs if readiness failed

Use this report shape:

```text
Rollout completed but smoke failed.

Tag: <tag>
Digest: <digest>
Ready: ok | failed
Metrics: ok | failed
Failure point: /ready | /metrics
Primary signal: <short error or response excerpt>
```

## Metrics Reachable But Custom Families Missing

This is **not** a rollout failure by itself.

Use this note:

```text
/metrics is reachable. Custom raiko2 metric families are not present yet, which can happen when
the fresh pod has not observed new proof traffic since startup.
```

## Config Or Key Check Failed

If `.codex/skills/raiko2-rollout/scripts/verify-config-keys.sh` fails, stop before building,
registering, or rolling out. Do not patch secrets unless the user explicitly asks for a live fix.

The report must include:

- which source failed
- the derived address mismatch if one was printed
- whether the Boundless signer reused the host `NETWORK_PRIVATE_KEY`
- the next required action

Use this report shape:

```text
Rollout blocked by config/key check.

Failure point: verify-config-keys.sh
Primary signal: <short error>
Boundless signer source: <source>
Expected signer source: <source>
Network key source: <source-or-unavailable>
Next step required: fix the config secret or signer source before rollout
```

## Unexpected Server Config Diff

If a rollout is intended to be image-only but the redacted server config diff shows changed runtime
configuration, stop before continuing unless the user explicitly confirms the diff.

The report must include:

- snapshot paths used for the comparison
- the redacted diff summary, without raw secret values
- whether the diff came from deployment env, resources, replicas, or `config.toml`
- the next required action

Use this report shape:

```text
Rollout blocked by unexpected server config diff.

Failure point: server config diff gate
Before snapshot: <path>
After snapshot: <path>
Primary signal: <short summary>
Next step required: confirm the config diff is intended or restore the live config before rollout
```
