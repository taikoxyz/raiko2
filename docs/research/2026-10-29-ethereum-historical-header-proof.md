# Ethereum historical-header proof experiment

Checked on 2026-10-29 for the [Etna proving design](../superpowers/specs/2026-10-29-etna-pos-proving-design.md).
This is an offline cryptographic inclusion experiment, not an implemented Raiko2 guest or a
zkVM/STARK/SNARK proof. No protocol or deployment was changed.

## Exact case and result

**PASS:** execution header 25,198,983 was authenticated to a beacon root read through EIP-4788
at execution height 26,298,983: 1,100,000 blocks / 153.365972 days apart. The complete proof is
committed as a [small JSON fixture](fixtures/ethereum-mainnet-25198983-to-26298983.json).

| Item | Value |
| --- | --- |
| Chain / both beacon layouts | Ethereum mainnet (1) / Fulu |
| Target execution time (UTC) | 2026-05-29 05:38:59 |
| Reference execution time (UTC) | 2026-10-29 14:25:59 |
| Target execution hash | `0xf522b27126a2a639c7bfe1b73f6afb8f15718530eec2ffbbc55ad6f7b23aab74` |
| Target beacon slot / period offset | 14,434,093 / 7,981 |
| Historical period / summary index | 1,761 / 1,003 |
| Recent beacon slot / summary count | 15,538,327 / 1,138 |
| Recent beacon root | `0x52bf8fb3cb12528107b660206866a581686713c9c513b1f274ebebc3cb1ab0d4` |
| Recent state root | `0x11ac0bba14dceeed18170118c3c3786d6caf50830db0b9333f3d3f712b7e6048` |
| Generalized index / branch depth | 1,646,065,731,146,799,404 / 60 |
| Merkle siblings / explanatory JSON | 1,920 bytes / 7,663 bytes |
| Fixture SHA-256 | `7eabbeeff217fa8295730bc0fd8b01739560decd8e2f20ca3f9348026fbddea7` |

## What was verified

- Re-encoded both execution headers as RLP and recomputed their Keccak hashes. The reference
  header's parentBeaconBlockRoot matched the EIP-4788 call result at that exact execution height.
- Recomputed the complete old beacon block SSZ root, including its execution payload; its block
  number/hash matched the target header. The header RPC reported the old block canonical/finalized.
- Read a 1 MiB HTTP Range from a public Nimbus era archive. Checked the Snappy-frame CRC32C and
  extracted the 262,144-byte block_roots vector from boundary state slot 14,434,304.
- Matched vector item 7,981 to the old beacon root and recomputed the period root. It matched
  historical_summaries[1003].block_summary_root in the recent state.
- Rehashed all 38 fields of the 339,180,254-byte recent Fulu state and matched the authenticated
  state root. Built 12 execution-to-old-beacon siblings, 13 old-beacon-to-period siblings, and
  35 period-to-recent-beacon siblings; independently folded the complete 60-level proof.
- Verified another EIP-4788-authenticated recent root at slot 15,538,240 using the same lower
  25 siblings. The full proof's upper branch was regenerated for that root.
- Independent review and independent behavioral verification found no unresolved findings.
  All 15 focused tests passed, including each of the 60 sibling mutations, wrong external root
  or target number, wrong slot/index/list length, malformed RLP and truncated/extended branches.

Pure Python full-state hashing took 65.56 seconds; warm verification of the small complete witness
had median 0.095985 ms and p95 0.108084 ms over 1,000 iterations. These are **native host** timings,
not zkVM cycles or proving times. The full state is witness-construction data and does not enter
GuestInput. In the selected hybrid design, only the latest anchor gets this bridge; earlier
anchors retain their existing header links.

## Recheck the committed witness

From the repository root, use an existing Python virtual environment with `Crypto.Hash.keccak`
(PyCryptodome). This small check independently hashes the target RLP bytes and folds the branch
against a fixed, externally authenticated root. The fixture must not select its own trusted root.

```python
import hashlib
import json
from pathlib import Path
from Crypto.Hash import keccak

fixture = Path("docs/research/fixtures/ethereum-mainnet-25198983-to-26298983.json")
raw = fixture.read_bytes()
assert hashlib.sha256(raw).hexdigest() == "7eabbeeff217fa8295730bc0fd8b01739560decd8e2f20ca3f9348026fbddea7"
proof = json.loads(raw)
expected = bytes.fromhex("52bf8fb3cb12528107b660206866a581686713c9c513b1f274ebebc3cb1ab0d4")
header = bytes.fromhex(proof["target"]["rlp_header"][2:])
leaf = keccak.new(digest_bits=256, data=header).digest()
assert leaf.hex() == "f522b27126a2a639c7bfe1b73f6afb8f15718530eec2ffbbc55ad6f7b23aab74"
siblings = [bytes.fromhex(value[2:]) for value in proof["proof"]["siblings"]]
assert len(siblings) == 60 and all(len(value) == 32 for value in siblings)
index = 1646065731146799404
assert proof["proof"]["generalized_index"] == index
assert siblings[50] == (1138).to_bytes(32, "little")
node = leaf
for sibling in siblings:
    data = sibling + node if index & 1 else node + sibling
    node = hashlib.sha256(data).digest()
    index >>= 1
assert index == 1 and node == expected
print("PASS: header 25198983 -> authenticated beacon root, 60 Merkle levels")
```

This is a fixed-fixture reproduction check, not a production codec. The fixture's reference
execution number/hash/timestamp describe the separately performed onchain root lookup; Merkle
folding alone does not authenticate that metadata or establish Ethereum finality.

## Collection and protocol caveats

Publicnode served the old blocks but returned HTTP 501 for full-state export. Nimbus archive
Range reads and an independently root-checked Nimbus state completed this case without any
pre-captured proof. Historical commitments persist in current state; availability of old leaves
and branch material depends on archive/retention policy and is not guaranteed by an arbitrary RPC.

The old period must complete before its fixed summary branch exists. The upper branch depends
on the chosen recent state. EIP-4788 root queries have a rolling window, so an old completed proof
needs its original root authenticated/retained or a newly bound wrapper; its root cannot simply
be replaced. Recent uncompleted periods use block_roots, a path not measured in this experiment.
SSZ paths depend on fork layout: the Fulu state historical_summaries field has gindex 91, not the
earlier 59. Production integration and guest performance remain unimplemented.

## Pinned sources

- [Consensus schema revision 8e7f4dc](https://github.com/ethereum/consensus-specs/tree/8e7f4dc7128ac850299599a548dcd06a1ca0a8cd):
  Fulu BeaconState, Electra BeaconBlockBody, Deneb ExecutionPayload and Capella history accumulation.
- [EIP-4788](https://eips.ethereum.org/EIPS/eip-4788): root lookup and its timestamp window.
- [Nimbus archive](https://mainnet.era.nimbus.team/mainnet-01762-19d748eb.era): bytes
  662778955..663827530 of the compressed boundary-state record.
- Public exact-state source: `http://testing.mainnet.beacon-api.nimbus.team/eth/v2/debug/beacon/states/0x11ac0bba14dceeed18170118c3c3786d6caf50830db0b9333f3d3f712b7e6048`.
- Execution RPC: `https://ethereum-rpc.publicnode.com`; beacon blocks/headers:
  `https://ethereum-beacon-api.publicnode.com`. These identify sources used, not retention promises.
