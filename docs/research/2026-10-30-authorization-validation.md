# Authorization-list validation workload

The pinned Unzen fork accepts EIP-7702 transactions. Canonical signed-envelope
conversion recovers every authorization before opcode collection begins; repeated
entries are recovered separately, even when later nonce checks skip them. Current
new pricing has no authorization-count term. Signed payload and deduplicated
account/code resources still contribute, so actual sampling must establish whether
their charges cover the additional work.

## Offline constructor extension

Add optional `program.authorizations` with `pattern` (sequential, repeated,
distinct) and `count` in 1..1024. Omission preserves the existing EIP-1559 path,
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
program, model 537a965e and guarded sampler. Verify guest inputs/source fingerprint
are unchanged rather than rebuilding a guest for this host-only capability.

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
