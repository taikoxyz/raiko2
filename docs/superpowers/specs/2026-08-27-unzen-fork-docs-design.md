# Unzen Fork Documentation Refresh

Date: 2026-08-27
Scope: documentation and banner assets only. No code, config, scripts, or fixtures.

## Problem

`README.md` opens with "Raiko2 is a Shasta proof service for Taiko." Every Taiko network has since
forked into Unzen, so the headline names a fork that is no longer the active one.

Confirmed against `alethia-reth/crates/chainspec/src/hardfork.rs`:

| Network | Unzen activation | Date |
| --- | --- | --- |
| `taiko-hoodi` | `1781787600` | 2026-06-18 |
| `mainnet` | `1786021200` | 2026-08-06 |
| `masaya` | genesis | — |
| `devnet` | genesis | — |

Across all repository Markdown, 510 lines mention `Shasta`/`shasta`/`SHASTA`; 160 of those fall in
the 11 files this change touches. Most are correct and must not change: they name frozen wire
strings, on-disk paths, and Rust identifiers. A blind find-and-replace would corrupt live API
routes and stored-artifact schemas.

## Decisions

1. **Naming rule**: Unzen names the active fork; machinery drops the fork word; frozen literals are
   untouched. Fork-neutral phrasing reuses vocabulary `CONTEXT.md` already defines ("proposal"),
   introducing no new terms.
2. **`docs/precompile-status.md`**: full rewrite for Unzen, not a scoping note.
3. **Historical documents stay historical**: `docs/plans/**`, the completed Hoodi rollout runbook,
   and the opcode-gas experiment are not rewritten.
4. **Docs and PNG only.** Code defects discovered during this work are recorded as follow-ups.

## The Naming Rule

Each mention is classified by one test, applied in order:

| Class | Test | Action |
| --- | --- | --- |
| Frozen literal | Is it a string on a wire, on disk, or in an identifier? | Never touch |
| Active fork | Does the prose name which fork is live? | Rewrite to Unzen |
| Machinery | Does it describe code that spans forks? | Drop the fork word |

The machinery class exists because `crates/pipeline/src/forks/shasta/spec.rs` branches on both
`TaikoFork::Shasta` (`:1305`) and `TaikoFork::Unzen` (`:1848`), and `docs/development.md:485`
documents a live `shasta_unzen_transition` regression suite carrying pre-fork control proposals.
Prose calling that code "the Shasta pipeline" is wrong today; calling it "the Unzen pipeline" would
be wrong in the opposite direction. Naming it for no fork is the only accurate option, and it
survives the next hardfork without another documentation pass.

### Frozen identifiers

These appear in the touched files and keep the `shasta` spelling:

- Routes: `/v3/proof/batch/shasta`, `/prove/shasta`, `/prove/shasta-aggregate`
- Request schemas: `raiko2-shasta-request-v1`, `raiko2-shasta-aggregate-request-v1`
- Rust identifiers: `PipelineKey::ShastaSp1`, `TaikoSpecId::SHASTA`
- Crates: `raiko2-primitives-shasta`, `raiko2-protocol-shasta`
- GCS proof URI path component: `shasta-sp1-local`
- Guest artifacts: `risc0_shasta_*.elf`, `sp1_shasta_*.elf`, `sp1_shasta_*.vk.bin`
- xtask profiles: `hoodi-shasta`, `mainnet-shasta`
- Fixtures and directories: `shasta_guest_input_taiko_mainnet_proposal_23077_*.json`,
  `test/guest_inputs/shasta/`, `test/regression/shasta/`
- Scripts and config: `shasta_regression.py`, `stress_shasta_proposal.py`,
  `config/shasta_regression_devnet.json`
- Log strings: `registered shasta proof task`, `completed shasta proof task`,
  `shasta tx-list witnesses ready`
- Regression suite: `shasta_unzen_transition`, and the `SHASTA -> UNZEN` boundary description

`PipelineKey` variant names are serialized into stored `ProofArtifactRecord` JSON, and
`shasta-sp1-local` is a live GCS path component. These are two separate wire surfaces on one enum.
Neither is in scope here, but documentation must not imply either is renameable.

### Representative rewrites

```
README.md:9      - Raiko2 is a Shasta proof service for Taiko.
                 + Raiko2 is a proposal proof service for Taiko. The active proposal fork is Unzen.

README.md:19     - Shasta-first pipeline for preflight, validation, proving, and aggregation
                 + Proposal pipeline for preflight, validation, proving, and aggregation

architecture:68    Pipeline["Shasta pipeline"] -> Pipeline["Proposal pipeline"]

API.md:696       - not the Taiko Shasta verifier registered in the chain spec
                 + not the Taiko proposal verifier registered in the chain spec
```

`CONTEXT.md:19` is amended rather than substituted. It defines "proposal fork" and uses a fork name
as an illustration, so it becomes `such as Shasta or Unzen`, which teaches the distinction instead
of erasing it.

## File Scope

In scope (11 files plus two banner assets):

| File | Mentions | Nature of change |
| --- | --- | --- |
| `README.md` | 14 | Headline, feature list, pipeline prose, banner alt text |
| `docs/README.md` | 1 | Precompile pointer; add archive note |
| `docs/architecture.md` | 4 | Service description, Mermaid node label |
| `docs/API.md` | 28 | Prose only; all routes and URIs frozen |
| `docs/development.md` | 25 | Prose only; profiles, fixtures, scripts frozen |
| `docs/operations.md` | 27 | Prose only; ELF globs and profiles frozen |
| `docs/precompile-status.md` | 12 | Full rewrite (below) |
| `docs/gaiko2-remote-prover-integration.md` | 15 | Prose only; schemas and routes frozen |
| `scripts/regression/README.md` | 25 | Title and prose; ~20 filename mentions frozen |
| `tests/fixtures/README.md` | 7 | Title and one prose line; rest frozen |
| `CONTEXT.md` | 2 | Vocabulary amendment |
| `docs/assets/readme-banner.svg` / `.png` | — | Rewrite and re-render |

Out of scope:

- `docs/plans/**` (99 files, 43 mentioning Shasta). Dated design records. "This is what we decided
  in April 2026" is true with Shasta in it; rewriting makes them misrepresent their own moment.
- `docs/hoodi-txlist-witness-rollout.md`. A completed rollout runbook pinned to "image built from
  #70 or a later commit"; the repository is at #239.
- `experiments/opcode-gas/README.md`. A lab record.
- `CONTRIBUTING.md`, `AGENTS.md`. Their only mentions are crate names, which are frozen.

`docs/README.md` gains one sentence stating that `docs/plans/` is a historical archive whose fork
names are accurate as of each document's date. This resolves the staleness question for all 99 plan
files at once rather than per-file.

## Precompile Status Rewrite

`docs/precompile-status.md` is currently wrong for every live network, not merely stale. It is
built on the Shasta mapping and explicitly asserts that `0x0A`, `0x0B..0x11`, and `0x0100` are
inactive. Under Unzen all three groups are active.

Source of truth is `revm-precompile 34.0.0`, the version both guests pin
(`guests/sp1/Cargo.toml:39`, `guests/risc0/Cargo.toml:29`). The workspace lock also contains
`41.0.0`, which is not what the guests compile against; the document must say which version it
describes.

Mapping, verified in `crates/primitives/src/chain_spec.rs:376-377`:

- `TaikoFork::Unzen` maps to `SpecId::OSAKA`
- all other Taiko forks fall through to `SpecId::SHANGHAI`

In `revm-precompile 34.0.0`, `SHANGHAI` collapses to `PrecompileSpecId::BERLIN` while `OSAKA` maps
to `PrecompileSpecId::OSAKA`, defined as `prague() + modexp::OSAKA + secp256r1::P256VERIFY_OSAKA`.
The full chain is homestead, byzantium, istanbul, berlin, cancun, prague, osaka.

Unzen therefore activates nine addresses Shasta never had: `0x0A`, `0x0B` through `0x11`, and
`0x100`.

The replacement table pairs each active address with its guest acceleration hook. The `Crypto`
trait exposes 14 overridable methods; RISC0 overrides 6 and SP1 overrides 4.

| Address | Precompile | RISC0 hook | SP1 hook |
| --- | --- | --- | --- |
| `0x01` | `ECRECOVER` | Yes | Yes |
| `0x02` | `SHA256` | Yes | Yes |
| `0x03` | `RIPEMD160` | No | No |
| `0x04` | `IDENTITY` | No hook exists | No hook exists |
| `0x05` | `MODEXP` (Osaka repricing, EIP-7823/7883) | Yes | No |
| `0x06` | `BN254_ADD` | Yes | Yes |
| `0x07` | `BN254_MUL` | Yes | Yes |
| `0x08` | `BN254_PAIRING` | No | No |
| `0x09` | `BLAKE2F` | No | No |
| `0x0A` | `KZG_POINT_EVALUATION` | No | No |
| `0x0B` | `BLS12_381_G1_ADD` | No | No |
| `0x0C` | `BLS12_381_G1_MSM` | No | No |
| `0x0D` | `BLS12_381_G2_ADD` | No | No |
| `0x0E` | `BLS12_381_G2_MSM` | No | No |
| `0x0F` | `BLS12_381_PAIRING` | No | No |
| `0x10` | `BLS12_381_MAP_FP_TO_G1` | No | No |
| `0x11` | `BLS12_381_MAP_FP2_TO_G2` | No | No |
| `0x100` | `P256VERIFY` | Yes | No |

Two findings the document states plainly:

1. The eight newly-active precompiles at `0x0A` and `0x0B`-`0x11` have no guest hook in either
   backend.
2. RISC0 and SP1 diverge. RISC0 overrides `modexp` and `secp256r1_verify_signature`; SP1 overrides
   neither, so `0x05` and the newly-active `0x100` run unaccelerated under SP1.

Backend note: guests build `revm-precompile` with `default-features = false, features = ["bn"]`,
selecting neither `blst` nor `c-kzg`. BLS12-381 and KZG therefore fall back to pure-Rust arkworks
inside the zkVM.

Stated limitation: the table describes the *active* precompile set. Whether Taiko execution reaches
`0x0A` or `0x0B`-`0x11` on real proposals is a separate question that this pass does not trace. The
document says so rather than implying these are hot paths.

The existing note that blob proof validation does not depend on the EVM `0x0A` precompile is
retained, and updated to reflect that `0x0A` is now active but still unused by that path.

## Banner

`docs/assets/readme-banner.svg` changes:

- Delete the `HOODI-COMPATIBLE` pill (rect and text at `y=374`).
- Delete the `SHASTA-FIRST` pill (rect and text at `y=374`).
- Subtitle becomes `Taiko proof orchestration`, with no fork name. This is a deliberate exception
  to the naming rule: the rule sends the active fork name into prose, but a banner is the one
  surface that cannot be corrected in a hurry, so it carries no fork name at all and cannot go
  stale at the next hardfork.
- Close the gap the pills leave by moving both remaining elements up 62px: the divider from
  `y=454` to `y=392`, and the `PREFLIGHT / VALIDATE / PROVE / AGGREGATE` rule from `y=488` to
  `y=426`. This leaves a 72px gap below the subtitle baseline at `y=320`. Confirm the spacing
  visually in the rendered PNG and adjust if it reads loose.

`README.md:1` alt text becomes `Raiko2 — Taiko proof orchestration`, matching the image.

Re-render to `readme-banner.png` at 1800x600. This machine has no system rasterizer and PEP 668
blocks installing into system Python, so use a throwaway virtualenv:

```
python3 -m venv <tmp>/svgvenv
<tmp>/svgvenv/bin/pip install cairosvg
DYLD_FALLBACK_LIBRARY_PATH=/opt/homebrew/lib <tmp>/svgvenv/bin/python -c \
  "import cairosvg; cairosvg.svg2png(url='docs/assets/readme-banner.svg', \
   write_to='docs/assets/readme-banner.png', output_width=1800, output_height=600)"
```

libcairo is present via Homebrew. The SVG requests Adwaita Sans, which is absent on this machine,
so glyph metrics drift slightly from the original render. Removing both pills removes the element
that was previously clipped by that drift.

## Verification

Per the `AGENTS.md` verification policy, docs-only changes need no Rust checks unless commands or
paths changed. This change alters no commands or paths.

1. Run `grep -n 'Shasta\|shasta\|SHASTA'` over each touched file.
2. Check every surviving hit against the frozen-identifier list above. The success criterion is
   that each remaining mention is deliberate, not that the count reaches zero.
3. Confirm no file outside the scope table is modified: `git diff --name-only`.
4. Confirm the rewritten precompile table against `revm-precompile 34.0.0` sources and the two
   guest `crypto.rs` files.
5. Inspect the re-rendered PNG: both pills gone, no fork name, no clipped text, no gap.

## Follow-ups (out of scope, code changes)

1. **Hoodi verifier misregistration.** `xtask/src/register_image.rs:417` hardcodes
   `/verifier_address_forks/SHASTA/{proof_type}`. In `config/chain_spec_list_default.json`,
   `taiko_hoodi` SHASTA and UNZEN entries differ:

   | Backend | SHASTA | UNZEN |
   | --- | --- | --- |
   | SP1 | `0xc42Ef1A7A606162e144F696A07A7D3Ad98bF4EE7` | `0x2a872461C4629D5626Cb6852e50d75Bc7702f0e2` |
   | RISC0 | `0xfa0e7dAFe9785627df034c123A9B87497EB06b41` | `0x8f2007dC3Bf34a1E4A4Ea5303EDC2D8e140934E9` |

   `--profile hoodi-shasta` therefore registers against Shasta verifiers on a network running
   Unzen. `taiko_mainnet` entries are identical, so only hoodi is exposed; `taiko_dev` and
   `taiko_masaya` have no UNZEN entry at all. Because this pass cannot change code, the operations
   and development docs carry a warning note at each `--profile hoodi-shasta` occurrence.

2. **SP1 guest hook gaps.** SP1 lacks `modexp` and `secp256r1_verify_signature` hooks that RISC0
   has, and `0x100` is newly active under Unzen. Worth a cycle-cost measurement.

3. **Unaccelerated OSAKA precompiles.** `0x0A` and `0x0B`-`0x11` run on arkworks inside the zkVM
   with no hooks. Whether they are reachable on real proposals determines whether this matters.
