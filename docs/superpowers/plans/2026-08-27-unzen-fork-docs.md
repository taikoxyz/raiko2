# Unzen Fork Documentation Refresh Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Retire "Shasta" as the current-fork shorthand across raiko2 documentation and the README banner, without touching code.

**Architecture:** Each `Shasta` mention is classified by one three-way test and edited accordingly. Frozen wire strings, on-disk paths, and Rust identifiers are never touched. Prose naming the live fork becomes Unzen. Prose describing fork-spanning machinery drops the fork word. `docs/precompile-status.md` is rewritten wholesale because it is factually wrong for every live network. The banner drops both badge pills and every fork reference.

**Tech Stack:** Markdown, SVG, Python `cairosvg` (throwaway virtualenv) for PNG rendering, `grep` for verification.

**Spec:** `docs/superpowers/specs/2026-08-27-unzen-fork-docs-design.md` (committed as `8c185e8`)

## Global Constraints

- **Branch:** `docs/unzen-fork-naming`. Already created; the spec is committed on it.
- **No code.** Do not modify any `.rs`, `.toml`, `.py`, `.json`, `.yml` file, or any fixture. This
  change touches only `.md` files and `docs/assets/readme-banner.{svg,png}`.
- **Never touch these frozen identifiers,** wherever they appear:
  - Routes: `/v3/proof/batch/shasta`, `/prove/shasta`, `/prove/shasta-aggregate`
  - Schemas: `raiko2-shasta-request-v1`, `raiko2-shasta-aggregate-request-v1`
  - Rust identifiers: `PipelineKey::ShastaSp1`, `ShastaSp1`, `TaikoSpecId::SHASTA`
  - Crates: `raiko2-primitives-shasta`, `raiko2-protocol-shasta`
  - GCS proof URI path component: `shasta-sp1-local`
  - Guest artifacts: `risc0_shasta_*.elf`, `sp1_shasta_*.elf`, `sp1_shasta_*.vk.bin`
  - xtask profiles: `hoodi-shasta`, `mainnet-shasta`
  - Fixtures/directories: `shasta_guest_input_taiko_mainnet_proposal_23077_l2_9051439_9051630.json`,
    `shasta_aggregate_request_v1_single_fixture_proof.json`, `test/guest_inputs/shasta/`,
    `test/regression/shasta/`, `scripts/regression/shasta/`
  - Scripts/config: `shasta_regression.py`, `stress_shasta_proposal.py`,
    `shasta_regression_devnet.json`, `shasta_regression_hoodi.json`, `internal/protocol/shasta_v1.go`
  - Log strings: `registered shasta proof task`, `completed shasta proof task`,
    `shasta tx-list witnesses ready`
  - Regression suite name and boundary: `shasta_unzen_transition`, `SHASTA -> UNZEN`
- **Out of scope, do not edit:** `docs/plans/**`, `docs/hoodi-txlist-witness-rollout.md`,
  `experiments/opcode-gas/README.md`, `CONTRIBUTING.md`, `AGENTS.md`, `config.example.toml`.
- **Fork-neutral vocabulary is "proposal".** `CONTEXT.md` already defines it. Do not invent new
  terms such as "current-fork" or "active-fork" in prose.
- **Verification criterion:** after each task, every surviving `Shasta`/`shasta`/`SHASTA` hit in the
  touched file must be on the frozen list above. The criterion is a deliberate audit, not a zero
  count.
- **Commit style:** Conventional Commits, per `AGENTS.md`. One commit per task.
- **No Rust checks required.** `AGENTS.md` exempts docs-only changes unless commands or paths
  changed. This change alters no commands or paths.

---

### Task 1: Vocabulary and archive note

Establishes the fork-neutral vocabulary the remaining tasks apply, and closes the `docs/plans/`
staleness question for all 99 plan files with one sentence.

**Files:**
- Modify: `CONTEXT.md:10-11`, `CONTEXT.md:19-20`
- Modify: `docs/README.md:24-25`, `docs/README.md` (append to the "How to Use These Docs" list)

**Interfaces:**
- Consumes: nothing.
- Produces: the vocabulary decision every later task follows — machinery prose says "proposal",
  active-fork prose says "Unzen".

- [ ] **Step 1: Record the current state**

Run: `grep -c "Shasta\|shasta\|SHASTA" CONTEXT.md docs/README.md`
Expected output:
```
CONTEXT.md:2
docs/README.md:1
```

- [ ] **Step 2: Amend the `Proposal proof` glossary entry in `CONTEXT.md`**

`batch` wording is legacy v3 naming, not a fork property. Replace lines 10-11.

Before:
```markdown
- **Proposal proof**: A proof for one Taiko proposal. Public prover interfaces should use this term
  instead of Shasta-specific `batch` wording. The covered L2 blocks are contiguous; the HTTP prover
```

After:
```markdown
- **Proposal proof**: A proof for one Taiko proposal. Public prover interfaces should use this term
  instead of the v3 `batch` wording. The covered L2 blocks are contiguous; the HTTP prover
```

- [ ] **Step 3: Amend the `Proposal fork` glossary entry in `CONTEXT.md`**

This entry defines the concept and uses fork names as illustration. Naming both forks teaches the
distinction that the rest of this change depends on. Replace lines 19-20.

Before:
```markdown
- **Proposal fork**: The Taiko proposal rules active for a network and proposal, such as Shasta or a
  future hardfork. Clients should not select proposal forks in route names.
```

After:
```markdown
- **Proposal fork**: The Taiko proposal rules active for a network and proposal, such as Shasta or
  Unzen. Every Taiko network is on Unzen as of 2026-08-06; Shasta remains a real fork that earlier
  proposals were proved under. Clients should not select proposal forks in route names.
```

- [ ] **Step 4: Update the precompile pointer in `docs/README.md`**

Replace lines 24-25.

Before:
```markdown
- Read [precompile-status.md](precompile-status.md) when you need the current Shasta precompile
  activation and guest hook coverage.
```

After:
```markdown
- Read [precompile-status.md](precompile-status.md) when you need the current Unzen precompile
  activation and guest hook coverage.
```

- [ ] **Step 5: Extend the existing `## Historical Notes` section in `docs/README.md`**

Do NOT add a bullet to the "How to Use These Docs" list. `docs/README.md` already has a
`## Historical Notes` section covering both `plans/` and `issues/`, and that is where this belongs.

Do not call `docs/plans/` an archive. `docs/plans/README.md` defines six status values of which only
`Archived` is historical, frames the directory as the home for notes "that should survive handoff",
and the directory holds live drafts. Scope the claim to what is actually true: individual plans are
point-in-time records.

Before:
```markdown
## Historical Notes

The files under [`plans/`](plans) and [`issues/`](issues) are historical design, implementation,
and review notes. They are useful background, but they are not the current source of truth for
using or operating the project.
```

After:
```markdown
## Historical Notes

The files under [`plans/`](plans) and [`issues/`](issues) are historical design, implementation,
and review notes. They are useful background, but they are not the current source of truth for
using or operating the project.

Each plan records a decision as of its own date. Its fork names, file paths, and command names are
not maintained against current behavior, so read them as point-in-time records rather than as
current documentation.
```

- [ ] **Step 6: Verify**

Run: `grep -n "Shasta\|shasta\|SHASTA" CONTEXT.md docs/README.md`
Expected: exactly two hits, both in `CONTEXT.md` — line 19 ending `such as Shasta or` and line 20
containing `Unzen. Every Taiko network is on Unzen as of 2026-08-06; Shasta remains a real fork`.
Both are the deliberate glossary text from Step 3. `docs/README.md` returns nothing.

- [ ] **Step 7: Commit**

```bash
git add CONTEXT.md docs/README.md
git commit -m "docs: make fork vocabulary explicit and mark plans as archive"
```

---

### Task 2: Banner SVG and PNG

**Files:**
- Modify: `docs/assets/readme-banner.svg`
- Regenerate: `docs/assets/readme-banner.png`

**Interfaces:**
- Consumes: nothing.
- Produces: a banner with no fork reference. Task 3 writes README alt text that must match the new
  subtitle exactly: `Raiko2 — Taiko proof orchestration`.

- [ ] **Step 1: Confirm the current pills exist**

Run: `grep -n "HOODI-COMPATIBLE\|SHASTA-FIRST\|orchestration for Shasta" docs/assets/readme-banner.svg`
Expected: three hits, at the subtitle text, the `HOODI-COMPATIBLE` text, and the `SHASTA-FIRST` text.

- [ ] **Step 2: Edit the SVG**

Replace this entire block (subtitle, both pills, divider, and rule):

```xml
  <text x="114" y="320" fill="#A9C0D4" font-family="Adwaita Sans, Liberation Sans, sans-serif" font-size="31" font-weight="400" letter-spacing="0.4">Taiko proof orchestration for Shasta</text>

  <rect x="114" y="374" width="186" height="34" rx="17" fill="#12283B" stroke="#56D6FF" stroke-opacity="0.34"/>
  <text x="144" y="397" fill="#D8EEF8" font-family="Adwaita Sans, Liberation Sans, sans-serif" font-size="18" font-weight="600" letter-spacing="1.8">HOODI-COMPATIBLE</text>

  <rect x="322" y="374" width="145" height="34" rx="17" fill="#12283B" stroke="#7AF0C2" stroke-opacity="0.30"/>
  <text x="352" y="397" fill="#DCF7EC" font-family="Adwaita Sans, Liberation Sans, sans-serif" font-size="18" font-weight="600" letter-spacing="1.6">SHASTA-FIRST</text>

  <rect x="114" y="454" width="462" height="1" fill="#35556D" fill-opacity="0.7"/>
  <text x="114" y="488" fill="#7F99AE" font-family="Adwaita Sans, Liberation Sans, sans-serif" font-size="17" font-weight="400" letter-spacing="1.3">PREFLIGHT  •  VALIDATE  •  PROVE  •  AGGREGATE</text>
```

with:

```xml
  <text x="114" y="320" fill="#A9C0D4" font-family="Adwaita Sans, Liberation Sans, sans-serif" font-size="31" font-weight="400" letter-spacing="0.4">Taiko proof orchestration</text>

  <rect x="114" y="392" width="462" height="1" fill="#35556D" fill-opacity="0.7"/>
  <text x="114" y="426" fill="#7F99AE" font-family="Adwaita Sans, Liberation Sans, sans-serif" font-size="17" font-weight="400" letter-spacing="1.3">PREFLIGHT  •  VALIDATE  •  PROVE  •  AGGREGATE</text>
```

Both pills are deleted. The divider moves `y=454` to `y=392` and the rule `y=488` to `y=426`, a
uniform 62px lift that leaves a 72px gap below the subtitle baseline at `y=320`.

- [ ] **Step 3: Verify the SVG has no fork reference**

Run: `grep -c "Shasta\|shasta\|SHASTA\|HOODI" docs/assets/readme-banner.svg`
Expected: `0`

- [ ] **Step 4: Build the render environment**

This machine has no system rasterizer, and PEP 668 blocks installing into system Python. Use a
throwaway virtualenv in the scratchpad, not the repository.

```bash
python3 -m venv /private/tmp/claude-501/-Users-davidcai-taiko-raiko2/88459047-b4a2-4c0b-b6e7-6f27edce604d/scratchpad/svgvenv
/private/tmp/claude-501/-Users-davidcai-taiko-raiko2/88459047-b4a2-4c0b-b6e7-6f27edce604d/scratchpad/svgvenv/bin/pip install cairosvg
```

Expected: `Successfully installed cairosvg-...`. libcairo is already present via Homebrew.

- [ ] **Step 5: Render the PNG**

```bash
DYLD_FALLBACK_LIBRARY_PATH=/opt/homebrew/lib /private/tmp/claude-501/-Users-davidcai-taiko-raiko2/88459047-b4a2-4c0b-b6e7-6f27edce604d/scratchpad/svgvenv/bin/python -c \
  "import cairosvg; cairosvg.svg2png(url='docs/assets/readme-banner.svg', \
   write_to='docs/assets/readme-banner.png', output_width=1800, output_height=600)"
```

Expected: no output, exit code 0.

- [ ] **Step 6: Verify the PNG**

```bash
file docs/assets/readme-banner.png
```
Expected: `PNG image data, 1800 x 600, 8-bit/color RGBA, non-interlaced`

Then open `docs/assets/readme-banner.png` and confirm visually:
- "Raiko2" wordmark and the subtitle "Taiko proof orchestration" are present.
- No badge pills anywhere.
- No fork name anywhere.
- No clipped or overlapping text.
- The gap between the subtitle and the divider does not read as loose. If it does, lower the lift
  from 62px to about 50px (divider `y=404`, rule `y=438`) and re-render.

The SVG requests Adwaita Sans, which is absent here, so glyph metrics drift slightly from the
original render. This is expected and affects only kerning.

- [ ] **Step 7: Commit**

```bash
git add docs/assets/readme-banner.svg docs/assets/readme-banner.png
git commit -m "docs: drop badge pills and fork name from README banner"
```

---

### Task 3: README.md

**Files:**
- Modify: `README.md` lines 1, 9, 19, 160, 182, 205, 230, 236

**Interfaces:**
- Consumes: the banner subtitle from Task 2. Alt text must match it.
- Produces: the canonical one-line description of the service that `docs/architecture.md` echoes in
  Task 4.

- [ ] **Step 1: Record the current state**

Run: `grep -c "Shasta\|shasta\|SHASTA" README.md`
Expected: `14`

- [ ] **Step 2: Update the banner alt text (line 1)**

Before:
```markdown
![Raiko2 — Taiko proof orchestration for Shasta](docs/assets/readme-banner.png)
```

After:
```markdown
![Raiko2 — Taiko proof orchestration](docs/assets/readme-banner.png)
```

- [ ] **Step 3: Update the headline (lines 9-11)**

This is the sentence that prompted this change.

Before:
```markdown
Raiko2 is a Shasta proof service for Taiko. It builds canonical guest inputs from RPC data,
validates them, runs local or remote proving routes, and exposes a typed v4 API for
asynchronous proposal-side proof requests.
```

After:
```markdown
Raiko2 is a proposal proof service for Taiko. It builds canonical guest inputs from RPC data,
validates them, runs local or remote proving routes, and exposes a typed v4 API for
asynchronous proposal-side proof requests. Every Taiko network runs the Unzen fork.
```

- [ ] **Step 4: Update the At a Glance bullet (line 19)**

Before:
```markdown
- Shasta-first pipeline for preflight, validation, proving, and aggregation
```

After:
```markdown
- Proposal pipeline for preflight, validation, proving, and aggregation
```

- [ ] **Step 5: Update the Core Flow step (line 160)**

Before:
```markdown
1. `Preflight` resolves canonical Shasta inputs from L1 and L2 RPC.
```

After:
```markdown
1. `Preflight` resolves canonical proposal inputs from L1 and L2 RPC.
```

- [ ] **Step 6: Update the blob proof type note (lines 182-183)**

Before:
```markdown
- Shasta manifests support `blob_proof_type = "proof_of_equivalence"` only; legacy
  `kzg_versioned_hash` manifests are rejected.
```

After:
```markdown
- Proposal manifests support `blob_proof_type = "proof_of_equivalence"` only; legacy
  `kzg_versioned_hash` manifests are rejected.
```

- [ ] **Step 7: Update the SGX route description (line 205)**

Before:
```markdown
- `sgx/remote` submits Shasta proving to the dedicated remote SGX runtime. This repo now ships
```

After:
```markdown
- `sgx/remote` submits proposal proving to the dedicated remote SGX runtime. This repo now ships
```

- [ ] **Step 8: Update the conformance harness prose (line 230)**

The fixture file name stays frozen; only the adjective goes.

Before:
```markdown
The harness builds the proposal request from the shared Shasta `GuestInput` fixture and posts it to:
```

After:
```markdown
The harness builds the proposal request from the shared `GuestInput` fixture and posts it to:
```

- [ ] **Step 9: Update the SGX prover validation prose (lines 234-236)**

The two schema names and the route on these lines are frozen. Only the trailing prose changes.

Before:
```markdown
This harness targets providers whose `/prove/shasta` input is the v1
`raiko2-shasta-request-v1` packet with `payload.guest_input`. `raiko2-sgx-prover` consumes the
same request shape and runs the Shasta guest validation path before signing.
```

After:
```markdown
This harness targets providers whose `/prove/shasta` input is the v1
`raiko2-shasta-request-v1` packet with `payload.guest_input`. `raiko2-sgx-prover` consumes the
same request shape and runs the guest validation path before signing.
```

- [ ] **Step 10: Verify**

Run: `grep -n "Shasta\|shasta\|SHASTA" README.md`
Expected: exactly 6 hits, all frozen — lines carrying
`shasta_aggregate_request_v1_single_fixture_proof.json`, `raiko2-shasta-aggregate-request-v1`,
`POST /prove/shasta`, `/prove/shasta` (twice, in the harness paragraph),
`raiko2-shasta-request-v1`, and `POST /prove/shasta-aggregate`.
Confirm each against the Global Constraints frozen list. Zero prose mentions remain.

- [ ] **Step 11: Commit**

```bash
git add README.md
git commit -m "docs: retire Shasta as current-fork shorthand in README"
```

---

### Task 4: docs/architecture.md

**Files:**
- Modify: `docs/architecture.md:3`, `docs/architecture.md:68`

**Interfaces:**
- Consumes: the service description wording from Task 3.
- Produces: nothing later tasks depend on.

- [ ] **Step 1: Record the current state**

Run: `grep -c "Shasta\|shasta\|SHASTA" docs/architecture.md`
Expected: `4`

- [ ] **Step 2: Update the service description (lines 3-5)**

Before:
```markdown
Raiko2 is a Shasta proof orchestration service. It turns a normalized v4 proof request into a
durable proposal or aggregation proof while keeping request identity, execution state, remote
provider checkpoints, and published proof artifacts recoverable across process restarts.
```

After:
```markdown
Raiko2 is a proposal proof orchestration service. It turns a normalized v4 proof request into a
durable proposal or aggregation proof while keeping request identity, execution state, remote
provider checkpoints, and published proof artifacts recoverable across process restarts.
```

- [ ] **Step 3: Update the Mermaid pipeline node (line 68)**

Before:
```
  Engine --> Pipeline["Shasta pipeline"]
```

After:
```
  Engine --> Pipeline["Proposal pipeline"]
```

- [ ] **Step 4: Verify**

Run: `grep -n "Shasta\|shasta\|SHASTA" docs/architecture.md`
Expected: exactly 2 hits, lines 181 and 193, both `ShastaSp1`. Both are frozen Rust identifiers.

Then confirm the Mermaid block still parses by checking the node line is well-formed:
Run: `grep -n 'Pipeline\["Proposal pipeline"\]' docs/architecture.md`
Expected: one hit at line 68.

- [ ] **Step 5: Commit**

```bash
git add docs/architecture.md
git commit -m "docs: retire Shasta as current-fork shorthand in architecture"
```

---

### Task 5: docs/API.md

Thirteen prose edits. Every route, proof URI, and `ShastaSp1` reference stays.

**Files:**
- Modify: `docs/API.md` lines 5, 324, 605, 653, 655, 656, 696, 1180, 1183, 1223, 1345, 1353, 1359

**Interfaces:**
- Consumes: fork-neutral vocabulary from Task 1.
- Produces: nothing later tasks depend on.

- [ ] **Step 1: Record the current state**

Run: `grep -c "Shasta\|shasta\|SHASTA" docs/API.md`
Expected: `28`

- [ ] **Step 2: Update the API summary (line 5)**

Before:
```markdown
Raiko2 exposes a Shasta-first `/v4` API for explicit proof-type proposal proving,
```

After:
```markdown
Raiko2 exposes a proposal-first `/v4` API for explicit proof-type proposal proving,
```

- [ ] **Step 3: Update the v4 proposal id constraint (line 324)**

The `uint48` width is an inbox protocol field that Unzen inherits, so name the protocol, not a fork.

Before:
```markdown
- `proposals[].proposal_id` must fit Shasta's `uint48` protocol field.
```

After:
```markdown
- `proposals[].proposal_id` must fit the inbox protocol's `uint48` field.
```

- [ ] **Step 4: Update the v3 batch root description (line 605)**

Before:
```markdown
Registers a Shasta batch root task. The server expands it into proposal prove tasks and, when
```

After:
```markdown
Registers a legacy batch root task. The server expands it into proposal prove tasks and, when
```

- [ ] **Step 5: Update the v3 proposal derivation notes (lines 653-656)**

Three consecutive edits in one block.

Before:
```markdown
- `proposal.l1_inclusion_block_number` is required. The server derives canonical Shasta proposal
```

After:
```markdown
- `proposal.l1_inclusion_block_number` is required. The server derives canonical proposal
```

Before:
```markdown
- `proposal.proposal_id` must fit Shasta's `uint48` protocol field.
- `proposal.last_anchor_block_number` participates in Shasta anchor monotonicity validation.
```

After:
```markdown
- `proposal.proposal_id` must fit the inbox protocol's `uint48` field.
- `proposal.last_anchor_block_number` participates in anchor monotonicity validation.
```

- [ ] **Step 6: Update both verifier references (lines 696 and 1359)**

These matter beyond naming. `config/chain_spec_list_default.json` gives `taiko_hoodi` different SP1
and RISC0 verifier addresses under `SHASTA` and `UNZEN`, so "Shasta verifier" now points at the
wrong contract on hoodi. Fork-neutral phrasing is the only correct wording.

Before (line 696):
```markdown
  not the Taiko Shasta verifier registered in the chain spec.
```

After:
```markdown
  not the Taiko proposal verifier registered in the chain spec.
```

Before (line 1359):
```markdown
  proving. This verifier is separate from the Taiko Shasta verifier address in the chain spec.
```

After:
```markdown
  proving. This verifier is separate from the Taiko proposal verifier address in the chain spec.
```

- [ ] **Step 7: Update the preflight cache notes (lines 1180 and 1183)**

Before:
```markdown
- Canonical Shasta preflight cores are keyed by chain/range/proposal/L1 inclusion and effective rule
```

After:
```markdown
- Canonical preflight cores are keyed by chain/range/proposal/L1 inclusion and effective rule
```

Before:
```markdown
  revalidated with guest-equivalent Shasta semantics before lane-specific `GuestInput` materialization.
```

After:
```markdown
  revalidated with guest-equivalent semantics before lane-specific `GuestInput` materialization.
```

- [ ] **Step 8: Update the beacon RPC note (line 1223)**

Before:
```markdown
- `rpc.pairs[*].beacon_rpc` is optional. When set, Shasta blob sidecar fetches use that L1
```

After:
```markdown
- `rpc.pairs[*].beacon_rpc` is optional. When set, blob sidecar fetches use that L1
```

- [ ] **Step 9: Update the preflight tuning notes (lines 1345 and 1353)**

Before:
```markdown
- Shasta preflight splits proposals into chunks of `8` blocks by default and runs at most `6`
```

After:
```markdown
- Preflight splits proposals into chunks of `8` blocks by default and runs at most `6`
```

Before:
```markdown
- Shasta preflight retries retryable provider/RPC/IO failures inside the preflight stage with
```

After:
```markdown
- Preflight retries retryable provider/RPC/IO failures inside the preflight stage with
```

- [ ] **Step 10: Verify**

Run: `grep -n "Shasta\|shasta\|SHASTA" docs/API.md`
Expected: exactly 15 hits. Audit each against the frozen list. They should be only:
- proof URI examples containing `shasta-sp1-local` (lines near 389, 395, 1028, 1036, 1040)
- `PipelineKey::ShastaSp1` / `ShastaSp1` (lines near 425, 1206, 1209)
- `/v3/proof/batch/shasta` route mentions (lines near 500, 572, 601, 668, 706, 850)
- the heading `## Legacy V3 Submit Shasta Batch Proof` (line 596)

The heading is retained deliberately: it names the frozen `/v3/proof/batch/shasta` route, and its
anchor `#legacy-v3-submit-shasta-batch-proof` is a stable link target.

- [ ] **Step 11: Commit**

```bash
git add docs/API.md
git commit -m "docs: retire Shasta as current-fork shorthand in API reference"
```

---

### Task 6: docs/development.md and docs/operations.md

These two change together because both document `--profile hoodi-shasta` and must carry the same
warning about it.

**Files:**
- Modify: `docs/development.md` lines 108, 311, 332, 429, 443, 492, plus a warning block inserted after line 294
- Modify: `docs/operations.md` lines 204, 529, 696, 740, 1521, 1528, plus a warning block inserted after line 934

**Interfaces:**
- Consumes: fork-neutral vocabulary from Task 1.
- Produces: nothing later tasks depend on.

- [ ] **Step 1: Record the current state**

Run: `grep -c "Shasta\|shasta\|SHASTA" docs/development.md docs/operations.md`
Expected:
```
docs/development.md:25
docs/operations.md:27
```

- [ ] **Step 2: Update `docs/development.md` prose (lines 108, 311, 332, 429, 443, 492)**

Six independent single-line edits. The `/v3/proof/batch/shasta` route on line 109, the
`stress_shasta_proposal.py` script name on line 333, the `test/guest_inputs/shasta/` path on line
443, and `shasta_regression.py` on line 493 all stay.

Line 108, before:
```markdown
Use the legacy `xtask` helper to discover the latest onchain Shasta proposal and emit a
```
After:
```markdown
Use the legacy `xtask` helper to discover the latest onchain proposal and emit a
```

Line 311, before:
```markdown
generated from a real Shasta preflight.
```
After:
```markdown
generated from a real preflight run.
```

Line 332, before:
```markdown
For live Shasta proposals, first use
```
After:
```markdown
For live proposals, first use
```

Line 429, before:
```markdown
through fixed deterministic inputs. These deliberately remove Shasta proposal/block noise and avoid
```
After:
```markdown
through fixed deterministic inputs. These deliberately remove proposal/block noise and avoid
```

Line 443, before:
```markdown
Checked-in Shasta GuestInput fixtures live under `test/guest_inputs/shasta/<network>/`.
```
After:
```markdown
Checked-in GuestInput fixtures live under `test/guest_inputs/shasta/<network>/`.
```

Line 492, before:
```markdown
The file-based Shasta regression flow lives in
```
After:
```markdown
The file-based regression flow lives in
```

- [ ] **Step 3: Add the hoodi verifier warning to `docs/development.md`**

Insert immediately after the closing fence of the `register-image` code block that ends at line 294
(the block containing the four `--profile hoodi-shasta` / `--profile mainnet-shasta` commands).

```markdown
> **Warning — hoodi profiles resolve the Shasta verifier.** `xtask` reads
> `/verifier_address_forks/SHASTA/<proof_type>` from the chain spec. In
> `config/chain_spec_list_default.json`, `taiko_hoodi` carries different SP1 and RISC0 verifier
> addresses under `SHASTA` and `UNZEN`, so `--profile hoodi-shasta` registers against the Shasta
> verifiers on a network that has run Unzen since 2026-06-18. Verify the resolved address against
> the `UNZEN` entry before using `--apply` on hoodi. `taiko_mainnet` entries are identical under
> both forks and are unaffected.
```

- [ ] **Step 4: Update `docs/operations.md` prose (lines 204, 529, 696, 740, 1521, 1528)**

Six independent single-line edits. The ELF globs on lines 496-498, 516-518, 530-532, 710, and 717,
the routes on lines 140-141 and 250, the log strings on line 1273, and the v3 route on line 1459
all stay.

Line 204, before:
```markdown
same Shasta guest validation path as the zk guests before signing the resulting public input.
```
After:
```markdown
same guest validation path as the zk guests before signing the resulting public input.
```

Line 529, before:
```markdown
- Shasta guest artifact assets:
```
After:
```markdown
- Guest artifact assets:
```

Line 696, before:
```markdown
Derive the expected Shasta asset names from the release tag, require the GitHub Release to publish
```
After:
```markdown
Derive the expected guest asset names from the release tag, require the GitHub Release to publish
```

Line 740, before:
```markdown
and the Shasta SP1 ELF/VK pairs without comparing source fingerprints to the current host. This
```
After:
```markdown
and the SP1 ELF/VK pairs without comparing source fingerprints to the current host. This
```

Line 1521, before:
```markdown
Shasta verifier address used for proof registration and chain-spec data carried in proofs.
```
After:
```markdown
Proposal verifier address used for proof registration and chain-spec data carried in proofs.
```

Line 1528, before:
```markdown
Shasta preflight defaults are aligned with the old raiko hosted deployment shape:
```
After:
```markdown
Preflight defaults are aligned with the old raiko hosted deployment shape:
```

- [ ] **Step 5: Add the hoodi verifier warning to `docs/operations.md`**

Insert immediately after the closing fence of the `register-image` code block that ends at line 934.
Use the identical warning text from Step 3 so the two documents do not drift.

```markdown
> **Warning — hoodi profiles resolve the Shasta verifier.** `xtask` reads
> `/verifier_address_forks/SHASTA/<proof_type>` from the chain spec. In
> `config/chain_spec_list_default.json`, `taiko_hoodi` carries different SP1 and RISC0 verifier
> addresses under `SHASTA` and `UNZEN`, so `--profile hoodi-shasta` registers against the Shasta
> verifiers on a network that has run Unzen since 2026-06-18. Verify the resolved address against
> the `UNZEN` entry before using `--apply` on hoodi. `taiko_mainnet` entries are identical under
> both forks and are unaffected.
```

- [ ] **Step 6: Verify**

Run: `grep -n "Shasta\|shasta\|SHASTA" docs/development.md docs/operations.md`

Audit every hit against the frozen list. Surviving hits must be only: xtask profile names, ELF and
VK globs, fixture and directory paths, script names, v3 and `/prove` routes, crate names, log
strings, the `shasta_unzen_transition` suite and its `SHASTA -> UNZEN` boundary line, and the two
new warning blocks (which deliberately name both `SHASTA` and `UNZEN`).

Confirm no prose adjective form survives:
Run: `grep -n "Shasta preflight\|Shasta guest\|Shasta proposal\|Shasta asset\|Shasta SP1\|Shasta verifier\|Shasta regression\|Shasta GuestInput" docs/development.md docs/operations.md`
Expected: no output.

- [ ] **Step 7: Commit**

```bash
git add docs/development.md docs/operations.md
git commit -m "docs: retire Shasta shorthand and flag hoodi verifier fork mismatch"
```

---

### Task 7: docs/gaiko2-remote-prover-integration.md

**Files:**
- Modify: `docs/gaiko2-remote-prover-integration.md` lines 34, 61, 75

**Interfaces:**
- Consumes: fork-neutral vocabulary from Task 1.
- Produces: nothing later tasks depend on.

- [ ] **Step 1: Record the current state**

Run: `grep -c "Shasta\|shasta\|SHASTA" docs/gaiko2-remote-prover-integration.md`
Expected: `15`

- [ ] **Step 2: Update the three prose lines**

Everything else in this file is a frozen route, schema name, Go file path, or fixture name.

Line 34, before:
```markdown
The active proposal request sent by `raiko2` is v1 and carries a Shasta `GuestInput`-shaped payload
```
After:
```markdown
The active proposal request sent by `raiko2` is v1 and carries a `GuestInput`-shaped payload
```

Line 61, before:
```markdown
Proposal conformance is generated from the shared Shasta `GuestInput` fixture at test runtime.
```
After:
```markdown
Proposal conformance is generated from the shared `GuestInput` fixture at test runtime.
```

Line 75, before:
```markdown
The harness builds a v1 proposal request from the shared Shasta `GuestInput` fixture and posts it to:
```
After:
```markdown
The harness builds a v1 proposal request from the shared `GuestInput` fixture and posts it to:
```

- [ ] **Step 3: Verify**

Run: `grep -n "Shasta\|shasta\|SHASTA" docs/gaiko2-remote-prover-integration.md`
Expected: exactly 12 hits, all frozen — `/prove/shasta`, `/prove/shasta-aggregate`,
`raiko2-shasta-request-v1`, `raiko2-shasta-aggregate-request-v1`, `internal/protocol/shasta_v1.go`,
`internal/protocol/shasta_v1_test.go`, the two `testdata/`/`tests/fixtures/` fixture names.

- [ ] **Step 4: Commit**

```bash
git add docs/gaiko2-remote-prover-integration.md
git commit -m "docs: retire Shasta shorthand in gaiko2 integration guide"
```

---

### Task 8: scripts/regression/README.md and tests/fixtures/README.md

Both are tool and fixture READMEs whose mentions are overwhelmingly filenames. Only titles and a
handful of prose lines change, so both files still read heavily "shasta" afterwards. That is
correct: the scripts really are named `shasta_regression.py` and this change does not rename files.

**Files:**
- Modify: `scripts/regression/README.md` lines 1, 33, 42, 46, 57, 69, 171
- Modify: `tests/fixtures/README.md` lines 1, 3

**Interfaces:**
- Consumes: fork-neutral vocabulary from Task 1.
- Produces: nothing later tasks depend on.

- [ ] **Step 1: Record the current state**

Run: `grep -c "Shasta\|shasta\|SHASTA" scripts/regression/README.md tests/fixtures/README.md`
Expected:
```
scripts/regression/README.md:25
tests/fixtures/README.md:7
```

- [ ] **Step 2: Update `scripts/regression/README.md`**

Seven single-line edits. Every `shasta_regression.py`, `stress_shasta_proposal.py`,
`config/shasta_regression_*.json`, and `scripts/regression/shasta/` reference stays untouched.

Line 1, before:
```markdown
# Shasta Regression Tool
```
After:
```markdown
# Proposal Regression Tool
```

Line 33, before:
```markdown
- Current `preflight` requires the full Shasta proposal tuple: proposal id, L1 inclusion block,
```
After:
```markdown
- Current `preflight` requires the full proposal tuple: proposal id, L1 inclusion block,
```

Line 42, before:
```markdown
full Shasta proposal metadata from L1/L2, submits HTTP proof requests to `--raiko-rpc`, and is the
```
After:
```markdown
full proposal metadata from L1/L2, submits HTTP proof requests to `--raiko-rpc`, and is the
```

Line 46, before:
```markdown
`GuestInput` and then runs `guest-launcher` without a `raiko2` server. For current Shasta preflight,
```
After:
```markdown
`GuestInput` and then runs `guest-launcher` without a `raiko2` server. For current preflight,
```

Line 57, before:
```markdown
For a single L2 block, use the stress discovery helper to resolve the containing Shasta proposal
```
After:
```markdown
For a single L2 block, use the stress discovery helper to resolve the containing proposal
```

Line 69, before:
```markdown
The stress helper derives the default L1 RPC, L2 RPC, and Shasta inbox contract from
```
After:
```markdown
The stress helper derives the default L1 RPC, L2 RPC, and inbox contract from
```

Line 171, before:
```markdown
Shasta request envelope that includes the full `GuestInput` and post it to the SGX prover:
```
After:
```markdown
Request envelope that includes the full `GuestInput` and post it to the SGX prover:
```

- [ ] **Step 3: Update `tests/fixtures/README.md`**

Two edits. The fixture filename on line 3 stays.

Line 1, before:
```markdown
# Shasta Fixture
```
After:
```markdown
# Proposal GuestInput Fixture
```

Lines 3-4, before:
```markdown
`shasta_guest_input_taiko_mainnet_proposal_23077_l2_9051439_9051630.json` is a checked-in Shasta
`GuestInput` fixture used by:
```
After:
```markdown
`shasta_guest_input_taiko_mainnet_proposal_23077_l2_9051439_9051630.json` is a checked-in
`GuestInput` fixture used by:
```

- [ ] **Step 4: Verify**

Run: `grep -n "Shasta" scripts/regression/README.md tests/fixtures/README.md`
Expected: no output. Every surviving mention in these two files is lowercase `shasta` inside a
filename or path.

Run: `grep -c "shasta" scripts/regression/README.md tests/fixtures/README.md`
Expected:
```
scripts/regression/README.md:18
tests/fixtures/README.md:6
```

`tests/fixtures/README.md` drops only its title line; line 3 keeps the frozen fixture filename, so
6 of the original 7 lines survive.

Audit each remaining hit to confirm it is a filename, config path, or directory.

- [ ] **Step 5: Commit**

```bash
git add scripts/regression/README.md tests/fixtures/README.md
git commit -m "docs: retire Shasta shorthand in regression and fixture READMEs"
```

---

### Task 9: Rewrite docs/precompile-status.md for Unzen

The largest single deliverable. The current file is wrong for every live network in two independent
ways: it describes the Shasta fork mapping, and its RISC0 hook column is stale.

**Files:**
- Rewrite: `docs/precompile-status.md`

**Interfaces:**
- Consumes: the `docs/README.md` pointer updated in Task 1, which now promises Unzen coverage.
- Produces: nothing later tasks depend on.

**Facts this rewrite asserts, and where each was verified:**

| Fact | Source |
| --- | --- |
| `TaikoFork::Unzen` maps to `SpecId::OSAKA`; all other Taiko forks fall through to `SpecId::SHANGHAI` | `crates/primitives/src/chain_spec.rs:376-377` |
| Both guests pin `revm-precompile = "=34.0.0"` with `default-features = false, features = ["bn"]` | `guests/sp1/Cargo.toml:39`, `guests/risc0/Cargo.toml:29` |
| In 34.0.0, `SHANGHAI` maps to `PrecompileSpecId::BERLIN`; `OSAKA` maps to `PrecompileSpecId::OSAKA` | `revm-precompile-34.0.0/src/lib.rs`, `PrecompileSpecId::from_spec_id` |
| `osaka()` is `prague() + modexp::OSAKA + secp256r1::P256VERIFY_OSAKA` | `revm-precompile-34.0.0/src/lib.rs` |
| `prague()` is `cancun() + bls12_381::precompiles()`; `cancun()` is `berlin() + kzg_point_evaluation::POINT_EVALUATION` | `revm-precompile-34.0.0/src/lib.rs` |
| BLS12-381 occupies `0x0b` through `0x11` | `revm-precompile-34.0.0/src/bls12_381_const.rs:7-19` |
| `P256VERIFY_ADDRESS = 256` (`0x100`) | `revm-precompile-34.0.0/src/secp256r1.rs:16` |
| RISC0 overrides exactly 6 `Crypto` methods: `sha256`, `bn254_g1_add`, `bn254_g1_mul`, `secp256k1_ecrecover`, `modexp`, `secp256r1_verify_signature` | `guests/risc0/src/crypto.rs:8-42` |
| SP1 overrides exactly 4: `bn254_g1_add`, `bn254_g1_mul`, `sha256`, `secp256k1_ecrecover` | `guests/sp1/src/crypto.rs:29-73` |
| The `Crypto` trait exposes 14 overridable methods | `revm-precompile-34.0.0/src/interface.rs:201-323` |

- [ ] **Step 1: Re-verify the two facts most likely to have drifted**

Do not trust this plan's table without checking. Run:

```bash
grep -n "TaikoFork::Unzen => SpecId::OSAKA" crates/primitives/src/chain_spec.rs
grep -n 'revm-precompile' guests/sp1/Cargo.toml guests/risc0/Cargo.toml
grep -n "^    fn " guests/risc0/src/crypto.rs guests/sp1/src/crypto.rs
```

Expected: the mapping line exists; both guests pin `=34.0.0` with `features = ["bn"]`; RISC0 lists
the 6 methods and SP1 the 4 named in the table above. If any differs, stop and reconcile before
writing.

- [ ] **Step 2: Replace the file in full**

Write `docs/precompile-status.md` with exactly this content:

```markdown
# Precompile Status for Unzen

This document describes the precompile surface relevant to the current `raiko2` proving path. Every
Taiko network runs the Unzen fork.

It answers three separate questions:

1. Which precompiles are active under the Unzen fork mapping?
2. Which active precompiles are routed through guest-specific crypto hooks?
3. Where do the RISC0 and SP1 guests differ?

Use this file together with:

- the upstream `alethia-reth` `crates/evm/src/spec.rs` tests at the revision pinned in `Cargo.lock`
- `guests/risc0/src/crypto.rs`
- `guests/sp1/src/crypto.rs`

## Fork Mapping

`raiko2` maps `TaikoFork::Unzen` to Ethereum `SpecId::OSAKA`
(`crates/primitives/src/chain_spec.rs:376`). Every other Taiko fork, including Shasta, falls
through to `SpecId::SHANGHAI` (`:377`).

This document describes `revm-precompile` version `34.0.0`, which is what both guests pin
(`guests/sp1/Cargo.toml`, `guests/risc0/Cargo.toml`). The workspace lockfile also contains
`41.0.0` for host-side crates; that is not the version the guests compile against.

In `34.0.0`, `SHANGHAI` collapses to `PrecompileSpecId::BERLIN` while `OSAKA` maps to
`PrecompileSpecId::OSAKA`, composed as:

```
osaka   = prague + modexp::OSAKA + secp256r1::P256VERIFY_OSAKA
prague  = cancun + bls12_381::precompiles()
cancun  = berlin + kzg_point_evaluation::POINT_EVALUATION
berlin  = istanbul + modexp::BERLIN
istanbul = byzantium + bn254 repricing + blake2::FUN
```

Unzen therefore activates nine addresses that the Shasta path never had: `0x0A`, `0x0B` through
`0x11`, and `0x100`.

## Active Precompiles and Guest Hook Coverage

Every address below is active under Unzen. The `Crypto` trait exposes 14 overridable methods;
`Risc0GuestCrypto` overrides 6 and `Sp1GuestCrypto` overrides 4.

| Address | Precompile | Introduced | RISC0 hook | SP1 hook |
| --- | --- | --- | --- | --- |
| `0x01` | `ECRECOVER` | Homestead | Yes | Yes |
| `0x02` | `SHA256` | Homestead | Yes | Yes |
| `0x03` | `RIPEMD160` | Homestead | No | No |
| `0x04` | `IDENTITY` | Homestead | No hook exists | No hook exists |
| `0x05` | `MODEXP` | Byzantium, repriced by Osaka | Yes | No |
| `0x06` | `BN254_ADD` | Byzantium | Yes | Yes |
| `0x07` | `BN254_MUL` | Byzantium | Yes | Yes |
| `0x08` | `BN254_PAIRING` | Byzantium | No | No |
| `0x09` | `BLAKE2F` | Istanbul | No | No |
| `0x0A` | `KZG_POINT_EVALUATION` | Cancun | No | No |
| `0x0B` | `BLS12_381_G1_ADD` | Prague | No | No |
| `0x0C` | `BLS12_381_G1_MSM` | Prague | No | No |
| `0x0D` | `BLS12_381_G2_ADD` | Prague | No | No |
| `0x0E` | `BLS12_381_G2_MSM` | Prague | No | No |
| `0x0F` | `BLS12_381_PAIRING` | Prague | No | No |
| `0x10` | `BLS12_381_MAP_FP_TO_G1` | Prague | No | No |
| `0x11` | `BLS12_381_MAP_FP2_TO_G2` | Prague | No | No |
| `0x100` | `P256VERIFY` | Osaka | Yes | No |

`0x04` `IDENTITY` is a byte copy with no cryptographic work, so the `Crypto` trait defines no hook
for it.

## Two Findings

**The eight precompiles newly activated by Unzen have no guest hook in either backend.** `0x0A` and
`0x0B` through `0x11` run entirely on default backend implementations inside the zkVM.

**RISC0 and SP1 diverge on three addresses.** RISC0 overrides `modexp` and
`secp256r1_verify_signature`; SP1 overrides neither. So `0x05` `MODEXP` and the newly active `0x100`
`P256VERIFY` run unaccelerated under SP1 but accelerated under RISC0.

## Backend Selection

Both guests build `revm-precompile` with `default-features = false, features = ["bn"]`. That
selects the `bn` crate for BN254 and enables neither `blst` nor `c-kzg`.

Consequences:

- BN254 operations use the `bn` backend, further overridden by guest hooks at `0x06` and `0x07`.
- BLS12-381 (`0x0B` through `0x11`) falls back to the pure-Rust `ark-bls12-381` implementation.
- KZG (`0x0A`) falls back to the pure-Rust arkworks implementation rather than `c-kzg`.

The SP1 hooks are not syscall shims in the way the RISC0 hooks are. `Sp1GuestCrypto::sha256`
delegates to `sha2::Digest`, `secp256k1_ecrecover` to `k256`, and the BN254 operations to hand
written `BigUint` arithmetic. These depend on SP1's patched crate graph for acceleration rather than
on direct precompile calls.

## KZG Clarification

Blob validation in `raiko2` does not use the EVM `0x0A` KZG point-evaluation precompile, even
though Unzen activates it.

The proving path computes and verifies KZG commitments and proof-of-equivalence data inside the
guest and runtime utility code under `crates/primitives/src/blob/util.rs`.

The absence of a guest hook for `0x0A` therefore says nothing about blob proof validation, which
does not route through that precompile.

## What This Document Does Not Establish

This document establishes the active Unzen precompile address set and the guest hook coverage
`raiko2` installs today.

It does not establish which of these precompiles real proposals actually reach. Whether Taiko
execution ever touches `0x0A`, `0x0B` through `0x11`, `0x08`, or `0x09` depends on transaction
content and execution trace, and has not been traced. Treat the newly activated addresses as a
correctness surface that is now reachable in principle, not as a measured hot path.
```

- [ ] **Step 3: Verify the rewrite**

Run: `grep -n "Shasta\|shasta\|SHASTA" docs/precompile-status.md`
Expected: exactly one hit, the Fork Mapping sentence reading `including Shasta, falls`. This is a
correct historical statement about the fork fallthrough, not current-fork shorthand.

Run: `grep -c "^| \`0x" docs/precompile-status.md`
Expected: `18` table rows, one per active address.

Confirm the doc no longer claims any address is inactive:
Run: `grep -n "not activate\|is not active" docs/precompile-status.md`
Expected: no output.

- [ ] **Step 4: Commit**

```bash
git add docs/precompile-status.md
git commit -m "docs: rewrite precompile status for Unzen and correct RISC0 hook table"
```

---

### Task 10: Final verification sweep

**Files:** none modified unless the sweep finds a defect.

**Interfaces:**
- Consumes: all prior tasks.
- Produces: the completed branch.

- [ ] **Step 1: Confirm no code was touched**

Run: `git diff --name-only main...HEAD`

At this point the branch still carries the spec and plan; they are removed in Step 7. Expected:
exactly these 15 paths and nothing else.
```
CONTEXT.md
README.md
docs/API.md
docs/README.md
docs/architecture.md
docs/assets/readme-banner.png
docs/assets/readme-banner.svg
docs/development.md
docs/gaiko2-remote-prover-integration.md
docs/operations.md
docs/precompile-status.md
docs/superpowers/plans/2026-08-27-unzen-fork-docs.md
docs/superpowers/specs/2026-08-27-unzen-fork-docs-design.md
scripts/regression/README.md
tests/fixtures/README.md
```

If any `.rs`, `.toml`, `.py`, `.json`, or `.yml` path appears, revert it.

- [ ] **Step 2: Confirm out-of-scope docs are untouched**

Run: `git diff --name-only main...HEAD -- docs/plans docs/hoodi-txlist-witness-rollout.md experiments AGENTS.md CONTRIBUTING.md config.example.toml`
Expected: no output.

- [ ] **Step 3: Audit every surviving mention across touched files**

```bash
git diff --name-only main...HEAD -- '*.md' | xargs grep -n "Shasta\|shasta\|SHASTA"
```

Read every line of output. Each must be one of:
- an entry on the Global Constraints frozen list
- the `CONTEXT.md` glossary line naming both Shasta and Unzen
- the `docs/precompile-status.md` fork-fallthrough sentence
- one of the two hoodi verifier warning blocks
- a line in this plan or the spec

Anything else is a defect. Fix it and re-run.

- [ ] **Step 4: Confirm no prose adjective survives repo-wide in scope**

```bash
git diff --name-only main...HEAD -- '*.md' \
  | grep -v superpowers \
  | xargs grep -n "Shasta-first\|Shasta pipeline\|Shasta proof service\|Shasta preflight\|Shasta manifests\|Shasta guest\|Shasta inputs\|Shasta verifier"
```
Expected: no output.

- [ ] **Step 5: Confirm the banner and its alt text agree**

Run: `grep -n "readme-banner.png" README.md`
Expected: `1:![Raiko2 — Taiko proof orchestration](docs/assets/readme-banner.png)`

Run: `grep -c "Shasta\|shasta\|SHASTA\|HOODI" docs/assets/readme-banner.svg`
Expected: `0`

- [ ] **Step 6: Check internal links still resolve**

The only heading this change could have altered is in `docs/API.md`, which was deliberately
retained. Confirm nothing links to a heading that moved:

```bash
grep -rn "](#" README.md docs/*.md | grep -i shasta
```
Expected: no output, or only links to `#legacy-v3-submit-shasta-batch-proof`, which still exists.

- [ ] **Step 7: Remove the working artifacts, then push and open a draft PR**

The spec and plan are working artifacts for this change, not documentation the repository ships.
Preserve them outside the repo first, then remove them from the branch.

```bash
mkdir -p /private/tmp/claude-501/-Users-davidcai-taiko-raiko2/88459047-b4a2-4c0b-b6e7-6f27edce604d/scratchpad/unzen-docs-artifacts
cp docs/superpowers/specs/2026-08-27-unzen-fork-docs-design.md \
   docs/superpowers/plans/2026-08-27-unzen-fork-docs.md \
   /private/tmp/claude-501/-Users-davidcai-taiko-raiko2/88459047-b4a2-4c0b-b6e7-6f27edce604d/scratchpad/unzen-docs-artifacts/
git rm -q docs/superpowers/specs/2026-08-27-unzen-fork-docs-design.md \
          docs/superpowers/plans/2026-08-27-unzen-fork-docs.md
git commit -m "docs: drop working spec and plan from the branch"
```

Confirm the net diff no longer contains them:

```bash
git diff --name-only main...HEAD -- docs/superpowers
```
Expected: no output.

Then confirm the final net diff is exactly these 13 paths:
```
CONTEXT.md
README.md
docs/API.md
docs/README.md
docs/architecture.md
docs/assets/readme-banner.png
docs/assets/readme-banner.svg
docs/development.md
docs/gaiko2-remote-prover-integration.md
docs/operations.md
docs/precompile-status.md
scripts/regression/README.md
tests/fixtures/README.md
```

Push and open the PR as a **draft**:

```bash
git push -u origin docs/unzen-fork-naming
```

The draft PR body states: the naming rule, the 13 files touched, that no code changed, and the
three out-of-scope follow-ups — foremost the hoodi verifier misregistration in
`xtask/src/register_image.rs:417`.

