# Transient prefix and map-size boundaries under model923

The unchanged model923 covers all four measured repeated-read probes at512 and1024
transient keys. Their node quotes are1.221–1.337x actual SP1 PGU. Two512-key zero controls
retain literal node deficits, maximum98,062,687 PGU; no measured deficit reaches the
>2B-PGU-within30M-gas priority screen. All nine measured complete-proposal quotes cover
actual. This finite validation round makes no pricing change or global workload claim.

The dataset contains18 distinct canonical native inputs and18 original quotes. Nine SP1
actuals completed once: eight qualified pressure inputs and one valid control whose
pressure hypothesis was not realized. Nine other native/quote inputs remain explicitly
unmeasured. No full-R canonical input or30M full-block packing experiment was run.

## Why these boundaries

The [previous transient-map correction](2026-11-05-transient-hash-pressure.md) prices
per-transaction, per-owner work with `C=ceil((N+7M)/8)` and `384*C*(A+N+J)` PGU.
N counts distinct materialized slots, M is the largest raw-slot low32 group, A counts
potential accesses and J potential store-journal work. The correction intentionally uses
a cheap concentration proxy rather than simulating hash-table placement or byte compares.

These controls concentrate actual hashes while varying the raw key prefix just below
the four-byte grouping boundary. The b2/b3 repeated-read cases test whether the smaller
C values still cover actual work. Doubling N checks that the512-key generic-operation
floor did not hide a larger-map deficit. Earlier controls and their findings remain
separate observations; they are not counted again as new actuals.

Under the observed b76 specialization, with `F(x,y)=low64(x*y) XOR high64(x*y)`:

```text
H = F(k2 XOR F(k0 XOR S, k1 XOR G), k3 XOR G)
```

Selecting k1=G and k2=k3=0 gives H0 for any own seed S and low limb k0. The recipe uses
query `G<<64`, candidates `query | (i<<(8*b))` for i1..N-1, then query last. b is0..4
at N512, and2/3 at N1024. This algebra is conditional on the actual shared G; it does
not qualify a newly signed input or dynamically trace all placements.

| N | b | Maximum low32 group M | C |
| ---: | ---: | ---: | ---: |
| 512 | 0/1/2 | 1 | 65 |
| 512 | 3 | 2 | 66 |
| 512 | 4 | 512 | 512 |
| 1024 | 2 | 1 | 129 |
| 1024 | 3 | 4 | 132 |

Each proposal has Anchor plus one signed user, empty primary persistent storage and no
transient MPT targets. It checks query0, writes N calldata slots to1, checks query1,
then repeats64 TLOAD/POP sites per R. Full transient state, stack, protected memory,
counters and transaction reset are independently checked. Same N/G/R uses identical
code across b; the different calldata is separately signed and authenticated.

Gas comes from actual calldata zero/nonzero counts. At N512, R0/R512 charged block gas
is498,055/3,870,095; the common envelope is14,997,665 and the calculated full R2179.
At N1024 those block values are815,435/4,085,623, envelope14,996,393 and calculated
full R2147. These full values are sizing metadata, not executed full samples.

## Completed measurements

All estimate/actual columns are PGU. The node column is the literal integer block quote,
without replacing it by a proposal quote, increment or startup-adjusted denominator.

| N | b | R | Actual SP1 PGU | Node PGU | Complete-proposal PGU |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 512 | 2 | 0 | 171,760,994 | 73,813,728 | 228,755,559 |
| 512 | 2 | 512 | 881,443,802 | 1,177,485,986 | 1,332,427,817 |
| 512 | 3 | 0 | 172,467,007 | 74,404,320 | 229,346,151 |
| 512 | 3 | 512 | 971,725,863 | 1,190,659,490 | 1,345,601,321 |
| 1024 | 2 | 0 | 195,321,158 | 215,520,590 | 370,462,421 |
| 1024 | 2 | 512 | 1,588,861,611 | 2,124,499,216 | 2,279,441,047 |
| 1024 | 3 | 0 | 198,147,958 | 219,061,838 | 374,003,669 |
| 1024 | 3 | 512 | 1,773,949,381 | 2,165,789,200 | 2,320,731,031 |

The four R512 node/actual ratios are1.335861x,1.225304x,1.337120x and1.220886x.
The512-key controls' positive node deficits are97,947,266 and98,062,687 PGU. They remain
visible despite complete-proposal coverage. All four1024-key qualified cases are also
node-covered. No admission-limit multiplier or normal-transaction availability guarantee
follows from these selected synthetic inputs.

The paired whole-input increments provide a separate scaling diagnostic:

| N | b | Actual R0-to-R512 increment | Node/proposal increment |
| ---: | ---: | ---: | ---: |
| 512 | 2 | 709,682,808 | 1,103,672,258 |
| 512 | 3 | 799,258,856 | 1,116,255,170 |
| 1024 | 2 | 1,393,540,453 | 1,908,978,626 |
| 1024 | 3 | 1,575,801,423 | 1,946,727,362 |

Each increment is covered. These are whole-input differences, not isolated memcmp
coefficients, per-candidate PGU attribution or replacements for the absolute node metric.

## The retained unrealized control

The first1024-key input reused reference G `0xd34e0eabce2aabb7` from the512-key panel.
Its canonical execution completed successfully at169,623,172 PGU against node215,520,590
and complete-proposal370,462,421 PGU. Its own observed shared G was instead
`0xd2b5ccfd88cdb855`, own S `0x285da586789f4441`, mask2047, query hash
`3705316461323002932`, bucket1076 and tag25. Recomputing under these actual parameters
gives1024 distinct hashes and only one query bucket/tag match; the observed successful
lookup has one candidate comparison. It is a valid non-pressure control, not an execution
failure or a passed pressure test. The other three reference-G actuals were not run.

Four distinct inputs were then constructed using the newly observed G. Every new actual
independently confirms its own G, seed, mask, query hash0/bucket0/tag0 and returned1.
The earlier control was neither overwritten nor rerun. The observations establish that
G differed between these input shapes; they do not establish the cause of that difference
or guarantee seed stability for other inputs, programs or backends.

At N512, b0/b1/b4 each have R0/R512 native inputs and quotes but no actual. Those six,
plus the three unrun reference-G1024 inputs, account for all nine unmeasured cases.
There are no invented zero actuals or extrapolated full-R observations.

## Runtime evidence and retained tools

All eight pressure actuals have qualified own-input inner-map and owner observations.
All512 or1024 catalog hashes are independently computed under their own observed
parameters and equal0. Only sampled query calls are dynamically hash-captured; those
computations are not a trace of every table placement. R512 observations are400M-instruction
prefixes with capped register samples, so their comparison counters are not whole-program
counts. Complete canonical GasVM totals remain independent of these optional diagnostics.

The reusable raw-key API, recipe and literal byte oracle are retained at Raiko commit
`60a0077cb970e173ee0e50cfb4e3f77085ec42b8`, with usage in
[the bench README](../../bin/zkgas/bench/README.md#raw-transient-keys-and-byte-prefix-controls).
One kernel/VM serves strict and raw inputs. All220 original complete row/request byte
pairs remain unchanged. Nine read-family tests, related store/catalog tests and an
independent22-scenario uncached execution check passed; two related opt-in native tests
remain skipped. Later new inputs completed real native execution and authentication
before their actuals; those skips are not treated as executed tests.

Independent checks cover every new signature, complete4-to-5-account transition, six
canonical account-trie nodes, three code bodies, receipts, minimal witness, gas/floor,
public output, model identity and `54+C*(A+N+J)` combined journal units. Proposal-once
journal work is0. All18 native/quote inputs and nine raw actuals were independently read
back. All36 newly owned shared-memory objects are directly confirmed absent on the
original host. Actual peak process-tree PSS is4,546,466,816 bytes and maximum guard
runtime57.354s, within the6GiB/600s limit. Native/quote guards remain2GiB/180s.

All actuals use the same retained b76 ELF, SDK6.3 GasVM and observer; actual PGU is
`floor(raw ExecutionReport.gas*10/191)`. Model923 has identity
`923c8f916f6e8603677b61adb60d92d2cd4355e02030c8fafa6ec549d10525f8`.
The quote executable remains8f6ed77 from actual dfa18/dirty=true source with clean upstream
cac269, distinct from later committed tool source60a0077. Full executable/program
identities and source context are recorded in the preceding correction report.

No pricing coefficient, model, guest or SDK changed for this round. These opened
development controls are not IID/holdout data, production SDK parity, a proof or a
universal30M-gas bound. Changed guest/backend programs require new measurements.

The [immutable Atlas fixture](https://github.com/taikoxyz/raiko2-atlas/releases/tag/fixture-unzen-transient-prefix-boundaries-v1)
retains all18 native/quote inputs, nine actuals, nine explicitly unmeasured cases and
the unrealized control. Its204 regular archive members include192 byte-identical raw
originals and ten explicitly identified portable projections. The compressed archive
is1,474,326 bytes with SHA256
`8c92a04063ef96db82923d4f129d44bf6d98f2d1e5e1a0fa75c4f34eb114bfdb`.
Real GitHub downloads passed the existing fixture-integrity assertions and the literal
README unpack/checksum instructions before publication; a fresh published download
also matched compressed and decoded checksums. These are retention checks, not new
measurements or a claim about remote CI completion.
