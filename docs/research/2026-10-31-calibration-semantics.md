# Calibration evidence and semantic validity

This audit separates numerical reproducibility from validation of the executed
workload. It follows the BLOCKHASH density result described in
[the experiment record](2026-10-31-blockhash-density.md). The current candidate
remains model `bbce9be0cd9792bfa9c46369203f6441c3fd288787ea357e311eae8ebd48aac4`.
This followup adds provenance corrections and regressions; it changes no price.

## Recovered original BLOCKHASH evidence

The original evidence was retained in Git, rather than lost:

- [Calibration plan](https://github.com/taikoxyz/raiko2/blob/d8ae27427003e572c231462dd48920eccb7b0bc3/docs/plans/2026-10-02-zkgas-anchor-operation-correction-implementation-plan.md).
- [Ninety measurement records and fit](https://github.com/taikoxyz/raiko2/blob/d8ae27427003e572c231462dd48920eccb7b0bc3/experiments/opcode-gas/calibrations/sp1-blockhash-v2/6af515a6377aaf0c2906b151/result.json).
- [Frozen fixture generator](https://github.com/taikoxyz/raiko2/blob/21ac9f900d880bb1167b3ca5d2b95be9afe72cd4/bin/guest-launcher/src/controlled_workload.rs#L2241).
- [Frozen fitting implementation](https://github.com/taikoxyz/raiko2/blob/21ac9f900d880bb1167b3ca5d2b95be9afe72cd4/experiments/opcode-gas/blockhash_campaign.py).

The result file's SHA256 is
`180ba27117966710ede6d4cf8f56cfe1beb729e2be72552ad6879ce35a3ed9b8`;
its content-addressed artifact is
`5fbdf5d26918e57214d70997795b2b41fad3219b8821730a08b0430f890c1d0c`.
All 90 per-row evidence hashes and all four bound source-file hashes match.
Both the source revision and the data commit remain retrievable from GitHub.

The panel declares three semantic classes, four fitting counts (0, 128, 256,
512), one checkpoint count (1024), target/control lanes and three repeats:
`3 * 5 * 2 * 3 = 90` records. The affine fit uses paired target-minus-control
observations. It adds the inherited ISZERO price to the fitted slope and
selects the largest class price:

```text
ISZERO = 38.631523870602717738...
selected paired slope = -6.513839285714285714...
BLOCKHASH = ISZERO + selected slope = 32.117684584888432024...
integer export = ceil(BLOCKHASH * 10^18) = 32117684584888432025
```

Independent rational replay reproduces this integer exactly. That confirms
what the old calculation did; it does not validate the semantic class labels.

### Reproduce the old numerical result

With the pinned data commit available locally, extract the retained data file:

```sh
git show d8ae27427003e572c231462dd48920eccb7b0bc3:experiments/opcode-gas/calibrations/sp1-blockhash-v2/6af515a6377aaf0c2906b151/result.json > old-blockhash-result.json
~/.venv/bin/python - old-blockhash-result.json <<'PYTHON'
import hashlib, json, sys
from fractions import Fraction
x = json.load(open(sys.argv[1]))
canonical = lambda v: json.dumps(v, sort_keys=True, separators=(",", ":")).encode()
assert hashlib.sha256(canonical({k: v for k, v in x.items()
                               if k != "artifact_sha256"})).hexdigest() == x["artifact_sha256"]
assert len(x["execution_rows"]) == 90
registry = x["fit_registry"]
iszero = Fraction(registry["common_dispatch"]) + 3 * Fraction(
    registry["models"]["opcode:0x15"]["parameters"]["body_per_raw_gas"])
prices = []
for semantic in ("recent_ancestor_hit_1", "recent_ancestor_hit_256", "out_of_range_zero"):
    ys = {}
    for n in (0, 128, 256, 512):
        lanes = {lane: [r["prover_gas"] for r in x["execution_rows"]
                        if r["semantic_class"] == semantic and r["count"] == n
                        and r["lane"] == lane] for lane in ("target", "control")}
        assert all(len(values) == 3 and len(set(values)) == 1 for values in lanes.values())
        ys[n] = Fraction(lanes["target"][0]) - Fraction(lanes["control"][0])
    mean_x, mean_y = Fraction(896, 4), sum(ys.values()) / 4
    slope = sum((n - mean_x) * (y - mean_y) for n, y in ys.items()) / sum(
        (n - mean_x) ** 2 for n in ys)
    prices.append(iszero + slope)
scaled = max(prices) * 10**18
coefficient = -(-scaled.numerator // scaled.denominator)
assert coefficient == 32117684584888432025
print("Numerical replay only; inherited coefficient:", coefficient)
PYTHON
```

This reproduces arithmetic from retained observations. It does not execute the
old guest or qualify the semantic class labels. The new operand-order tests
are described in the [bench usage](../../bin/zkgas/bench/README.md#blockhash-density).

## Confirmed generator defect and missing gate

The frozen generator emits `NUMBER; PUSH3 offset; SUB; BLOCKHASH; POP`.
REVM 41 implements SUB as the popped top operand minus the next operand.
Consequently this program requests `(offset - NUMBER) mod 2^256`, while the
calibration intended `NUMBER - offset`.

At the fixture's block number 1,166,000, offsets 1, 256 and 0 all produce huge
future heights. Following the bound generator's semantics, BLOCKHASH takes
the early zero-return branch, without invoking the ancestor lookup. In
particular, the two declared ancestor-hit classes do not exercise their
intended branch. The target result is discarded by POP.

The old Rust validator independently computes the intended `B - offset` and
checks that the witness contains the corresponding ancestor window. It does
not inspect the bytecode's actual requested operand or result. Opcode-count,
repeat, residual and checkpoint checks can therefore pass for the wrong
branch. The checkpoint reuses the same generator, so increasing its count
does not provide an independent semantic oracle.

The defect is confirmed in the exact source blobs named by the artifact.
This audit has not recovered and replayed every old GuestInput or established
a complete historical binary-to-source binding. The preserved normalized
records cannot alone prove that the declared ancestor-hit paths were sampled.
The old result must not be used as evidence for their normal cost.

## Current regression and pricing implications

The maintained density producer already emits `PUSH3 offset; NUMBER; SUB`,
checks the expected zero/nonzero result and is checked by an independent
operand interpreter. The new regression explicitly mutates it to the old
ordering:

- Nearest/farthest valid targets must fail the semantic guard under the old
  ordering; the real native constructor rejects them without publishing inputs.
- Current/expired targets may still return zero and finish successfully under
  the old ordering. Their complete opcode counts can also match. The actual
  operand oracle must identify the wrong height; result-only checks are
  insufficient for these classes.

The original 12 density and six prospective requests retain byte-identical
contents. The 700-PGU successor uses their independently qualified operands,
new SP1 observations, frozen 25% increment policy and actual native requotes.
Its evidence is separate from the invalid ancestor-hit interpretation of
`5fbdf5...`; the new result does not depend on treating that interpretation
as valid.

The runtime artifact retains the old route authority hash as parent history.
The current coefficient's derivation is identified by
`successor.blockhash_evidence_path`, its evidence hash and frozen policy.
A reader that inspects only the inherited per-route authority hash will miss
that replacement. This documentation followup does not reseal the runtime
artifact or alter its executable coefficient table.

## Wider audit scope

The current export contains 174 route records. A route's presence, a source
hash or an accepted fit status does not establish operand or branch coverage.
This first followup checks the named legacy context, opcode, campaign and
isolated-operation generator entrypoints, including the linked arithmetic
and copy/return helpers. It is not a complete semantic certification of the
174 routes or their feature-error and precompile domains.

For each subsequently audited family, retain the intended operands and
branch, independently decoded operands, expected result or terminal condition,
actual native outcome, exact execution counts, source/data/program identities,
and the equation or deliberate conservative policy that produced its price.
Symmetric/zero-only samples are valid minimal observations but cannot establish
noncommutative ordering or value-sensitive worst-case coverage. Distinguish
an intentionally priced failure control from a normal-path sample that
accidentally takes a failure or early-return path.

Additional family findings and remaining limits are appended after independent
verification; no unverified suspicion is used to change a tariff.

### First scoped followup results

Independent reconstruction at legacy revision
`aa5990d9c4e1fc3c5b6312d6aaaf3e07a2811175` produced these bounded conclusions:

| Generator or property | Result | Meaning for pricing |
| --- | --- | --- |
| Historical BLOCKHASH height setup | Confirmed reversed SUB operands | Old ancestor-hit semantic evidence is invalid; current replacement has separate evidence. |
| Four named Rust loop countdowns | Correct `counter - 1` | No countdown-order defect found in the inspected loops. |
| Reviewed copy, memory, CALL-family, RETURN and REVERT argument stacks | Correct for inspected paths | No argument-order defect found; this does not certify all value/size domains. |
| Formal fixed EXP slots | Reset operands for each event | They do not inherit the chained-operand drift described below. |
| Formal dynamic EXP relation setup | Exponent below base, matching the opcode's pop order | The reviewed setup covers its declared exponent-byte inputs. |
| Legacy variable-footprint `build_stack_exp_bytecode` | Operands change on each repetition | Its raw gas can be 60, 60, 60, 160, 10, ...; a constant-60 interpretation is unsafe. No current tariff impact has been established. |
| Task6 isolated arithmetic operations | Zero/symmetric operands | Valid minimal execution observations, insufficient to validate ordering or value-sensitive worst cases. |
| General preexpanded-memory prefix | Allocates one extra word beyond the rounded requested length | Both lanes share this prefix. Exact initial-memory or boundary claims need separate review; this is not a demonstrated failed fit. |

The EXP observation concerns a legacy variable-footprint helper. The current
core derivation separately declares exponent-byte lengths 0/1/2/4 and
8/16/32, with 24 held out; the formal dynamic generator sets those operands
explicitly. That source information does not complete an execution-record
and authority-chain audit. No EXP price is changed on the strength of this
helper observation.

Source inspection and stack reconstruction do not measure SP1 PGU. The native
operand-order regression adds direct execution evidence only for its declared
BLOCKHASH cases. The current 174-route table is not marked globally audited.
