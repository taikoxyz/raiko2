# Checklists And Result Templates

## Successful Rollout

Confirm all of these before reporting success:

- Kubernetes context matches `tolba-prod`
- current deployment revision and image were captured before rollout
- dirty worktree state was shown
- `register-image` dry-run was executed
- pushed image digest was captured
- `kubectl set image` and `kubectl rollout status` both succeeded
- new deployment revision was captured
- at least one new pod is `Ready`
- pod `imageID` matches the pushed digest
- `/ready` returned `200`
- `/metrics` endpoint was reachable

Use this report shape:

```text
Rollout completed.

Previous revision/image: <old-revision> <old-image>
New revision/image: <new-revision> <new-digest>
Tag: <tag>
Register: checked only | applied
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
- report the exact backend/object list that needs registration
- say whether `PRIVATE_KEY` is available
- do not apply automatically unless the user explicitly asks for it

Use this report shape:

```text
Rollout paused before image build.

Register check found pending registrations for:
- <item-1>
- <item-2>

Apply requested: no
PRIVATE_KEY present: yes | no
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
