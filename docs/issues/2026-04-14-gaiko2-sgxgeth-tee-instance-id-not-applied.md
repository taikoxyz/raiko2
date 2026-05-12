# Gaiko2 Sgxgeth Tee Instance Id Not Applied

## Summary

The SGX regression compose stack can start the external `gaiko2-sgxgeth` tee container, but live
prove requests still fail with:

```text
tee proving requires GAIKO2_INSTANCE_ID or a registered GAIKO2_FORK mapping
```

This happens even when `GAIKO2_INSTANCE_ID` is explicitly present in the container environment.

## Evidence

Regression compose env:

- `GAIKO2_INSTANCE_ID=3131899905`
- `GAIKO2_FORK=shasta`

Container environment inspection confirmed the variable is set:

```text
GAIKO2_PROVING_MODE=tee
GAIKO2_TEE_TYPE=ego
GAIKO2_FORK=shasta
GAIKO2_INSTANCE_ID=3131899905
```

But a direct prove call still returns:

```json
{
  "schema": "gaiko2-proof-v1",
  "status": "error",
  "error": {
    "code": "PROVER_ERROR",
    "message": "tee proving requires GAIKO2_INSTANCE_ID or a registered GAIKO2_FORK mapping"
  }
}
```

Relevant implementation references in `../gaiko2`:

- `internal/prover/config.go`
- `internal/prover/signer.go`
- `ego/enclave.json`

## Likely Scope

This looks like an external `gaiko2` tee-runtime issue rather than a `raiko2` routing problem.

`raiko2` successfully registers `proof_type=sgxgeth` work and routes it onto the `sgx/remote`
lane, but the external prover still rejects tee signing due to missing effective instance-id
configuration.

One likely direction is that the tee runtime only forwards a fixed allowlist of environment
variables into the enclave, and `GAIKO2_INSTANCE_ID` / `GAIKO2_FORK` are not actually visible to
the proving process even though they exist in the outer container.

## Impact

- the regression stack cannot complete the `sgxgeth` proving lane end to end
- `proof_type=sgxgeth` routing in `raiko2` can be exercised, but external tee proving remains
  blocked

## Next Investigation

- verify which environment variables are forwarded into the EGo enclave runtime
- decide whether `GAIKO2_INSTANCE_ID` should be passed directly into enclave config or derived from
  mounted registration metadata instead
- re-test the compose stack after the `gaiko2` tee image is updated
