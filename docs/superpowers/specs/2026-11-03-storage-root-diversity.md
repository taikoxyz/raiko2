# Storage-root diversity at equal block gas

Continue the approved block-aware zkGas search after homogeneous storage fanout. Keep actual same-proposal SP1 PGU, literal node per-block ceilings/sum and full proposal estimates separate. Positive actual-minus-node >2B PGU under <=30M charged block gas is high priority; lesser gaps remain visible and may be deferred. Synthetic construction does not waive ordinary/historical usability.

No host, GuestInput, guest, SDK, ELF, model, price, dependency or production-pin change. Reuse accepted native ad0d78dd, frozen c893 quote/SDK6.3/retained ELF and existing storage_fanout/storage_mutation runtime/gas helpers. Preserve old producers and all existing data byte-exact. No guest build, fit, deployment, merge or new timer.

## Approaches and chosen finite scope

Changing a single untouched value guarantees a different storage root, but can leave most witness nodes shared. Therefore measure two explicit profiles: `background-values` changes all untouched tail values; `disjoint-keys` shifts non-phase raw slots into separate per-account domains. Existing homogeneous inputs are immutable same-program references, not newly measured actuals. This is a small tools-only extension of the user-approved next target; proceed under continuing authorization without another approval loop.

Create32 new inputs: profiles2 x accounts(8,64) x aggregate slots(8192,16384) x modes(update,delete) x levels(zero,full). Always64 top-level CALL users, existing round-robin recipient addresses and unchanged guarded runtime. Reuse generated fullN84/update or106/delete, all ordinary gas/refund/envelope rules and31M header; charged block gas including Anchor remains<=30M. No A1 variant is claimed by this new panel.

Let A be account count, K=S/A, V=64/A, F=fullN and account index i in0..A-1. Own-N0/full use exactly the same initialized storage per group:
- `background-values`: raw slots0..K-1, phase0=1; values at1..V*F are1; values at V*F+1..K-1 are i+2. Only values unreachable even by full body change. Phase/body keys and every accessed value remain identical to homogeneous reference.
- `disjoint-keys`: B_i=(i+1)*65536; raw phase slot0 contains B_i+1; raw slots B_i+1..B_i+K-1 contain1, all other slots absent. Body bytecode/gas and body values stay identical, while actual raw keys and phase control values differ. Per-account start=B_i+1+floor(j/A)*N and final phase=B_i+1+V*N. Do not call all accessed values unchanged in this profile.

Every row must have A distinct authenticated initial and A distinct final storage roots. Record unique witness hashes/bytes, shared per-address proof support and quote hydration/materialization/churn selectors. Different roots alone do not prove zero internal-node sharing. Compare whole inputs, never assign actual PGU to an operation or trie.

## Reusable interface and qualification

Create `cases/storage_root_diversity.py` with make_case(profile='background-values',accounts=8,state_size=8192,mode='update',level='full')->dict; prepare(output:Path,*,model_id=MODEL)->int; CLI --output-dir/--model-id. Canonical corpus is unzen-storage-root-diversity-20261103. IDs storage-root-diversity-{profile}-a{A}-s{S}-{mode}-n{N}; baseline own profile/A/S/mode n0; homogeneous_reference_sample_id uses the existing storage-fanout ID. Extend metadata explicitly for profile, per-account bases/initial/final phases and strict source provenance; no stale homogeneous phase/range claims or compatibility aliases. Keep old generators unchanged and reuse authoritative runtime/gas construction.

Public standard-library tests execute actual bytes over isolated rolling accounts, independently derive exact maps/ordered reads/writes/originals/warmth/refunds/gas/routes and raw-key schedules, check complete N0/full prestates and unchanged signed payload declarations. Test invalid typed inputs/no overwrite/model provenance, wrong-owner/range/phase/tail-value mutants, and native-success but wrong intended key/value semantics. Optional bounded native smoke is not the independent qualification oracle.

Before actuals, independent gates qualify all32 signed recipients/receipts, whole-account and per-account initial/final roots, canonical conservative mutation proofs, exact gas/envelope prefixes and same-input original real quotes. Freeze source, tools, all inputs/quotes and16 unique homogeneous reference identities before sequential6GiB/600s execute-only measurements. Original quote reuse must have retained provenance, not a fabricated second invocation. No implicit retry. Preserve all controls/failures, source declarations versus build attestation, and published-fixture versus local-recovery distinctions.
