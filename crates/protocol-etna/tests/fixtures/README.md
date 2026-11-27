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
with powers 10 and 30. The current native body tests use these bytes directly through
the shared core.
The superseded certified-envelope gate and its caller-context wrapper were removed;
range committee and native certificate checks have their own integration tests.
These signed stock bytes retain their historical four-field envelope. They are
used only as an oracle for native protobuf/body commitments, and do not establish
acceptance by the current Etna envelope decoder. Current projection and payload
fixtures use exactly `[block, anchor, forced]`; the projection rejects four fields.

## Etna landing ABI vectors

`land-input.hex` and `land-statement.hex` contain one ABI word per line from the
[Etna design at `4bb5cff1`](https://github.com/taikoxyz/taiko-mono/blob/4bb5cff1e9e4f96506a8ee971e430014188619ac/packages/protocol/docs/etna/SPEC.md),
section 6.1 and rules ENC-A-29–ENC-A-35: seven input words and nineteen statement
words. The word layout was assembled from the specification and checked against
Foundry `cast abi-encode`. Foundry
`cast keccak` produced the expected digest literals in `land_statement.rs`.
All inputs are synthetic and the wide integers exercise full ABI widths.
