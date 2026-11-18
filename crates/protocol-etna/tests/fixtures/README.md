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
