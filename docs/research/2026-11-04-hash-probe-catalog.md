# Concentrated hash-probe catalog

A deliberately concentrated 512-key development probe exposes serious underpricing in the retained historical b76 program with SDK 6.3.0: R512 consumes **3,378,306,245 PGU at 4,894,506 charged block gas**, exceeding the literal node quote by **3,218,491,169 PGU** and the complete-proposal quote by **3,063,418,266 PGU**. It meets the research priority rule: actual-minus-node strictly greater than 2B PGU within 30M charged block gas. The model is not closed by the preceding finite panels. The next task is to identify and account for the missing block-observable work, then validate a proposed correction separately; no price or model changes accompany this record.

Exactly two new actual executions completed once, own-zero followed by probe. The qualified full R2028 input remains **unmeasured**: the measured probe already met the priority rule, so no full run was launched. This is a development probe (`holdout=false`, `actuals_used_for_fitting=false`), not an unopened holdout, and it was not used for fitting. It is separate from the [original warm-read baseline](2026-11-04-warm-read-catalogs.md), the [observer conformance record](2026-11-04-sp1-register-observer.md), and all fourteen prior pricing contexts. No union or historical count is revised.

## Results

Actual PGU is the canonical integer normalization `raw_gas * 10 // 191`. Node and complete-proposal values below are the original literal quotes under the frozen current calibration profile, not an unscaled historical tariff. Gaps are signed actual minus estimate.

| Input | R | Charged block gas | Actual PGU | Node PGU | Complete proposal PGU | Node gap | Proposal gap |
|---|---:|---:|---:|---:|---:|---:|---:|
| Own zero | 0 | 1,425,706 | 195,194,698 | 20,832,507 | 175,905,410 | +174,362,191 | +19,289,288 |
| Probe | 512 | 4,894,506 | 3,378,306,245 | 159,815,076 | 314,887,979 | +3,218,491,169 | +3,063,418,266 |
| Qualified full | 2028 | 15,165,406 | **Unmeasured** | 571,333,777 | 726,406,679 | — | — |

Probe actual/node is approximately 21.1388457807; actual/complete-proposal is 10.7285970577. The zero control also has both absolute deficits, but does not meet the priority threshold. These absolute findings remain distinct from the signed own-zero increment:

| Probe minus own zero | Signed difference |
|---|---:|
| Actual PGU | +3,183,111,547 |
| Node PGU | +138,982,569 |
| Complete proposal PGU | +138,982,569 |
| Actual minus either estimated increment | +3,044,128,978 |
| Charged block gas | +3,468,800 |

The user envelope, all signed transactions and calldata match between zero and probe, and the primary has authenticated EMPTY storage in both. Runtime, required code set, genesis/final roots and complete GuestInput differ. These are conditional whole-input diagnostics, not isolated per-lookup PGU or a causal attribution of every extra instruction.

## What was concentrated

The maintained [std-only hash utility](../../bin/zkgas/bench/hash_probe_catalog.rs) implements the independently disassembled four-u64 specialization, with u64 operands and a full u128 product:

```text
F(x,y) = low64(x*y) XOR high64(x*y)
A = F(k0 XOR S, k1 XOR G)
H = F(k2 XOR A, k3 XOR G)
```

Here k0 is the least significant limb. Search varies the highest 32 key bits through a deterministic permutation, preserves the low 224 bits and nonzero high-byte constraints, excludes duplicates/query, and appends the query last. The 16KiB calldata retains its zero/nonzero byte counts and long common key prefixes. It targets `H & 1023 = 956` plus `H >> 57 = 44`: **17 selected bucket/tag bits, not full-hash collisions**. One bounded search retained 511 candidates plus the query after 64,795,286 trials, with 294,814,796ns reported search time; inverse-counter and all512 hash checks passed independently. The configured bounds were 2^28 trials and 120 seconds, with deadline checks every1024 trials.

Observed seed `0xff5779f5c8e3fd0c` and shared word `0xefaa3143bec6736f` began as hypotheses from the old qualified observations. Both newly signed measured inputs independently expose all512 own cold keys/full hashes matching the selected bucket/tag, and first-warm mask1023. Their observed seed/shared values match the hypotheses. This realized gate was required before launching probe; seed stability was not assumed from equal calldata shape.

Selected register transitions and chronological catalog/map/key-pointer correlation agree with the declared source path. The source/ELF explanation supports a mechanism involving concentrated journal-map probing and key comparison, but does not turn prefix counts into per-lookup PGU. Direct owner address, warm-key lower128 bits, whole table occupancy and the other shared seed words are unexported. This record does not establish their values or arbitrary new-input seed stability.

The [single maintained constructor](../../bin/zkgas/bench/cases/warm_read_catalogs.py) exports zero/probe/full with distinct actual64R IDs and their own-zero relation. R512 executes 32,769 warm reads including the guard; R2028 would execute129,793. Full+1 fails the single-user15M fixture envelope, not the30M block budget. Even the qualified full charges only15,165,406; neither measured input establishes near-full occupancy or global packing.

## Qualification, execution and retained failures

Fresh independent source/spec and quality review passed the complete seven-file source increment. Final retained author checks passed: direct rustc build; three Rust unit tests; focused Python tests with ten passes and one optional-native skip. Default primary4/custom2 requests, rows, bytecode, gas and envelopes retain their frozen identities without the new option; helper provenance changes are explicit. Scoped source inventory is197 files, separate from native's81 declared files/66 declared Rust/70 compiled workspace entries.

Independent hash verification checked1024 observed tuples across two old512-key captures, four integer edges and eight negative CLI cases. Qualification then used one canonical export, three fresh native replays, three fresh original quotes and independent byte/gas/counter/signature/genesis/root/required-code checks. All three original quote paths/hashes/lengths are explicitly indexed and members of the167-file preactual qualification manifest. Native acceptance alone is not a per-read key/value trace or an intent oracle.

Not every command passed on its first attempt. A pure preparation constant exceeded U256 and was corrected before freeze. Source development retains expected RED failure and an intermediate failed focused run before final checks passed. The independent V1 qualifier selected the memory0 seed guard when matching R512. Additive V2 binds the actual PUSH3 memory64 prefixes at offsets205/545 while retaining the512 seed guard at501; six old-target tests, two byte-mutation negatives and all custom3 byte/gas/counter checks passed. Original scripts, error and outputs remain retained. None of these corrections retried the producer, native, quotes or actual executions, and no old measurement was rerun.

| Execution scope | Own zero | Probe |
|---|---:|---:|
| Canonical raw gas | 3,728,218,750 | 64,525,649,298 |
| Canonical instructions, complete | 178,940,001 | 4,279,435,409 |
| Observer processed instructions | 178,940,001 | 400,000,000 |
| Observer status / completed chunks | complete / 1 | instruction_limit / 0 |
| Retained samples | 1,698 | 1,924 |
| Guarded wall seconds | 11.924717 | 100.986323 |
| Peak sampled process-tree PSS bytes | 1,397,467,136 | 4,552,135,680 |

Retained samples are not exhaustive in either input. Probe's observer stops at its declared400M prefix; **canonical GasVM completes independently**, so the prefix limit is not an incomplete PGU estimate. Both ran within the unchanged6,144MiB sampled-tree PSS/600-second actual guard,134,217,728 memory-entry/two-slot producer cadence,16MiB log/192MiB stage/576MiB disk policy; native/quote limits remained180/1800 seconds. Sampling does not guarantee a continuous memory peak. Device-matched original-host receipts verify all eight exact runtime names absent after zero/probe (shared-memory device27, mount namespace4026531841); sandbox absence is not substituted for host evidence.

## Frozen identities and command receipts

All hashes below are SHA256 except Git revisions and the semantic model ID. Original167-member qualification and final195-member manifests have distinct identities; root independently verified all195 final members, raw arithmetic and host receipts.

| Identity | Value |
|---|---|
| Reviewed tool/constructor Git revision | `27085e7f5f7b9518307467b02254ae07b8636aea` |
| Source provenance | `bfbf3333852d3c4bb2985a57e99e6227f06bccd412314504178d6b4d48b72bc3` |
| Hash executable, rustc1.95.0 | `ecfbc41357124d6ad218838df5647832c4499e974448b689df55340891d240c3` |
| Observer Git revision | `cfe6971929cea0df658c76a30e1aecacef95e6f2` |
| Observer executable | `dfaee7ac41099df02e8a66d9b15b6c4b53d539d1a7e051a67b7180c64d6a0992` |
| Observation plan | `ec752035f7a3fd7e47168109cc203682bde4a54d445c9c3fef5b5f495c7d73be` |
| SDK / guest source | `6.3.0 / 4580faf7dc009714b310fd625b5810667019d683` |
| Fixed b76 ELF | `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034` |
| Model semantic ID | `c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef` |
| Model artifact | `4cb6c65c6ad28e25e1f30639803958f9555e43d51e6d35d2f1945b0757ce1ac7` |
| Quote executable (original receipt marks dirty) | `c10eae230681d9938b99b3cadf2b77c50d6af7ca8dbb934252ebf550cf10535e` |
| Native executable | `ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec` |
| Catalog JSON | `8301bad4f43bb05a0c9f9419c99955fd46362ccfd50996072b9e403f29c3dccf` |
| Calldata / sample-ID prefix | `6cd1df4f5963a66055df2c2304103807bb75ac4ea7da8b90750e6341a3755a5c` |
| Own-zero canonical input | `cd4cf24a53af2e6fcc80096a5609f42be12f70485c0641a2c32e7e351ffa8a89` |
| Probe canonical input | `3c8ccba435477d7262dae316ec97baa48868887f6a09f431ee8e17530eb4370e` |
| Unmeasured full canonical input | `58faed9731a3d2605e8f6a93f7e1a4ee969715b592de52cbce15b33454cc2cc4` |
| `qualified-artifact-manifest.json` (167) | `e4e6a8014023c466d026711aa32551e1b9194197615b18fa0e5263c5d2c17ede` |
| `final-artifact-manifest.json` (195) | `f5f365078a8b68cac5910849a951ab7c62135a6a7f8b9c6ee6d8ef1e500bea37` |
| `final-independent-report.json` | `08a3296c0675caa622661f6ffc92c8870751c42528538bc9c08bc56ea65c896a` |
| `actual/probe/verification.json` | `bfab53805094458173ee933bcc5777c47416174671473389a83bcafa8e31ae4f` |

The retained command receipts use the following exact argument forms, with machine paths replaced by neutral configured variables. These summarize commands already run; this documentation step repeats no build, search, native, quote or actual. Source checks are author executions authenticated by the independent source review; later vector/qualification/actual checks are independent executions.

```bash
rustc --edition=2024 -O -D warnings bin/zkgas/bench/hash_probe_catalog.rs -o "$HASH_EXE"
rustc --edition=2024 -D warnings --test bin/zkgas/bench/hash_probe_catalog.rs -o "$HASH_UNIT_EXE"
"$HASH_UNIT_EXE" --nocapture
"$PYTHON_BIN" -S -B -m unittest discover -s bin/zkgas/bench/tests -p 'test_*catalog*.py' -v
"$HASH_EXE" search --seed 0xff5779f5c8e3fd0c --shared 0xefaa3143bec6736f \
  --query 0x1122334455667788000000000000000000000000000000000000000000000000 \
  --bucket-mask 1023 --max-trials 268435456 --max-seconds 120 --output "$CATALOG_JSON"
"$PYTHON_BIN" -S -B bin/zkgas/bench/cases/warm_read_catalogs.py \
  --output-dir "$CASES_DIR" --catalog-file "$CATALOG_JSON" --probe-repetitions 512 \
  --model-id c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef
"$OBSERVER_EXE" --input-bincode "$INPUT_BIN" --elf "$ELF" \
  --program-manifest "$PROGRAM_MANIFEST" --output "$ACTUAL_JSON" \
  --resource-receipt "$RESOURCE_RECEIPT" --observation-plan "$OBSERVATION_PLAN"
```

The final invocation was used separately for zero and probe only. Raw commands/outcomes, original quotes, canonical inputs, all failures and immutable manifests remain private retained evidence. The verified private recovery archive `hash-probe-catalog-evidence-20261104.tar.gz` has SHA256 `5a2b4953e904b2071a46f10c4c645c05323cc4b927c4ccda9ae5679585152855`: 1,997,470 compressed bytes, 272 regular members and 11,621,362 payload bytes. Three regenerable compiled unit or earlier binaries are excluded; five external artifacts are retained. Root streamed and hash-verified every member without extraction. This public report is not a standalone reproduction bundle. Maintained tools and tests stay here; later immutable data-only fixtures belong in Atlas.

## Remaining boundary

This establishes a serious local pricing failure in the historical b76/SDK6.3 `evm-gas-only` GasVM profile. It does not certify all production versions, GPU wall time, generated proofs, practical network execution, an operator mining limit at least159.8M PGU, global bounds, IID confidence or arbitrary hash/key/seed cases. The incomplete register prefix does not measure every lookup or full occupancy. No startup-only conclusion follows. Price/model/SDK/guest/ELF/native-builder/pins/resource caps remain unchanged; PR296 stays Draft/do-not-merge pending the broader work. The finding takes priority over expanding this catalog to the unmeasured full point.
