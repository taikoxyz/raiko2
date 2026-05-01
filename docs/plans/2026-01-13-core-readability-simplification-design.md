# Core Readability Simplification Design

> Historical design document. It may not match the current implementation. Use `README.md`,
> `docs/API.md`, and `config.example.toml` as the current source of truth.

## Goals
- Improve readability of core libraries without changing behavior or public interfaces.
- Reduce repetitive patterns and nested error handling.
- Keep data flow and error semantics identical.

## Non-Goals
- No feature changes or API redesigns.
- No cross-cutting architecture changes outside the three target areas.
- No performance optimizations beyond minor cleanup.

## Scope
- `crates/engine/src/lib.rs`: simplify `execute` flow and repeated task-result extraction.
- `crates/pipeline/src/lib.rs`: de-duplicate Risc0/Sp1 backend ELF selection.
- `crates/stateless/src/validation.rs`: linearize validation steps into smaller functions.

## Approach A (Selected)
Implement simplifications in three focused passes to keep risk low and diffs understandable.

### 1) Engine execute flow
Current issues:
- `execute` is long, nested, and repeats task result extraction logic for each stage.

Design:
- Extract small private helpers:
  - `prepare_stage_input(...)` – returns the input for a given stage (preflight/validate/encode/prove/aggregate) using scheduler outputs.
  - `get_stage_output<T>(...)` – fetches a typed output from a scheduler result with shared error mapping.
  - `map_task_state_error(...)` – centralizes error mapping for non-success states.
- Keep the stage enum and scheduler API unchanged; only relocate logic.

Expected effect:
- Reduced nesting and repeated `match` blocks.
- Consistent error mapping without altering error types.

### 2) Pipeline backend duplication
Current issues:
- `Risc0ShastaBackend` and `Sp1ShastaBackend` are structurally identical with repeated `elf` selection logic.

Design:
- Introduce a common struct (e.g., `ShastaElfBackend`) that holds `proposal_elf` and `aggregation_elf`.
- Implement the shared `elf(ProofStage)` logic once.
- Keep Risc0/Sp1-specific constructors that wire in their ELF bytes.

Expected effect:
- Remove duplicated `match` logic and reduce structural redundancy.

### 3) Stateless validation linearization
Current issues:
- `stateless_validation_with_trie` performs multiple responsibilities in a single function.

Design:
- Split into internal steps:
  - `decode_header_and_block(...)`
  - `run_consensus_checks(...)`
  - `compute_ancestor_hashes(...)`
  - `prepare_db(...)`
  - `execute_evm(...)`
  - `verify_state_root(...)`
- Keep arguments and return type of the public function unchanged.

Expected effect:
- Clearer control flow and easier local reasoning.

## Error Handling
- Preserve existing error types and messages.
- Centralize repeated mappings without changing what error surfaces.

## Testing
- Run existing tests for engine/pipeline/stateless modules.
- If any snapshot or golden tests exist for decoding/validation, keep inputs identical.

## Risks and Mitigations
- Risk: behavior drift in scheduler output handling.
  - Mitigation: strictly refactor by extraction, no logic changes.
- Risk: backend ELF selection regression.
  - Mitigation: keep exact match logic, only move it.
- Risk: validation flow ordering changes.
  - Mitigation: keep step order identical and use extracted functions as wrappers.
