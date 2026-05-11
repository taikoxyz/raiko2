# Raiko2 Open Source Readiness Review

## Summary

`raiko2` is not quite ready to open source as-is.

The main blockers are not architectural. They are mostly repository hygiene, hardcoded/mock
security-sensitive behavior, and documentation that still assumes an internal operator or
AI-assisted workflow.

The good news is that the cleanup is finite:

1. remove tracked crash/runtime artifacts
2. remove or isolate the hardcoded native mock private key
3. sanitize public-facing docs and examples
4. add a minimal open-source repo policy surface

## Blockers

### 1. Tracked crash/runtime artifacts must be removed

The repository currently tracks generated artifacts that should not live in a public source tree:

- `guests/risc0/core.268699`
- `guests/risc0/core.280521`
- `guests/sp1/core.280557`
- `test/regression/shasta/regression.log`

These are not source files and do not help downstream users understand or build the project.
They should be deleted from git history going forward and ignored in `.gitignore`.

Recommended follow-up:

- remove the tracked files
- add ignore rules for `core.*` and regression/runtime log artifacts
- verify there are no other generated runtime leftovers in `test/` or `guests/`

### 2. `NativeProver` contains a hardcoded private key

`crates/prover/src/native.rs` defines:

- `NATIVE_PROOF_PRIVATE_KEY`

and then uses it in runtime code paths to:

- sign the native proof hash
- derive the signer address

This is likely intended as a deterministic local/mock path, not a production secret leak.
Even so, a public repository should not normalize hardcoded private-key material in non-test code.

Recommended follow-up:

- replace the hardcoded key with explicit mock/test-only behavior
- gate the behavior behind a feature, fixture path, or generated deterministic test key that cannot
  be confused with operational credentials
- document clearly that `native/local` is non-verifying and mock-oriented

### 3. Public-facing docs still expose internal AI/operator workflow assumptions

Several tracked files make the repo look like an internal working tree rather than a public project:

- `README.md` points readers to `.codex/skills/raiko2-image-release/`
- `AGENTS.md` instructs agents to use internal project skills and local execution rules
- `docs/development.md` references `~/code/github.com/taikoxyz/alethia-reth`
- `.codex/skills/*` is tracked and visible as part of the repo

This is not a secret leak by itself, but it is poor public packaging.
It exposes internal workflow scaffolding that external contributors do not need.

Recommended follow-up:

- remove agent-specific guidance from `README.md`
- decide whether `AGENTS.md`, `CLAUDE.md`, and `.codex/skills/*` should stay in the public repo at
  all
- if they must stay, move them out of the main user-facing documentation path and make sure the
  public docs do not treat them as part of the normal project workflow

## High-Priority Cleanup

### 4. Replace environment-specific RPC/IP and deployment references in public examples

The repository still contains many real-looking infra references:

- raw RPC IPs in `config.example.toml`
- raw RPC IPs in `config/chain_spec_list_default.json`
- repeated raw RPC IPs inside checked-in guest input fixtures under `test/guest_inputs/`
- `tolba`, `hoodi-shasta`, and `us-docker.pkg.dev/evmchain/images/raiko2` in operations/release
  docs
- Boundless deployment endpoints such as `base-mainnet.boundless.network`

These values may be public, but they make the repo feel coupled to one operator environment.
For an open-source release, examples should default to placeholders or clearly documented public
sample endpoints.

Recommended follow-up:

- replace raw IPs in examples with `example.com`-style placeholders where possible
- keep only the minimum number of real public endpoints needed for developer success
- scrub internal environment names from public-facing operational docs
- review large checked-in fixtures to decide whether they should be minimized or regenerated from a
  sanitized source

### 5. Historical/internal docs dominate the public docs tree

The docs tree currently includes a large `docs/plans/` directory, a `docs/issues/` directory, and a
docs index that still highlights an internal production-readiness note as the "latest dated
assessment".

This is useful internally, but it is noisy for a public project.

Recommended follow-up:

- keep historical plans if they are valuable, but clearly separate them from the public onboarding
  path
- make `docs/README.md` primarily point users to architecture, API, setup, and contribution docs
- avoid presenting internal planning artifacts as the main public narrative

## Missing Open-Source Project Surface

The repo appears to be missing some standard public-project files:

- `SECURITY.md`
- `CONTRIBUTING.md`
- `CODEOWNERS` (optional, but helpful)

None of these are hard blockers for publishing code, but together they affect how credible and
maintainable the public repository feels.

Recommended follow-up:

- add a minimal `SECURITY.md` with reporting instructions
- add a short `CONTRIBUTING.md` with local setup, checks, and PR expectations
- optionally add `CODEOWNERS` if ownership routing matters

## Review Scope Notes

### What this review did confirm

- no committed `*.pem`, `*.key`, `*.crt`, `*.p12`, `*.pfx`, `*.jks`, or `*.keystore` files were
  found
- editor config files under `.vscode/` and `.zed/` are benign
- the most obvious sensitive artifact is the hardcoded mock private key in `NativeProver`

### What this review did not fully confirm

Dependency vulnerability status is still unknown.

This environment did not have:

- `cargo-audit`
- `cargo-deny`

installed, so this review cannot claim that Rust dependencies are free of known CVEs or license
issues.

Recommended follow-up:

- run `cargo audit`
- run `cargo deny check advisories licenses bans sources`

## Suggested Cleanup Order

1. Delete tracked crash/runtime artifacts and add ignore rules.
2. Fix `NativeProver` so no hardcoded private key remains in non-test code.
3. Remove internal/agent workflow references from `README.md` and decide what to do with
   `AGENTS.md`, `CLAUDE.md`, and `.codex/skills/*`.
4. Sanitize example configs, operations docs, and checked-in fixtures.
5. Add `SECURITY.md` and `CONTRIBUTING.md`.
6. Run dependency and license auditing before announcing public availability.

## Readiness Call

Current call: **not ready yet, but close enough that a short cleanup pass should get it there.**

The blocking work is mostly packaging and hygiene rather than deep product redesign.
