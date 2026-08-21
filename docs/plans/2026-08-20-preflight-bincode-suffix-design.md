# Canonical Preflight Bincode Suffix Design

## Goal

Give canonical preflight cache content a typed object suffix that identifies its serialization
format and artifact class.

## Design

Canonical preflight content objects change from `<hash>.bin` to `<hash>.preflight.bincode`.
The existing canonical preflight key, content hash, bincode payload, manifest format, and
invalidation layout remain unchanged. The change does not affect `GuestInput`, proof artifacts, or
public API behavior.

Existing `.bin` preflight objects are not migrated. Without startup cleanup, a deployment using the
new suffix reports a cache read error for each legacy manifest, generation-protected deletion removes
that manifest, and the same request rebuilds and republishes the canonical preflight core under the
new name. Existing `.bin` content remains unreachable for its bucket lifecycle policy to remove.

Deploy once with `runtime.startup_cleanup = ["preflight"]` to remove legacy manifests before serving
traffic. This produces ordinary cache misses instead of a cutover-wide cache-error metric spike.
Remove the one-shot setting after the replacement starts successfully. Rolling back to a version
that uses `.bin` requires the same cleanup and rebuild in the opposite direction.

## Lifecycle Separation

GCS can already separate flat Boundless program keys from runtime-prefixed preflight objects with
combined prefix and suffix conditions, so lifecycle separation does not require this rename. The
typed suffix instead makes the object class explicit and permits a direct `*.preflight.bincode`
lifecycle condition.

The deployed preflight cache remains disabled until the `*.preflight.bincode` lifecycle rule is in
place. The operational sequence is: merge this code, install the seven-day lifecycle rule, deploy
with one-shot preflight cleanup, and only then enable shared preflight caching. Bucket lifecycle
changes are outside this code change.

## Verification

Update the GCS artifact-store tests to assert the new object name and reject the old suffix, then run
the focused runtime artifact-store test suite and formatting checks.
