# Runtime Namespace Reset Design

## Goal

Provide an explicit operator-controlled startup reset for one Raiko2 runtime
namespace. It removes all persisted runtime state and proof objects for the
configured `(runtime.environment, runtime.namespace)` before the service starts
recovering tasks or accepting requests.

## Configuration

Add `runtime.reset_namespace_on_start`, defaulting to `false`.

When set to `true`, every startup performs the reset. The process does not
clear the flag itself; an operator must set it back to `false` after a
successful reset. This keeps the action explicit across restarts and avoids an
implicit state transition in configuration.

## Scope And Ordering

The reset is a store-level operation over exactly the configured namespace:

- Memory: discard the in-memory runtime state, artifact manifests, immutable
  proof content, and invalidation markers.
- GCS: enumerate only `<prefix>/<environment>/<namespace>/`, then conditionally
  delete every object in that prefix by its observed generation. This includes
  runtime state, checkpoints, manifests, immutable proof content, pending
  publications, and invalidation markers.

`AppState::new` creates the store, executes the reset when configured, and only
then calls `RuntimeManager::initialize` and the normal recovery path. No worker
or HTTP listener exists before this point.

The deployment model already requires one non-overlapping live process per
namespace. The reset relies on that invariant; it is not a distributed
multi-writer protocol.

## Failure Behavior

The operation is intentionally all-or-stop, not best effort. A listing or
conditional deletion error aborts startup. GCS deletion is not globally atomic,
so a crash can leave a partially cleared prefix; with the flag still enabled,
the next startup repeats the reset before serving traffic. No recovery,
publication, or admission work runs after a failed reset.

The flag is a replacement/remote-identity cutover tool. It deliberately does
not reuse the request-scoped prune endpoint or the v4 artifact invalidation
endpoint, which preserve parts of the persistence domain by design.

## Tests

- Config defaults to disabled and accepts the explicit flag.
- A memory store reset removes runtime state and all proof-object forms.
- A fake GCS transport reset deletes every object under its exact scope while
  retaining sibling namespaces.
- A transport deletion failure is surfaced so startup cannot continue.
- App startup with reset enabled does not recover a pre-existing task; with it
  disabled, normal recovery behavior remains unchanged.
