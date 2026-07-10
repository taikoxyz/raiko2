# SGX EDMM Image Variants Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make one TEE provider release produce a default non-EDMM `raiko2-sgx` image and an explicit `-edmm` image with separate digests and attestation metadata.

**Architecture:** Parameterize the signed Gramine manifest with a validated Docker build argument, then model the two local SGX variants in `xtask`. Keep the unsuffixed tag non-EDMM, suffix the EDMM tag, and serialize the capability on each local image entry while leaving external provider behavior unchanged.

**Tech Stack:** Rust, Clap xtask, Serde JSON, Docker BuildKit, Gramine manifest templates, Docker Compose

---

### Task 1: Record EDMM Capability In The TEE Handoff Manifest

**Files:**
- Modify: `xtask/src/release_tee_manifest.rs`

**Step 1: Write the failing serialization test**

Add `sgx_edmm: Some(false)` to the local `TeeProviderImage` fixture and assert the JSON contains:

```rust
assert!(contents.contains("\"sgx_edmm\": false"));
```

Add a second fixture with `sgx_edmm: None` and assert the field is omitted.

**Step 2: Run the focused test and verify it fails**

Run: `cargo test -p xtask release_tee_manifest -- --nocapture`

Expected: FAIL because `TeeProviderImage` has no `sgx_edmm` field.

**Step 3: Add the optional image capability field**

Extend the image schema:

```rust
pub(crate) struct TeeProviderImage {
    pub(crate) repository: String,
    pub(crate) tag: String,
    pub(crate) digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sgx_edmm: Option<bool>,
}
```

Set existing external or generic test fixtures to `None` until the local variant builder is updated.

**Step 4: Run the focused test and verify it passes**

Run: `cargo test -p xtask release_tee_manifest -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add xtask/src/release_tee_manifest.rs
git commit -m "feat(xtask): record SGX EDMM image capability"
```

### Task 2: Parameterize The Signed Gramine Manifest

**Files:**
- Modify: `Dockerfile.sgx`
- Modify: `docker/raiko2-sgx-prover.manifest.template`
- Modify: `docker/docker-compose.sgx.yml`
- Modify: `docker/docker-compose.sgx.regression.yml`
- Modify: `docker/.env.sgx.sample`
- Modify: `docker/.env.sgx.regression.sample`

**Step 1: Add a static regression test for the build contract**

Add a focused Rust test in `xtask/src/release_tee_providers.rs` that reads the repository Dockerfile
and manifest template and asserts:

```rust
assert!(dockerfile.contains("ARG SGX_EDMM_ENABLE=false"));
assert!(dockerfile.contains("-Dedmm_enable=\"${SGX_EDMM_ENABLE}\""));
assert!(manifest.contains("sgx.edmm_enable = {{ edmm_enable }}"));
```

**Step 2: Run the test and verify it fails**

Run: `cargo test -p xtask release_tee_providers_renders_edmm_build_argument -- --nocapture`

Expected: FAIL because EDMM is currently hard-coded in the template.

**Step 3: Implement the Docker and Gramine parameter**

In `Dockerfile.sgx`:

```dockerfile
ARG SGX_EDMM_ENABLE=false
```

Before `gramine-manifest`, reject values other than `true` and `false`, then pass:

```dockerfile
-Dedmm_enable="${SGX_EDMM_ENABLE}"
```

In the manifest template, render the unquoted boolean:

```toml
sgx.edmm_enable = {{ edmm_enable }}
```

Forward `SGX_EDMM_ENABLE: ${SGX_EDMM_ENABLE:-false}` from both Compose build definitions and add
`SGX_EDMM_ENABLE=false` to both SGX env samples.

**Step 4: Verify the focused test and Compose rendering**

Run: `cargo test -p xtask release_tee_providers_renders_edmm_build_argument -- --nocapture`

Expected: PASS.

Run: `docker compose --env-file docker/.env.sgx.sample -f docker/docker-compose.sgx.yml config`

Expected: PASS and the rendered build argument is `SGX_EDMM_ENABLE: "false"`.

Run: `docker compose --env-file docker/.env.sgx.regression.sample -f docker/docker-compose.sgx.regression.yml config`

Expected: PASS and the rendered local SGX build argument is `SGX_EDMM_ENABLE: "false"`.

**Step 5: Commit**

```bash
git add Dockerfile.sgx docker/raiko2-sgx-prover.manifest.template docker/docker-compose.sgx.yml docker/docker-compose.sgx.regression.yml docker/.env.sgx.sample docker/.env.sgx.regression.sample xtask/src/release_tee_providers.rs
git commit -m "feat(sgx): parameterize EDMM enclave builds"
```

### Task 3: Build Both Local SGX Variants In One Release

**Files:**
- Modify: `xtask/src/release_tee_providers.rs`
- Test: `xtask/src/release_tee_providers.rs`

**Step 1: Write failing tag and command tests**

Add tests proving:

```rust
assert_eq!(local_sgx_variant_tag("v1.2.3", false), "v1.2.3");
assert_eq!(local_sgx_variant_tag("v1.2.3", true), "v1.2.3-edmm");
```

Build two local Docker commands and assert one contains
`SGX_EDMM_ENABLE=false`, the other contains `SGX_EDMM_ENABLE=true`, and both contain the same
`GRAMINE_ENCLAVE_KEY_SHA256` and BuildKit secret source.

Add a pure manifest-entry test that expects provider names `raiko2-sgx` and
`raiko2-sgx-edmm`, lane `sgx`, and `sgx_edmm` values `false` and `true`.

**Step 2: Run the focused tests and verify they fail**

Run: `cargo test -p xtask release_tee_providers_local_sgx -- --nocapture`

Expected: FAIL because variant helpers and dual entries do not exist.

**Step 3: Implement local variant descriptors and build commands**

Introduce a small private descriptor:

```rust
struct LocalSgxVariant {
    provider: &'static str,
    edmm: bool,
}
```

Define exactly two variants, derive the tag with `-edmm` only for the EDMM variant, and create a
testable `local_sgx_docker_build_command` that forwards:

```text
GRAMINE_ENCLAVE_KEY_SHA256=<hash>
SGX_EDMM_ENABLE=<true|false>
```

Resolve the Gramine signing key once, then use the same key for both builds.

**Step 4: Emit and validate both local manifest entries**

Replace `build_local_provider_entry` with `build_local_provider_entries`. For each variant, build,
push unless `--no-push`, resolve its own digest, read its own attestation metadata, and set
`image.sgx_edmm = Some(variant.edmm)`. Append external providers afterward with
`image.sgx_edmm = None`.

**Step 5: Run the focused xtask tests**

Run: `cargo test -p xtask release_tee_providers -- --nocapture`

Expected: PASS.

**Step 6: Commit**

```bash
git add xtask/src/release_tee_providers.rs
git commit -m "feat(xtask): release EDMM and non-EDMM SGX images"
```

### Task 4: Document Variant Selection And Release Output

**Files:**
- Modify: `docs/operations.md`

**Step 1: Update operator documentation**

Document that:

- `<release>` is non-EDMM and works on hosts without EDMM support;
- `<release>-edmm` explicitly selects EDMM;
- both local variants are built and recorded by `release-tee-providers`;
- each variant has a distinct digest and MRENCLAVE;
- local Compose defaults to non-EDMM and accepts `SGX_EDMM_ENABLE=true` for an explicit EDMM build.

Update the release notes template to list both local image digests and both MRENCLAVE values.

**Step 2: Verify commands and paths against the implementation**

Run: `rg -n "SGX_EDMM_ENABLE|raiko2-sgx-edmm|non-EDMM" Dockerfile.sgx docker xtask/src docs/operations.md`

Expected: all documented names match implemented names.

**Step 3: Commit**

```bash
git add docs/operations.md
git commit -m "docs(sgx): explain EDMM image variants"
```

### Task 5: Run Final Verification

**Files:**
- Verify all modified files.

**Step 1: Run formatting and focused tests**

Run: `cargo fmt --all -- --check`

Expected: PASS.

Run: `cargo test -p xtask release_tee -- --nocapture`

Expected: PASS.

**Step 2: Run Clippy**

Run: `cargo clippy -p xtask --locked -- -D warnings`

Expected: PASS.

**Step 3: Validate Compose and source hygiene**

Run: `docker compose --env-file docker/.env.sgx.sample -f docker/docker-compose.sgx.yml config`

Run: `docker compose --env-file docker/.env.sgx.regression.sample -f docker/docker-compose.sgx.regression.yml config`

Run: `git diff --check origin/main...HEAD`

Expected: all commands PASS. Review added and modified files for hard-coded machine paths or
person-identifying names; none should be present.

**Step 4: Review final diff and commit any verification-only corrections**

Run: `git status --short`

Expected: clean worktree after any necessary correction commit.
