# ECMUL scalar mixtures and route aggregation

Model923 covers the measured mixed-scalar workload: at 416,425 charged block EVM
gas, actual workload is 4,700,584,886 SP1 PGU against literal node 5,725,410,834
and complete-proposal 5,880,353,177 PGU. Node is 1.218021x actual. The zero-call
control retains a 153,017,715-PGU node deficit; its complete-proposal quote covers
actual. Neither measured input crosses the >2B-PGU deficit at <=30M block-gas
priority screen. No price or model change is selected by this finite experiment.

## Hypothesis and actual pricing branches

The current PC07 route takes the maximum of three accumulated terms for a block.
For each successful 6,000-native-gas ECMUL call at G=(1,2), in PGU units:

```text
A_i = 20,000,000
I_i = 179,406,000
S_i = 572,936 * W_i
W_i = bit_length(scalar mod group_order) + popcount(scalar mod group_order)
route price = max(sum A_i, sum I_i, sum S_i)
```

The API carries these terms scaled by 10^18. These A/I/S values are derived from
source and authenticated inputs, not exported native per-event accumulators.
Scalar zero has W=0 and a successful 64-byte all-zero result. The legal scalar
H=(3<<252)-1 has W=507. A 31-call sequence mixes 12 zero and 19 H calls. Its
route aggregate chooses I=5,561,586,000; the structural sum is 5,519,092,488.
Summing event-wise maxima instead would give 7,671,964,488, a difference of
2,110,378,488 PGU. That is a difference between price expressions, not actual
underpricing. The experiment measures whether existing conservative margins cover
this attainable branch mixture.

Six native inputs independently establish the crossover: low31, high31 and the
mixed sequence, each at R0/R1. Within each R, runtime, opcode counts, dispatch,
initial world and exact signed Anchor/context agree. Subtracting the derived
PC07 aggregate from actual sealed operations leaves a common remainder. Mixed
R1 sealed operations equal low R1; high minus low is exactly 3,443,249,112 PGU.
This comparison is limited to operations. It does not assume payload, signatures,
trie work or complete totals can be canceled between arbitrary inputs.

## Finite measurements

| Recipe | R | Charged block gas | Actual SP1 PGU | Literal node PGU | Complete-proposal PGU |
| --- | ---: | ---: | ---: | ---: | ---: |
| low | 0 | 218,115 | unmeasured | 8,527,125 | 163,468,956 |
| low | 1 | 394,537 | unmeasured | 5,725,410,834 | 5,880,353,177 |
| high | 0 | 307,395 | unmeasured | 8,527,125 | 163,468,956 |
| high | 1 | 430,249 | unmeasured | 9,168,659,946 | 9,323,602,289 |
| mixed | 0 | 272,835 | 161,544,840 | 8,527,125 | 163,468,956 |
| mixed | 1 | 416,425 | 4,700,584,886 | 5,725,410,834 | 5,880,353,177 |

There are six distinct canonical/native/quote inputs and exactly two completed
actual executions, each once. The four unmeasured actuals remain unmeasured.
Mixed R0 includes the same 2,976-byte input and setup/guards but no PC07 calls.
Its literal node deficit remains visible; startup is not subtracted from the
comparison denominator. The two complete-proposal estimates cover actual.

The own-control whole-input increment is 4,539,040,046 actual PGU, versus node
5,716,883,709 and proposal 5,716,884,221. These differences are diagnostic; they do
not isolate one precompile call or replace the literal whole-input metrics. The
measured R1 node margin is 1,024,825,948 PGU. No conclusion about arbitrary scalar
mixtures, ordering, full-R occupancy or any 30M-gas packing maximum follows.

## Reusable tools and qualification

The [compiler-free mixture producer and literal byte VM](../../bin/zkgas/bench/README.md#compiler-free-ecmul-scalar-mixtures)
are retained at tools commit `639030060632bc982b8db8e680c400f5bea19a19`.
The old single-scalar runtime rejects the valid infinity result and cannot encode
this mixture; its contract and known scalar table are unchanged. The new data
entry is scalar plus expected X/Y. Every STATICCALL7 checks success, exactly
64 returned bytes and both output coordinates, including zero. Counter, memory
sentinel, fixed point, output clearing and empty terminal stack are checked.
Independent Jacobian/affine vectors cover reduction, infinity and dense scalars.

Gas is computed from actual zero/nonzero calldata bytes, with gross user gas:

```text
intrinsic + 287 + (6,256 * K + 55) * R, K=31
charged = max(gross, EIP-7623 calldata floor)
```

Actual native receipts match the independent byte execution. Signed envelopes
come from the sizing reference full R77/R76/R77 for low/high/mixed. No full-R
canonical input or actual was generated. Twelve signature recoveries, complete
four-account parent/five-account final roots, six minimal trie nodes, three code
bodies, exact route counts, receipts, successful PC07 behavior and all scaled
quote equations passed the independent real-input gate. The branch gate is
separate from actual PGU measurement.

All runs use `evm-gas-only`; historical Unzen caps cannot screen the samples out.
Raw Unzen scores remain a separate unit. Actual uses the same canonical input,
b76 ELF and frozen SDK6.3 streaming observer, without an observation plan.
Normalization is exactly `floor(ExecutionReport.gas * 10 / 191)`. Full execution
options, program/build identities and public output are checked. All eight owned
SHM objects are directly absent on the original host. Peak actual process-tree
PSS is 4,544,195,584 bytes and maximum guard runtime is 140.982s, within 6GiB/600s;
native/quote limits remain 2GiB/180s. No guest, ELF, SDK, helper or runner was built.

The quote8f6 binary retains its actual Raiko dfa18/dirty=true and clean upstream
cac269 provenance; the later tool commit is not a replacement build identity.
The finite final panel SHA256 is
`4ecbba6ef23bc47cb50d3e29c1a0f5b6d34071f0fbf79cc24b280b5fe5bdecbb`.
Independent final raw/metrics/resource verification passed with report SHA256
`bcf706484ecbcea2c4c8610bd6b9a77ff9736b5c52986bf4fa993e97388d58cd`.
Model identity remains
`923c8f916f6e8603677b61adb60d92d2cd4355e02030c8fafa6ec549d10525f8`.
Opened development cases are not IID holdouts, production-SDK parity certification,
a GPU proving deadline, ordinary-transaction admission certification or a global
workload bound. This finite branch mixture does not justify repricing by itself.
