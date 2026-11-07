# Same-route composition: a bounded CALLDATACOPY follow-up

The model923 node price sums different operation routes and separately adds normal
dispatch, payload, resources and journal work. It does not take one global maximum
across those workloads. One narrower convention remains: within a route, it takes
the maximum of accumulated accepted, incremental and structural prices. That can
be smaller than summing the maximum selected for each event.

For normal successful CALLDATACOPY events in one included canonical Unzen block
with charged EVM gas at most30M, this convention alone loses at most225M PGU
relative to the sum of per-event maxima. Without refunds, the bound is180M.
It is below the current >2B priority screen and is a low-priority composition TODO.
No new SP1 panel or price change was selected on this premise alone.

This bounds a difference between two pricing expressions. It does **not** bound
actual SP1 PGU minus node PGU, establish that either event estimate is accurate,
or extend to all67 Maximum routes, multiple blocks or rejected candidate attempts.

## Exact arithmetic and implementation scope

The source is upstream `cac269da858031b1d9f9a22ab5141ea0fde2b5dc`, with model
`923c8f916f6e8603677b61adb60d92d2cd4355e02030c8fafa6ec549d10525f8`.
The [per-route aggregation](https://github.com/taikoxyz/alethia-reth/blob/cac269da858031b1d9f9a22ab5141ea0fde2b5dc/crates/evm/src/zk_gas/pricing/model.rs#L399)
selects a maximum within each route and then sums routes. Component-wise merging
is in the accumulator. The emitted runtime specification binds0x37 to
`Composition::Maximum`, no structural rule, and incremental coefficient
`6 * 10^18` on `InterpreterRawGas`. Coefficients are already normalized to PGU;
there is no further discount multiplier on this expression.

Let A_i be the nonnegative accepted price and I_i=6g_i the incremental price of
normal copy event i, in PGU. Structural price is0. Then:

```text
E = sum_i max(A_i, I_i)
Q = max(sum_i A_i, sum_i I_i)
D = E - Q

0 <= D <= min(sum_i A_i, sum_i I_i) <= 6 * sum_i g_i
```

The upper bound follows from `max(A_i,I_i) <= A_i+I_i`. Normal dispatch and the
other separately added components are the same under both expressions.

The normal copy input validator requires
`g_i = 3 + 3*copy_words_i + memory_evm_gas_delta_i`. These are real gas charges
for successful copies, so their sum is no more than gross gas spent by the same
included block execution. Nested execution does not create additional gas; a
reverted child retains gas spent. Feature-error rows bypass the normal A/I prices
and are separately additive, so an unexecuted oversized/OOG copy is not inserted
as an arbitrary normal-copy g_i in this argument.

Current canonical Unzen configuration maps to OSAKA with the mainnet refund
quotient5. EIP7702 refund enters before the final refund cap. The calldata floor
can raise charged gas and cannot weaken the bound. For this execution domain:

```text
charged >= gross - gross/5 >= 4*gross/5
charged <= 30M  =>  gross <= 37.5M
D <= 6*37.5M = 225M PGU
```

Custom gas parameters or refund rules require a new argument. This is not a bound
for an unlimited proposal, replay attempts rejected before inclusion, arbitrary
instruction routes, or workload effects absent from either event model.

## Priority and remaining work

Warm bulk-copy and expansion-heavy events can favor different0x37 branches; their
possible crossover was not silently relabeled as measured by the old mixed
arithmetic/storage or MCOPY panels. The bound only explains why this aggregation
convention by itself does not warrant a new four-actual matrix at present.
It leaves copy pricing accuracy, failure-path classification and other interactions
open. A later native branch-crossover check can remain a small completion test.

An independent source/arithmetic review checked15 source identities, the real
capture/error/refund paths and the inequality. Its report has SHA256
`04e490080a5edf4fddbf741bd6a4a07e1f0297c0fd0115201f3674aca66b7f1e`.
No native, quote, SP1, guest or SDK run was added by this analysis. Higher-priority
same-route candidates must establish their actual selectors and reachable input
relationships before using the gas budget to rank possible aggregation losses.
