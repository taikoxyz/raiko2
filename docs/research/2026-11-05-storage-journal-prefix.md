# Storage-journal comparison pricing

This record documents a conservative block-observable correction under development on the
maintained shadow branches. It does not activate a consensus rule or change Unzen difficulty.
Actuals below are finite whole-input SP1 observations, not an isolated `memcmp` benchmark or a
proof of a global PGU bound.

## Observation and layout

EVM `SLOAD` and `SSTORE` use an arbitrary raw `U256` storage slot to index REVM's journal storage
map. Keccak hashing for the MPT is a separate step. Reading an unset raw slot is legal and returns
zero; no cryptographic preimage is required to choose these slot values.

For sampled guest ELF `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034`,
the journal entry specialization preserves the input key pointer in `s2`, passes it unchanged to
`memcmp` with length32, and compares from offset0. Vacant-key loads at offsets0/8/16/24 matched
all4096 observed limbs across1024 keys in two saved runs. This rules out reversed limb order and
byte-swapped-word interpretations for those observations: the first four compared bytes are the
numeric raw slot's low32, least-significant byte first. This is specific to the observed ELF and
specialization, not a Rust ABI or future compiler guarantee. There is no full source-to-ELF build
attestation. The observer's warm-key trace is partial.

Native collection uses `key.as_limbs()[0] as u32`, not pointer reinterpretation, native struct
layout or the first characters of a big-endian hex string. Re-evaluate this projection when the
proving ELF or relevant comparison implementation changes.

## Simple work proxy

For each transaction and storage owner, retain:

- `N`: initial physically cached slots plus subsequent potential materializations. A captured
  absent slot increments this count; failed vacant attempts and clear/refill may count the same
  key again. It conservatively covers physical occupancy without a second full-key set.
- `M`: largest materialization-count group with the same raw-slot low32.
- `A`: potential ordinary-storage lookup captures, including repeated accesses.
- `J`: cold-potential insertions plus every potential `SSTORE` change, reserving rollback lookups.

Use the final high-water counts for that transaction and owner:

```text
C = ceil((N + 7*M) / 8)
work_units = sum_transaction,owner C * (A + N + J)
storage_journal_PGU = 384 * work_units
```

Counts are retained across frame reverts and storage clearing and reset between transactions.
Initial cached slots seed once; ordinary cached repeats add lookup/change work, without adding
another materialization. Checked arithmetic and missing metadata produce typed incomplete
reports, never a silently valid zero surcharge. A stack underflow before slot lookup is known to
perform no journal access and retains the existing opcode feature-error price.

The byte component follows from at most4 bytes for a candidate with a different low32 and at
most32 otherwise: `4*N + 28*M`. This only bounds a candidate scan comparing each candidate once.
The coefficient conservatively absorbs measured whole-input residual work; it is not a precise
bucket simulator and does not prove a bound for repeated probes, hashing or growth. Hash-disperse
keys with a shared low32 can be severely overquoted. Deliberately constructed overquotes are an
accepted tradeoff for this simple model; historical admission compatibility still needs evaluation.

## Calibration and retained counterexample

These controlled post-Unzen Taiko inputs share the same631-byte runtime and512 seed slots.
Long-prefix and short-prefix R512 cases retain the same calldata byte cost and4,894,506 charged
block gas. Hash concentration was checked using each run's own observed profile; a hypothesized
search seed was not taken as proof of realization. The fixed actual program is the ELF above,
local SDK6.3.0. Normalization is integer `ExecutionReport.gas * 10 // 191`.

| Input | N | M | C | Work units | Old node PGU | Actual PGU | New native node PGU |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Concentrated long prefix, zero repetitions | 512 | 512 | 512 | 786,944 | 20,832,507 | 195,194,698 | 323,039,739 |
| Concentrated long prefix, R512 | 512 | 512 | 512 | 17,564,160 | 159,815,076 | 3,378,306,245 | 6,904,473,252 |
| Concentrated short prefix, R512 | 512 | 1 | 65 | 2,229,825 | 159,815,076 | 831,917,069 | 1,016,088,612 |
| Hash-disperse long-prefix control, R512 | 512 | 512 | 512 | 17,564,160 | 159,815,076 | 207,144,495 | 6,904,473,252 |

The short-prefix run was newly measured once, without a retry: canonical GasVM completed
971,098,101 instructions in35.18 seconds, with sampled full-process-tree PSS4.127GB. The optional
register observer covered a400M-instruction prefix; its comparator count is not a full-run count.
Existing long-prefix actuals are retained observations, not new measurements in this round.

The earlier p256 candidate quotes730,650,276 for the short-prefix case and **underestimates by
101,266,793 PGU**. This negative result must remain visible. The selected coefficient is:

```text
ceil_to_64(1.25 * max_positive((whole_input_actual_PGU - old_node_PGU) / work_units)) = 384
```

The maximum residual ratio is `672101993/2229825`; the25% coefficient margin followed by64-unit
rounding selects384. Fresh authoritative native quotes under model
`5f93909ea1372eb6e10fa2b599c6087aafd4ec493ba28eb1cea4c4abb3708a11` are shown in the table.
The target-only work units shown there receive54 additional fixture-system units in every complete
block quote, adding20,736 PGU beyond the initial projections. Replays preserve input identity,
public output and every prior non-journal component. The proposal-complete quotes are478,112,642 /
7,059,546,155 /1,171,161,515 /7,059,546,155 PGU in table order. The node quote is22.1% above actual
for short-prefix R512; long-prefix R512 is about2.04x and the dispersed long-prefix control33.33x.
Startup and unrelated work were not causally separated from the fit. Model identity is distinct
from ELF identity; these finite replays do not establish a global bound.

## Historical native compatibility panel

The new consumer replayed50 distinct saved mainnet inputs, covering9,600 distinct blocks, against
its previous model quotes under the same `evm-gas-only` execution policy. Every public output and
prior non-journal block component was unchanged. Added node and prove work each total211,636,608
PGU across the panel. Maximum proposal-level increases are0.46446% for node and0.18734% for prove.
The maximum block-relative increase is0.52802% (block13,255,810:5,236,148 →5,263,796); the maximum
absolute block addition is224,640 PGU (block13,275,687:174,801,452 →175,026,092). These are native
repricing compatibility results, not new SP1 actuals or evidence that all historical transactions
fit a future chosen limit. The older panel's external program identity differs from the b76
calibration program, so no same-ELF PGU accuracy certification is claimed from this panel.

## Report composition and verification boundary

`storage_journal_work_units` and `storage_journal_scaled` are mandatory fields in both node and
prove pricing snapshots. Node uses committed outer scopes. Prove uses all attempted scopes,
including candidates later discarded, while its exact resources, MPT and payload remain committed.
The term is independently additive outside `operations_scaled`, is included exactly once in each
block total, and is zero in proposal-once pricing. Current-model comparison checks the tariff;
foreign-model diagnostics still check typed selector domains and attempted coverage.

The journal work domain is independently bounded at10^12 units. The ordinary10^9 selector bound
is unchanged. The generated successor preserves the parent routes, global discount, fixed,
payload/resource/MPT tariffs, and ordinary dispatch reserve. No guest or SDK rebuild is required
for these native observation and pricing tools, and none is represented as having been performed.

Remaining work includes broader normal-history compatibility, capacity transitions and mixed
rollback/read/write SP1 compositions. The7-block Ethereum L1 survey is a
normal-access-shape diagnostic, not an SP1 replay or admission certificate. The user-defined
high-priority threshold is a **measured or justified30M-gas full-block gap over2B PGU**; the old
single-sample `high_priority` flag does not establish this criterion. No block-level completion
percentage or universal bound follows from this calibration.
