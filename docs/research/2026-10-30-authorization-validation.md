# Authorization-list validation workload

The pinned Unzen fork accepts EIP-7702 transactions. Canonical signed-envelope
conversion recovers every authorization before opcode collection begins; repeated
entries are recovered separately, even when later nonce checks skip them. Current
parent pricing model537a has no authorization-count term. Signed payload and deduplicated
account/code resources still contribute, so actual sampling must establish whether
their charges cover the additional work.

## Offline constructor extension

Add optional `program.authorizations` with `pattern` (sequential, repeated,
distinct, or the separate compact holdout) and `count` in 1..1024. Omission preserves
the existing EIP-1559 path,
canonical bytes and report shape except executable identity. Presence selects a
canonical EIP-7702 transaction with the same outer sender, target, calldata,
value, gas and fee settings. Use public deterministic offline fixture keys for
separate authorities and the program account as the fixed delegation target.
Never read a wallet or credential source.

For block index b and count K: sequential authorizations use one authority with
nonces b*K+i; repeated authorizations clone one signed entry with nonce b;
distinct authorizations use K authorities, each with nonce b. Native validation
must confirm type/list/signatures, successful inclusion, and actual final authority
nonces/delegation code hashes. Expected post-block nonces are respectively
(b+1)*K, b+1 and b+1 per authority. Keep minimal honest witness construction and
existing program/storage behavior. Native report additions are optional and
explicitly distinguish signed entries from applied delegations.

This extends only the feature-gated fixture constructor and CLI schema; canonical
GuestInput and production execution remain unchanged. Reuse the existing b76baf2e
program, model 537a965e and guarded sampler. Verify the guest source fingerprint
and canonical GuestInput format remain unchanged rather than rebuilding a guest
for this host-only capability.

## Bounded cohort and verification

Use fixed empty calldata and minimal RETURN bytecode. Measure one shared
authorization baseline, sequential/repeated counts 4/16/64/256 and distinct
counts 4/16: eleven EIP-7702 inputs. Add one explicitly labelled EIP-1559 reference;
do not call an empty authorization list a valid EIP-7702 control. Reuse the
count-one sample for comparisons instead of rerunning identical envelopes.

Test constructor bounds/encoding/signatures, final nonce/code gates and at least
two-block nonce progression. Check omitted-field canonical byte compatibility
against a frozen old native executable. Require independent review and native/quote
verification before actual SP1 sampling. Check real canonical type-4 inputs on
the existing program before expanding counts. Preserve failures and avoid claiming
an admission or workload deficit until actual PGU/quotes are joined.

## Confirmed parent-model deficit

All twelve inputs completed actual SP1 execution with the same identified b76baf2e
program and SDK 6.3.0. Independent verification recovered all 701 signed entries
and checked canonical inputs, execution/public-output joins, literal rankings and
source identities. Evidence integrity passed, while pricing adequacy failed:
nine parent complete quotes are below their actual PGU. These statuses are separate.

| Input | Signed entries | Actual proposal PGU | Parent node-known PGU | Parent complete PGU |
| --- | ---: | ---: | ---: | ---: |
| EIP-1559 reference | 0 | 160,984,193 | 6,116,868 | 161,058,698 |
| Shared type-4 baseline | 1 | 161,121,234 | 6,156,879 | 161,113,477 |
| Sequential | 256 | 186,558,632 | 12,390,735 | 167,347,333 |
| Repeated, only first applies | 256 | 184,275,517 | 12,358,991 | 167,315,589 |
| Distinct authorities | 16 | 163,004,110 | 6,704,621 | 165,113,579 |

The sequential 256-entry complete-price deficit is 19,211,299 PGU. This is not
only a startup allocation artifact: matched increments add roughly 90,000–127,000
actual PGU per authorization while node increments charge roughly 24,000–37,000.
The operation maps/prices remain equal across all twelve inputs. Authorization
validation happens before opcode collection, so a complete opcode table does not
cover it. Repeated stale entries still perform recovery and retain the deficit.

The literal ranking remains actual whole-proposal PGU / node-known zkGas; matched
increments explain the missed work without replacing that objective. Actual PGU
includes proposal startup, which the node subtotal does not own. Raw Unzen scores
and EVM gas are not PGU and are not substituted in either ratio.

Peak process-tree PSS was 1,359,886,336 bytes. All 48 exact owned resources were
confirmed absent in the host namespace. No proving or GPU throughput was measured.

## Additive pricing correction and separate holdout

[Reth #260](https://github.com/taikoxyz/alethia-reth/pull/260) adds 200,000 normalized
PGU per raw authorization entry to the node-known count. Count every entry of an
included type-4 transaction, including repeated, stale, wrong-chain and invalid
recovery entries and included user REVERT. Applied or unique-authority counts are
insufficient. This is additive to payload/resources, retained once in full block
completion, with no second discount or proposal-once authorization charge.

The term is an initial conservative empirical policy, not a universal upper bound.
It leaves all old terms and the global discount unchanged; zero authorization
entries preserve the old numeric result. Recovery for outer transactions filtered
before an EVM scope begins remains deferred under the honest/minimal-input scope.
That surface is distinct from ignored inner entries of an included transaction.

A separate five-input compact-signature holdout was selected before successor
validation and was not used to set the tariff. It uses canonical `(r=1,s=1,parity=0)`
authorization signatures, giving 30-byte entries instead of ordinary long signatures.
K=1/16/128/512 one-block proposals test count independently of byte pricing; K=16
across three blocks tests counting after every authorization is stale. Native
verification checks the recovered authority, raw typed RLP, actual nonce/code and
minimal witness progression. The three-block case has 48 signed entries but only
one applied delegation. The original twelve-input cohort remains immutable.

All five holdouts completed actual SP1 execution and independently verified joins:

| Compact entries per block | Blocks | Actual proposal PGU | Parent node-known PGU | Parent complete PGU |
| --- | ---: | ---: | ---: | ---: |
| 1 | 1 | 161,103,430 | 6,139,983 | 161,096,581 |
| 16 | 1 | 162,151,252 | 6,255,951 | 161,212,549 |
| 128 | 1 | 170,017,336 | 7,116,111 | 162,072,709 |
| 512 | 1 | 197,006,489 | 10,065,231 | 165,021,829 |
| 16 | 3 | 169,981,891 | 13,369,286 | 169,317,317 |

All five parent full quotes underestimate actual PGU. The largest deficit is
31,984,660 PGU at K=512; its user transaction consumes 12,821,006 EVM gas.
The three-block input confirms all 48 entries remain work despite only one
applied delegation. Extra blocks also change Anchor/state/proposal overhead,
so its difference from the one-block input is a separate multi-block diagnostic,
not an isolated authorization-per-entry estimate.

Independent audit checked 90 raw artifact identities, 80 source bindings,
normalization, exact literal ranking and matched increments. Peak process-tree
PSS was 1,440,530,432 bytes, and all 20 exact owned resources were absent in the
host namespace after execution. These observations still describe the parent
model; successor adequacy requires actual native re-quotation, not only adding
the proposed tariff in a spreadsheet.

## Actual successor re-quotation

The host quote tool pinned to reviewed Reth commit `b7060a1` and model
`4c0306015efc1572db444d8b78cafc9431988bf5ca3d678e750d0292e9ca8a55`
replayed all 17 retained canonical inputs. This is actual native replay using the
new implementation against the saved SP1 measurements, with no SP1 rerun or guest
rebuild. The old 14 complete-quote deficits become zero in this finite cohort:

| Input | Raw entries | Actual proposal PGU | New node-known PGU | New complete PGU |
| --- | ---: | ---: | ---: | ---: |
| Sequential K=256, one block | 256 | 186,558,632 | 63,590,735 | 218,547,333 |
| Repeated K=256, one block | 256 | 184,275,517 | 63,558,991 | 218,515,589 |
| Compact K=512, one block | 512 | 197,006,489 | 112,465,231 | 267,421,829 |
| Compact K=16, three blocks | 48 | 169,981,891 | 22,969,286 | 178,917,317 |

Every node/full increase equals exactly 200,000 PGU times its own raw entry count.
All old components, canonical input hashes, public outputs and raw legacy scores
remain unchanged. The EIP-1559 reference has zero count and identical numeric prices.
Proposal-once authorization is zero, so the three-block case charges 48 entries
once rather than charging one applied delegation or duplicating the contribution.

The native quote tool exports committed counts under `node` and attempted counts
under `prove`, retaining their existing separate ownership. Both the count and
scaled term are required, validated against the shared model and included in sum
closure. This remains internal Shadow development with no admission activation.
Updating the reviewed dependency pins changes declared guest-source fingerprints;
existing production guest artifacts are not qualified by a host quote build.

Independent verification replayed six selected inputs with the frozen new CLI,
checked all 17 reports against raw counts and old terms, and bound all 476 recorded
source/config files to the committed consumer revision `86584ed9`. Independent
trace and comparator tests passed 24/24 and 27/27, including missing fields,
forged count/price pairs and proposal double-charge rejection.

## Historical compatibility check

Actual new-model native replay also covered the retained 50 Mainnet proposals /
9,600 blocks. These retain their original external SP1 v6.1.0 / ELF416f actuals;
they are historical shape/compatibility evidence, separate from the b76b/SDK6.3
controlled workload cohort.

| Measure | Parent537a | Authorization successor |
| --- | ---: | ---: |
| Saved actual PGU, unchanged | 69,980,978,094 | 69,980,978,094 |
| Complete quote PGU sum | 145,919,113,551 | 145,920,113,551 |
| Workload-weighted complete quote / actual | 2.085125380 | 2.085139670 |
| Complete-quote deficits | 0 | 0 |

Exactly five included authorization entries occur in proposal 45304, block
13,294,968, adding exactly 1,000,000 PGU. That proposal's node quote changes from
787,366,572 to 788,366,572 and its full quote from 2,133,685,834 to 2,134,685,834.
All other proposal prices and all old terms, input/output identities and raw
legacy scores remain unchanged. This does not materially change the historical
roughly 2.09x overquote conclusion. It is not a current-program PGU certification.

## Validation and remaining scope

The consumer passed workspace Clippy with warnings denied, 24 trace tests,
99 default tool tests, 24 fixture-feature tests with frozen-native compatibility
controls, the 156-test primitives/protocol CI lane (one existing ignored doctest),
46 stateless MPT tests, formatting, and locked no-build metadata for all three
standalone guest manifests. The upstream correction separately passed 149 EVM
tests, eight signed-block tests, four independent API tests and 19 Python tests.
Source review and behavioral verification were performed independently of authors.

All 17 controlled actuals remain on the same experimental guest. New pricing was
validated by native replay, not a new guest build or proof. Production artifact
alignment, arbitrary input upper bounds, filtered outer-input validation work and
the deferred EVM-gas-only saturation question remain outside these finite results.

The immutable controlled data is in [Atlas #15](https://github.com/taikoxyz/raiko2-atlas/pull/15).
It preserves both cohorts, original SP1 measurements and parent deficits, actual
successor envelopes, source bindings and independent audits. Independent archive
review and the unchanged integrity workflow against the published Release verified
383 data files. Tools and implementation stay in Raiko/Reth; no execution policy,
source code, ELF, host binary or pricing table is copied into Atlas.
