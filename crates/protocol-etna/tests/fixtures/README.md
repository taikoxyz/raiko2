# Stock CometBFT fixture

`first-block.hex` and `first-commit.hex` are the unmodified `block_proto` and
`commit_proto` bytes of `own_commit_sig_at_unix_epoch` in
`etna-core/testdata/cometbft-v0.40/differential.json`. That file's Git blob is
`afaadf57757de7bc3a0cc7558610a16abc11efc8` at the pinned
[Taiko client revision](https://github.com/taikoxyz/taiko-mono/tree/f31e7ec55345ac7faf99756504535b0cc9929ec0/packages/taiko-client-rs/crates/etna-core).
The [upstream generator](https://github.com/taikoxyz/taiko-mono/blob/f31e7ec55345ac7faf99756504535b0cc9929ec0/packages/taiko-client-rs/crates/lander/testdata/cometbft-v0.40/generate.go)
uses the stock CometBFT v0.40.0 implementation to sign, verify and marshal
the block and certificate. `envelope.hex` is the hex encoding of the same
directory's `envelope.base64` payload. The expected block hash and parts
header come from the source's `block_rpc.block_id`.

The fixture domain is L1 1, L2 167001, Inbox `0x1111111111111111111111111111111111111111`,
generation 0. The deterministic test scalars are repetitions of bytes 1 and 2,
with powers 10 and 30. The Rust tests extend the stock bytes with a linked
two-height case using the same test keys. Those synthetic blocks exercise the
Raiko2 caller checks with a nonzero origin and different signer powers at each
height. They are separate from the stock Go oracle.

## Etna landing ABI vectors

`land-input.hex` and `land-statement.hex` contain one ABI word per line from the
[proposed Etna design at `93b59fe6`](https://github.com/taikoxyz/taiko-mono/blob/93b59fe629dc8987ecb03c7d59d1f66004de068f/packages/protocol/docs/etna/SPEC.md),
section 6.1 and rules ENC-A-29–ENC-A-35. The word layout was assembled from the
specification and checked against Foundry `cast abi-encode`. Foundry
`cast keccak` produced the expected digest literals in `land_statement.rs`.
All inputs are synthetic and the wide integers exercise full ABI widths.


## L1 raw-header native vectors

`l1-native-cases.tsv` contains 159 independent native-decoder cases (41 accepted,
118 rejected). Each row has a neutral case label, exact raw header hex and either
`ok` followed by raw hash, parent hash, state root, number and timestamp, or `err`
followed by the native diagnostic. Tests compare acceptance and all accepted
fields; diagnostic text is retained as provenance, not a stable error ABI.

The oracle compiled the unmodified
[native raw-header decoder](https://github.com/taikoxyz/taiko-mono/blob/af1b952adb2d14263da60c5090e15e65b5d8d48c/packages/taiko-client-rs/crates/abci/src/l1/header.rs)
against that client's locked Alloy consensus 2.5.0 and RLP 0.3.16. Raiko2 consumes
the vectors using its existing consensus 2.4.2 and the same RLP 0.3.16. The parser
source SHA256 is `e848a79c09958309b3fecd2434676626a849d2999faad749fd70bdae643ba3a9`.
The corpus SHA256 is `480fb9c22e137e0be70a2401b09b7117d461bf732d4e0aa9c5c62e160a644732`.

Only `mainnet_26155639` is a real Ethereum header, copied with its expected fields
from the same revision's
[public mainnet fixture](https://github.com/taikoxyz/taiko-mono/blob/af1b952adb2d14263da60c5090e15e65b5d8d48c/packages/taiko-client-rs/crates/abci/src/l1/testdata/mainnet_header_26155639.json).
Other cases exercise the positional wire parser, mutations and synthetic linked
headers; they are not full valid Ethereum blocks or evidence of L1 finality.
Coverage includes future opaque fields, 4096/4097 byte boundaries, canonical
integer/hash widths, truncated/nonminimal framing, raw-hash substitutions,
contiguous ancestors, height gaps and block-zero underflow. Unused nested list
payloads remain opaque exactly as in the native parser; only their top-level item
framing is checked. All 41 accepted hashes were independently recomputed from
the original bytes with a separate Keccak implementation.
