# Block-local transaction payload pricing

The user approved counting node-visible transaction payload work during block execution,
using the existing tariffs, while preserving complete proposal pricing. This addresses the
fixed 30-block fixture where adding about 30 MiB raised actual SP1 PGU by 4,508,435,290
without changing the 97,830,164 node subtotal.

## Design

Count canonical signed EIP-2718 encoded bytes and nonempty calldata transactions in the
block executor, where the original signed transaction exists. Include Anchor and included
REVERT/HALT receipts. Commit payload selectors atomically with inclusion; rejected candidates
must not change the committed node prefix. Preserve separate attempted/prover observations.
Do not infer signed-envelope length from TxEnv calldata or silently default an unavailable
selector to zero.

Use one authoritative payload evaluator and the existing coefficients: 256 PGU per encoded
byte, 16,384 PGU per nonempty calldata transaction, and the existing true-unattempted tariff
for that separate prover-only selector. The node-known subtotal adds its committed payload
term. Raiko validates the committed payload against the canonical committed transaction list;
proposal completion charges only additional input payload not already owned by the node.
Startup, blob/source topology, parent/block MPT and exact resource reconciliation keep their
existing ownership. No clamp-to-zero may hide an inconsistent selector or accounting mismatch.

Existing Unzen default pricing, limits, difficulty validation and feature modes are unchanged.
This change enriches shadow/model accounting; it does not activate new consensus pricing.
Keep ADDMOD/MCOPY coefficient changes separate. Preserve all published Atlas baselines and
frozen SP1 observations. No guest rebuild or new SP1 measurement is needed to re-quote them.

## Alternatives and decision

The selected approach adds a deterministic transaction-local term to the block report and
reconciles it during proposal completion. Increasing opcode tariffs would miss STOP payloads;
allocating startup or the entire proposal quote backward to blocks would depend on future
batch composition and double count unrelated work. Neither alternative addresses ownership.

## Verification

Test per-transaction commit/discard/reset, included failed receipts, Anchor, successive calls,
selector bounds/overflow and fixed-point rounding. Check node mode equivalence and Unzen
invariance. Test proposal-only extra candidates and the absence of double counting. Re-quote
all 271 archived plans that have valid inputs, retaining failures, and compare against their
fixed actual SP1 observations; analyze historical quotes separately for normal-tx impact.
For the matched 30-block fixtures, expected node totals with current coefficients are
100,763,924 (empty) and 8,154,359,316 (1 MiB per block), with complete totals unchanged at
274,913,627 and 8,328,509,019. These are predictions to verify, not measured new results.
