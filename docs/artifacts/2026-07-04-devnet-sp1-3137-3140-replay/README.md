# Devnet SP1 proposal 3137-3140 replay artifacts

This directory preserves the non-secret local artifacts used to replay the old devnet proposal 3137-3140 SP1 reserved-network issue after `/tmp` cleanup.

## Source requests

The originally reported old Succinct reserved request was proposal 3138:

```text
https://explorer.reserved.succinct.xyz/request/0xc5a67d4736b686df16d189f9a4dcabf08bb335402da94d5d2356dbce41c3e5ac
```

The adjacent proposals were replayed with the same old proposal ELF and their preserved local guest inputs:

| Proposal | Preserved input | Source mtime | SHA-256 |
| --- | --- | --- | --- |
| `3137` | `devnet-3137-guest-input.2026-07-03.json` | `2026-07-03 09:45:17 +0800` | `a9b40ea4fe8e685685567dec09a31ac353950ef4c775ffb94ea17eccb7b570d8` |
| `3138` | `devnet-3138-guest-input.2026-07-02.json` | `2026-07-02 20:03:46 +0800` | `4c068c217427980991068b08ecb69a56522b5556da71cf258dab0b8750de1b1c` |
| `3139` | `devnet-3139-guest-input.2026-07-03.json` | `2026-07-03 09:45:33 +0800` | `229657d9e1c1be3cbd64d7694c0c76aeb698042082eeb5428ae501fb0dd8b840` |
| `3140` | `devnet-3140-guest-input.2026-07-03.json` | `2026-07-03 09:45:41 +0800` | `198c0e001e76edcad3e9ff71355bec13395cc3903f810709ccdb9634f35addfa` |

All four inputs use devnet L2 chain ID `167001` and verifier `0xcb2d625bf8c2187b180646af4738af5f1413fb70`.

## ELF-ready stdin files

Each JSON input was converted into a single SP1 stdin buffer by deserializing it as
`raiko2_primitives_shasta::GuestInput` and serializing that value with `bincode`.
This matches the original host-side `SP1Stdin::write(&input)` path.

Use the `.bincode` files with:

```rust
let stdin_bytes = std::fs::read("devnet-3138-guest-input.2026-07-02.bincode")?;
let stdin = sp1_sdk::SP1Stdin::from(&stdin_bytes);
```

| Proposal | ELF-ready stdin | SHA-256 |
| --- | --- | --- |
| `3137` | `devnet-3137-guest-input.2026-07-03.bincode` | `46b45d8e28e0e06902bfa0580df3559ec8c327c6154b1350e5cccc6aa6d37202` |
| `3138` | `devnet-3138-guest-input.2026-07-02.bincode` | `2de08bf47e5db1dcb0a175ab447db5f015187548c479299c5348fd51be13082e` |
| `3139` | `devnet-3139-guest-input.2026-07-03.bincode` | `7a1b7737ddaccf1299ed9a931d1798a956dd533e10868a544b7e4689b42d382a` |
| `3140` | `devnet-3140-guest-input.2026-07-03.bincode` | `d4bca57b8b47464d80235e67d22155bafea3db7049c6c6dc362a1a8c548f1813` |

## Checkpoints

| Proposal | Checkpoint block | Block hash | State root |
| --- | --- | --- | --- |
| `3137` | `0xc41` | `0x808c6280726831ed78ef01121f42d0aa920eaabe083903b814ca48bb16ea8db4` | `0x53fadaa39480540a89b7af3fa5c337231ea86411f2700a9c923554e187187ba1` |
| `3138` | `0xc42` | `0xb96c89a963dcd06ac18ea4e0cfc961a694662acd3ab5d170c5d48c0fccc54588` | `0xf93cea1a356fd76a945fb3ade90e7d89c5f571a13e317013a9b46835db32a158` |
| `3139` | `0xc43` | `0x28b21f9c590e720d8fa29b308b5383cc09bd244ec13bf95fb751e6e0527154ec` | `0xdffb477cd635d2780479ef350f943c2f66b8fd5934cce4237997de02d2e98c78` |
| `3140` | `0xc44` | `0x136c0b22b9f3c31bdcb8c9bc53247aa1a974b61171b6d47f8f542c6e76488745` | `0xf15ae9cfdec7da71fe2f9b5d5d186628506a87dbae6ade4009526333b23fc85b` |

## ELF provenance

The proposal ELF and VK were extracted from the old devnet image:

```text
us-docker.pkg.dev/evmchain/images/raiko2@sha256:cd62cd1700d60db16f202cd17769047fb19e32c6f7638861918680a2186dd255
```

Preserved files:

| File | Source mtime | SHA-256 |
| --- | --- | --- |
| `sp1_shasta_proposal.elf` | `2026-07-04 13:23:47 +0800` | `b457418f43d1757408ab238426328f1e54e369940d769d40912d07402356483a` |
| `sp1_shasta_proposal.vk.bin` | `2026-07-04 13:23:47 +0800` | `7bdf5804218a77b3b086eb2df9c3dab3852ed9ec4c61c1cc7ac4e69fe9bfe202` |

The proposal VK hash bytes matched the old Succinct request program:

```text
0x72ec6fb728fa6a841ce136c05a847c59263f17c0652abe6f5ee13ca90c9d862d
```

The aggregation ELF from the same extracted bundle was not copied here because the problematic requests were proposal proof requests. Its SHA-256 was:

```text
bdb67d59beae8f94c902fd1a036b8bc66f3cc9f166a7668ca5140778f26f5749  sp1_shasta_aggregation.elf
3a449a6d45d9332fd53db0586d8f979d66efd77d9ac326ed0bdab5b3cf537541  sp1_shasta_aggregation.vk.bin
```

## Replay results

| Proposal | Succinct request | Public values | Wall time |
| --- | --- | --- | --- |
| `3137` | `0x8f43d1472b1137b93055f2293404fd1c39069e9e47c6618891b100cc2407033a` | `0x20cd07ded43c25605558f46d274ba746d878ad156b21b54a6371d5a20b60d967` | `0:48.28` |
| `3138` | `0x8bc2c6861a241e4da1655afd2d57e466d28677ee83e32dfb7c250196ecf93950` | `0x4cb88870391cdc620629f87552a38dff218421420a7a515ad39c97162a7b1028` | `0:43.13` |
| `3138` | `0x80d0e0e2e7ddacc0171d1162774c4dba32ecec392cc355b52dd30a09c082b918` | `0x4cb88870391cdc620629f87552a38dff218421420a7a515ad39c97162a7b1028` | `0:49.94` |
| `3138` | `0xe1e403e9dda67613dc0572e65e3c248caa4d5614e028a817a0fb4ffa0762d6c9` | `0x4cb88870391cdc620629f87552a38dff218421420a7a515ad39c97162a7b1028` | `0:49.67` |
| `3138` | `0xc191c0c9047822287be27749b9940d09c904c8148e28ef2356122c9626be523b` | `0x4cb88870391cdc620629f87552a38dff218421420a7a515ad39c97162a7b1028` | `0:45.55` |
| `3139` | `0x57373fc40e6210f6f026c50e9e62603eba0fac306fb44c4358286742a21108b9` | `0x73ff9f06f81911b68debf42355a758e09f39d719c9254197bb7accb809d881a9` | `0:52.93` |
| `3140` | `0x1d5d5ccc7b281d349facc57dc0844b2392c1126977d1be4aa73ac274b409deb9` | `0x602d87b0eebc8b7cb9ae7dacd06e8dcde2c242baa4fd1e0a057a0f52890181e2` | `0:49.93` |

The per-proposal network reports and command logs are preserved as
`devnet-<proposal>-sp1-network-report.2026-07-04*.json` and
`devnet-<proposal>-sp1-network.2026-07-04*.log`.

## Bundle

`devnet-sp1-3137-3140-succinct-replay.zip` contains all preserved non-zip files:
the proposal ELF, VK, four ELF-ready `.bincode` stdin files, four original JSON
guest inputs, replay logs/reports, this README, and `SHA256SUMS`. It is intended
to be attached to the public tracking issue so Succinct can replay the four
proposal inputs directly.

## Notes

The preserved guest inputs were not downloaded from Succinct; the reserved explorer did not expose original stdin.

- Proposal 3138 is the local preflight output generated during the original 2026-07-02 investigation and later reused for the 2026-07-04 replay.
- Proposals 3137, 3139, and 3140 are the local preflight outputs from the 2026-07-03 four-proposal investigation.

`SHA256SUMS` contains checksums for all preserved files.
