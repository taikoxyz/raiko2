# Ethereum mainnet L1 history witness

`mainnet.json` combines two frozen public execution-header membership proofs under Q at execution block 26,298,983. Its SHA-256 is `9451624cc7370fa775126075ff5a85ac0c673b3ee320b7a5673ea2d693a47413`. The tests also open Q directly. The snapshot exercises native authentication against the supplied statement hash. Landing, current finality and archive availability are outside its scope.

| Block | Execution hash | Beacon slot |
| --- | --- | --- |
| 25,198,983 | `0xf522b27126a2a639c7bfe1b73f6afb8f15718530eec2ffbbc55ad6f7b23aab74` | 14,434,093 |
| 26,294,192 | `0x953010d1d26d440b970e50a344dcb3c600417007d7dcad86a4b641f99b27db52` | 15,533,510 |
| Q | `0x8f2cf6cd35268b78a0ced65a2ab1f2c865675eb9ed063de464330b2151a6e129` | Parent beacon at 15,538,327 |

The `q_raw` field contains the complete canonical 21-field execution-header RLP reconstructed from the archived JSON-RPC header fields. Its Keccak-256 matches Q's recorded hash. The historical source supplies the original RLP for each target, a 60-sibling completed-period proof and a 34-sibling recent proof. [Historical experiment and original public fixtures](https://github.com/taikoxyz/raiko2/blob/170e0fd7d965b0ee20ac8f9d33295f8fb5d217c6/docs/research/2026-10-29-ethereum-historical-header-proof.md)

Both execution hashes use nine siblings to reach their respective body roots. The complete cached target beacon blocks supplied the body roots and five header fields. The next 13 vector siblings in the completed-period proof open period 1761. The recent path opens offset 1478 of the reference state's block roots. The post-state's cached 38 field roots reproduced its root and supplied the branches for slot and fork. Combining those branches with the two history branches retains the exact missing siblings at ascending generalized indices. The Fork version chunk contains `06000000` followed by 28 zero bytes. The list length chunk contains 1138 as a padded little-endian integer.

| Original public input | SHA-256 |
| --- | --- |
| Committed completed-period fixture | `7eabbeeff217fa8295730bc0fd8b01739560decd8e2f20ca3f9348026fbddea7` |
| Committed recent fixture | `b72e00c404a0159995a8bc6fbd5da5fd4a4adc699ef0e79a0b197495efb80c52` |
| Cached execution RPC batch | `0a93719d8126547b2b654be6a774801c2fd70e3fe717045d11c6dbbde017816b` |
| Cached reference state field roots | `9d4715f6c8b5d3bd7794b0ab9b4a83345b16d474aa7a0990f46e946cf0c6c41d` |
| Cached completed-period target beacon block | `b63907ccc92550aa1dfedd8dc5d6a9eb85fb1169b9c0ea44e336e25e4327aba8` |
| Cached recent target beacon block | `c93de7e05489cfa437ae0d502b6bd5249dac453992d6ad13fd19d192ce3c3d4e` |

The verifier uses the mainnet preset, consensus container definitions through Fulu and the chain-specific activation schedules. A configured Sepolia Gloas boundary ends the supported consensus schemas there. [Consensus schemas](https://github.com/ethereum/consensus-specs/tree/8e7f4dc7128ac850299599a548dcd06a1ca0a8cd) · [Mainnet schedule](https://github.com/eth-clients/mainnet/blob/980aee8893a2291d473c38f63797d5bc370fa381/metadata/config.yaml) · [Sepolia schedule](https://github.com/eth-clients/sepolia/blob/ddd2eb069df914017a3a40672dfb56e7e895e129/metadata/config.yaml) · [Hoodi schedule](https://github.com/eth-clients/hoodi/blob/2a7efb3c530cce7c71702cc98d0f965df2411c11/metadata/config.yaml)
