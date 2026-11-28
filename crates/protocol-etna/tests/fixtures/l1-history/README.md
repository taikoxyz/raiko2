# L1 history witnesses

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


## Captured internal L1 recent-window witness

`devnet-recent.json` contains the existing owned `L1HistoryInput` representation for
internal **L1 chain 32382**, frozen from read-only observations on **2026-11-28**.
Its SHA-256 is `f074bc005a0613fa4b7a698160da1eac762f62ef052ecd475a068f4387daee85`.
This is L1 history evidence; it does not establish an L2 execution, a live enclave
signature, an on-chain landing, or current finality.

| Opening | Execution height | Execution hash | Beacon slot | Age from Q's parent beacon |
| --- | --- | --- | --- | --- |
| Older anchor | 53,231 | `0x590143c5d129d9f81b2ebd4f23b64f496d35f63ac781d04e0846cc09d7784a6f` | 82,735 | 7,200 slots / 24 hours |
| Recent anchor | 55,363 | `0x399830a439e033acdfe9f80bdd599cc613fc2ef1b6397cba3dc19bd9ea47d01c` | 88,735 | 1,200 slots / 4 hours |
| Q | 55,788 | `0x4d952b19a2fb11c494570a8b10de9c126b60cad4d0e1424b945c77a42750a10b` | 89,936 | Parent beacon slot 89,935 |

Q's raw canonical execution-header RLP commits parent beacon root
`0x52bb9361b8e07410807867be48e806348da82add04fe546d663336164062fc38`.
The corresponding beacon header commits state root
`0xba2cb2c025672b7d7b857e3a1ea653a004061bebde354c24fa21de4b840dfc07`.
Full reference-state and full target-block SSZ roots were recomputed from the
captured JSON; every raw execution header reproduces its captured Keccak-256 hash.

Both targets use nine payload-to-body siblings. One shared state multiproof has
32 necessary helpers in ascending generalized-index order. Its opened leaves are
slot `66`, fork current version `269`, and recent block roots `566063` and `572063`.
No historical summaries, legacy periods, additional genesis membership openings,
raw-chain fallback, or host-selected network constants are added. Q is still an
externally supplied exact number/hash. An altered Q identity or detached opening
must fail; a correctly rebuilt subset authenticates only that subset. The guest
composer separately enforces exact opening coverage against the projected EL range.

### Compiled internal deployment profile

The internal profile is pinned to captured genesis time **1794749112**, 12 seconds
per slot, 32 slots per epoch and an 8,192-slot recent-root vector with compatible
Fulu dimensions. The captured `PRESET_BASE` is `interop`.
Bellatrix, Capella, Deneb, Electra and Fulu activate at epoch 0 with versions
`0x20000091`, `0x20000092`, `0x20000093`, `0x20000094`, `0x20000095` respectively;
Gloas is disabled at epoch `18446744073709551615`. The captured genesis validators
root is `0x83431ec7fcf92cfc44947fc0418e831c25e1d0806590231c439830db7ad54fda`
(provenance, not an additional proof leaf).

These are reviewed image constants for this captured deployment, not authoritative
values taken from a proving request or an arbitrary configuration override. Older
default genesis settings do not describe this snapshot. A reset that reuses chain
32382, or a schedule/preset change, requires review of the actual deployment and an
explicit profile/fixture update. Matching old proof bytes to their frozen Q is not
a claim about a later deployment. Unknown profiles remain rejected; the existing
public-network checks and direct-Q proof behavior are preserved.

| Captured primary source | SHA-256 |
| --- | --- |
| `genesis.json` | `94ee8d5929aa7fb7e4cc37a2ca668817642c9a9ad36876d36102fed3037877e3` |
| `spec.json` | `88d23b0d2faeda4a20c85823c99fe06db0c62f020229ab801c2e023892aaf2a4` |
| `execution-rpc.json` | `2e67cca7b257352f2dfd86a543074533a4b623fd40e3d3bb27308b6558b7bfda` |
| `parent_header.json` | `8026f13803273431a050b763633aa9230b90db9f8d9a51325aee271d676043ed` |
| `reference-state.json` | `db059568acdf7dc46d053e54fa935a93c322f3c30d3febdde894703c20814b2d` |
| `4h_block.json` | `8449d3967b30783000b31c8a970646894191a191faff28004659c83d9fadac0a` |
| `4h_header.json` | `2e5259a03757f13f3f7fd64e043eb7c5abf276130db5a6edfbdd18a9b9f095d5` |
| `24h_block.json` | `a4b5f78217c4804cf3760ff68960432cc980ad42760295c4dd99970d04fcc2dc` |
| `24h_header.json` | `a062169cd9f15473e8c6ae0ece9a3e7331fd0042e56be89f9b4b2767d1fbee3d` |

The source names identify the frozen standard Beacon API genesis/spec/header/block/
debug-state responses and the execution JSON-RPC response batch. The compact fixture
contains proof material only, not the full state dump or an endpoint-specific client.
The SSZ helper schemas use the same consensus-spec revision pinned above. Recreating
this fixture was an offline conversion, not a reusable live proof-generation service.
Native test execution and the SGX composition remain distinct validation gates.


### Fresh portable-producer output

`devnet-live.json` is a second frozen input from the temporary portable producer,
using explicit Q **55,995** and anchors **53,995 / 55,595** on 2026-11-28.
Its SHA-256 is `3f903b00e5826b749bbed212f78de71ac323615d8a653702f42b2554b913e0a4`.
The producer input itself has SHA-256
`3c762381cf29ebc5c0bdaa3cd3eb122849a28ea025f8bbd688f79e8d2de15111`.
One reference state and 32 shared helpers authenticate the two anchors. The
native test deserializes this actual producer output through the existing owned
carrier and verifies its exact Q and both header/state-root identities. No live
network call or new transport codec is part of the native test. A passing native
CI run is required before treating producer-format/native-history replay as
verified; complete SGX range verification and L1 acceptance remain separate gates.
