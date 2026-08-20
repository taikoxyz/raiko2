# Canonical Preflight Bincode Suffix Design

## Goal

Give canonical preflight cache content a distinct object suffix so bucket lifecycle rules can retain
Boundless program ELF files and preflight cache entries for different periods.

## Design

Canonical preflight content objects change from `<hash>.bin` to `<hash>.preflight.bincode`.
The existing canonical preflight key, content hash, bincode payload, manifest format, and
invalidation layout remain unchanged. The change does not affect `GuestInput`, proof artifacts, or
public API behavior.

Existing `.bin` preflight objects are not migrated. A deployment using the new suffix sees a cache
miss, rebuilds the canonical preflight core, and publishes the same bytes under the new object name.
Existing bucket lifecycle policy removes the unreachable old objects.

## Lifecycle Separation

The distinct suffix permits an independent canonical preflight lifecycle rule without matching
Boundless RISC0 program `*.bin` objects. Bucket lifecycle changes are operational follow-up work and
are intentionally outside this code change.

## Verification

Update the GCS artifact-store tests to assert the new object name and reject the old suffix, then run
the focused runtime artifact-store test suite and formatting checks.
