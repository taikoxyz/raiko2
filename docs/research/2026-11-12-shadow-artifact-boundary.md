# Shadow branch artifact boundary

The review distinguished the complete PR diff from the most recent pin update.
Commit `1955fcc3` had changed fourteen files under `crates/guests/elf`, including
proposal/aggregation executables, SP1 verification keys, lab artifacts and provenance.
Later updates preserved those changed bytes; they did not preserve main's artifacts.

This integration restores the entire directory to the exact reviewed Raiko base
`82d9a7f2bf6305db51e67ce88d20c3385ec0bd91`. Executable, verification-key and provenance
files are restored together, without rebuilding or editing their contents. This
keeps the research PR from introducing another artifact or verification-key update.

Restoring bytes is not source/artifact conformance. The internal Reth revision and
guest manifests/locks are source-fingerprint inputs. Their normal freshness checks
remain authoritative; these artifacts are not claimed to have been built from the
maintained research head. A final production-program build and same-input workload
qualification remain activation gates after the source/pin is frozen.

The maintained workload experiments separately identify their research ELF and SDK.
Those measurements must not be relabeled as results for these restored production
artifacts. Collector/model/executable/program identities remain separate.

The base integration preserves main's removal of Boundless, its SP1 native-runner
cache repair, ordinary launcher and SDK-smoke behavior. Shadow's optional
`zkgas-quote` launcher path and its separate CI test remain present. No guest build,
pricing activation, deployment or GitHub merge is part of this integration.
