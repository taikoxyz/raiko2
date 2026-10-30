# Minimal DELEGATECALL Repetition Plan

Use the existing autonomous implementation, independent review and independent
native verification workflow. Specification:
[finite experiment](../../research/2026-10-30-delegatecall-repetition.md).

1. Add one reusable Python producer and focused tests under `bin/zkgas/bench`,
   with bench README usage. TDD; reject invalid parameters and accidental
   short execution. Keep ordinary fixture interfaces unchanged.
2. Independently decode all 16 parent/child programs and qualify through the
   frozen native and quote executables. Exercise status/count/gas negative
   controls. Review the complete changed source before committing producer.
3. Freeze the committed producer/catalog and run each qualified canonical
   GuestInput once through frozen SP1. Apply existing memory/disk/time guards,
   verify public-output joins and owned-resource cleanup.
4. Audit literal ranking, full closure and within-terminal repetition deltas.
   Retain reusable code and measured results; publish immutable data in Atlas
   only after independent verification. No merge, deployment or guest build.

Progress: producer committed as `40a6e6ae` after independent source review,
16 native/quote qualifications, four state probes and five rejected negative
probes. All 16 actual SP1 observations completed; full quotes and all 20 matched
increments are covered under unchanged prices. Final independent actual audit
passed, including 64 owned objects absent in the host namespace and exact raw
identity/output joins. Immutable data are retained in Atlas PR19 and Release
`fixture-unzen-delegatecall-repetition-v1`; independent review verified all 335
archive members and 16 metadata files. An actual remote download passed the
existing integrity workflow before PR creation. Tools remain in Raiko; no
build, tariff change, merge or deployment was performed.
