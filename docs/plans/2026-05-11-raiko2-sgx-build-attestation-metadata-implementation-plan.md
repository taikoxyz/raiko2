# Raiko2 SGX Build Attestation Metadata Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make `raiko2-sgx` produce a baked `attestation.raiko2.json` file during image build so
operators can read `mr_enclave` directly from the image.

**Architecture:** Move Gramine manifest rendering and SGX signing from runtime startup into
`Dockerfile.sgx`, write a small metadata helper that derives `mr_enclave` from the generated
SIGSTRUCT, and simplify the runtime entrypoint so prebuilt `tee` images start without a mounted
signing key.

**Tech Stack:** Dockerfile BuildKit secrets, Gramine (`gramine-manifest`, `gramine-sgx-sign`,
`gramine-sgx-sigstruct-view`), bash helper scripts, repo operator docs.

---

### Task 1: Add the design/plan docs

**Files:**
- Create: `docs/plans/2026-05-11-raiko2-sgx-build-attestation-metadata-design.md`
- Create: `docs/plans/2026-05-11-raiko2-sgx-build-attestation-metadata-implementation-plan.md`

**Step 1: Write the design doc**

Describe:

- why `mr_enclave` must be build-time
- why runtime signing is too late
- the baked metadata file path
- the need for a build secret

**Step 2: Write this implementation plan**

Include exact files, TDD ordering, and verification commands.

**Step 3: Commit**

```bash
git add docs/plans/2026-05-11-raiko2-sgx-build-attestation-metadata-design.md docs/plans/2026-05-11-raiko2-sgx-build-attestation-metadata-implementation-plan.md
git commit -m "docs: plan raiko2 sgx attestation metadata"
```

### Task 2: Add a failing metadata extraction test

**Files:**
- Create: `docker/test-write-attestation-metadata.sh`
- Create: `docker/testdata/gramine-sigstruct-view.sample.json`

**Step 1: Write the failing test**

Write a shell test that:

- feeds sample `gramine-sgx-sigstruct-view --output-format=json` output into the metadata helper
- expects an output JSON file containing `mr_enclave`
- expects other stable fields such as `mr_signer` and `isv_prod_id`

The test should fail because the helper script does not exist yet.

**Step 2: Run the test to verify it fails**

Run:

```bash
bash docker/test-write-attestation-metadata.sh
```

Expected: fail with missing helper script or missing output file.

**Step 3: Commit**

```bash
git add docker/test-write-attestation-metadata.sh docker/testdata/gramine-sigstruct-view.sample.json
git commit -m "test: add sgx attestation metadata fixture"
```

### Task 3: Implement the metadata helper

**Files:**
- Create: `docker/write-attestation-metadata.sh`
- Test: `docker/test-write-attestation-metadata.sh`

**Step 1: Write minimal helper implementation**

Implement a helper that:

1. reads a `gramine-sgx-sigstruct-view --output-format=json` file
2. extracts:
   - `mr_enclave`
   - `mr_signer`
   - `isv_prod_id`
   - `isv_svn`
   - `debug_enclave`
   - `date`
3. writes `attestation.raiko2.json`

Prefer a small Python JSON parser inside the script over brittle `grep`/`sed`.

**Step 2: Run the test to verify it passes**

Run:

```bash
bash docker/test-write-attestation-metadata.sh
```

Expected: PASS and generated JSON contains the expected keys and values.

**Step 3: Commit**

```bash
git add docker/write-attestation-metadata.sh docker/test-write-attestation-metadata.sh
git commit -m "feat: add raiko2 sgx attestation metadata helper"
```

### Task 4: Move SGX signing into `Dockerfile.sgx`

**Files:**
- Modify: `Dockerfile.sgx`

**Step 1: Write the failing build command**

Run a build command that expects a baked metadata file:

```bash
DOCKER_BUILDKIT=1 docker build \
  --secret id=gramine_enclave_key,src=$HOME/.config/gramine/enclave-key.pem \
  -f Dockerfile.sgx \
  -t raiko2-sgx:attestation-test .
```

Expected before implementation: build succeeds without baked metadata or cannot access the signing
key during build.

**Step 2: Implement build-time signing**

In `Dockerfile.sgx`:

- add `/opt/raiko2-sgx/etc`
- copy the metadata helper
- use a BuildKit secret mount for the enclave key
- render the manifest
- sign it with `gramine-sgx-sign`
- dump SIGSTRUCT JSON with `gramine-sgx-sigstruct-view --output-format=json`
- write `/opt/raiko2-sgx/etc/attestation.raiko2.json`

**Step 3: Re-run the build**

Run:

```bash
DOCKER_BUILDKIT=1 docker build \
  --secret id=gramine_enclave_key,src=$HOME/.config/gramine/enclave-key.pem \
  -f Dockerfile.sgx \
  -t raiko2-sgx:attestation-test .
```

Expected: PASS

**Step 4: Commit**

```bash
git add Dockerfile.sgx
git commit -m "feat: bake sgx attestation metadata into image"
```

### Task 5: Simplify runtime startup for pre-signed images

**Files:**
- Modify: `docker/sgx-entrypoint.sh`

**Step 1: Write the failing runtime check**

Run a container with the newly built image and verify the `tee` path starts without needing to
generate fresh manifest artifacts at runtime.

Expected before implementation: entrypoint still tries to sign at startup.

**Step 2: Implement minimal runtime change**

Update the entrypoint so that for `tee` mode it:

- verifies baked manifest artifacts exist
- skips `gramine-manifest` / `gramine-sgx-sign`
- runs `bootstrap`, `check`, or `serve` directly with the prebuilt signed artifacts

Keep `native` mode behavior unchanged.

**Step 3: Verify startup**

Run:

```bash
docker run --rm raiko2-sgx:attestation-test cat /opt/raiko2-sgx/etc/attestation.raiko2.json
```

Expected: prints JSON with `mr_enclave`.

**Step 4: Commit**

```bash
git add docker/sgx-entrypoint.sh
git commit -m "refactor: use pre-signed sgx runtime artifacts"
```

### Task 6: Pass build secrets through compose

**Files:**
- Modify: `docker/docker-compose.sgx.yml`
- Modify: `docker/docker-compose.sgx.regression.yml`
- Modify: `docker/.env.sgx.regression.sample`

**Step 1: Write the failing compose config check**

Run:

```bash
docker compose --env-file docker/.env.sgx.regression.sample -f docker/docker-compose.sgx.regression.yml config
```

Expected before implementation: no build secret for the enclave key and runtime still assumes key
mount semantics.

**Step 2: Update compose/build wiring**

Add build-secret wiring for the enclave signing key and remove the runtime assumption that the key
must be mounted for normal `tee` startup.

**Step 3: Re-run the compose config check**

Run:

```bash
docker compose --env-file docker/.env.sgx.regression.sample -f docker/docker-compose.sgx.regression.yml config
```

Expected: PASS

**Step 4: Commit**

```bash
git add docker/docker-compose.sgx.yml docker/docker-compose.sgx.regression.yml docker/.env.sgx.regression.sample
git commit -m "chore: wire sgx build signing key through compose"
```

### Task 7: Document where operators read `mr_enclave`

**Files:**
- Modify: `docs/development.md`
- Modify: `docs/operations.md`

**Step 1: Update docs**

Document:

- `xtask register-image` is for zk digest registration only
- `raiko2-sgx` image measurement lives in `/opt/raiko2-sgx/etc/attestation.raiko2.json`
- SGX registration should use the baked metadata / quote with external verifier tooling

**Step 2: Verify docs reference the right path**

Run:

```bash
rg -n "attestation\\.raiko2\\.json|register-image|mr_enclave" docs
```

Expected: updated docs mention the correct SGX metadata path and separate zk/SGX registration.

**Step 3: Commit**

```bash
git add docs/development.md docs/operations.md
git commit -m "docs: describe sgx attestation metadata flow"
```

### Task 8: End-to-end verification

**Files:**
- Verify only

**Step 1: Run targeted checks**

Run:

```bash
bash docker/test-write-attestation-metadata.sh
cargo fmt --all --check
git diff --check
```

Expected: PASS

**Step 2: Build the SGX image with a real signing key**

Run:

```bash
DOCKER_BUILDKIT=1 docker build \
  --secret id=gramine_enclave_key,src=$HOME/.config/gramine/enclave-key.pem \
  -f Dockerfile.sgx \
  -t raiko2-sgx:attestation-test .
```

Expected: PASS

**Step 3: Verify the baked metadata**

Run:

```bash
docker run --rm raiko2-sgx:attestation-test cat /opt/raiko2-sgx/etc/attestation.raiko2.json
```

Expected: JSON includes non-empty `mr_enclave`.

**Step 4: Re-run a small compose bootstrap/start smoke check**

Run:

```bash
docker compose --env-file docker/.env.sgx.regression.sample -f docker/docker-compose.sgx.regression.yml config
```

Expected: PASS

**Step 5: Commit the final verification-safe state**

```bash
git status --short
```

Expected: only intended tracked changes remain.

Plan complete and saved to `docs/plans/2026-05-11-raiko2-sgx-build-attestation-metadata-implementation-plan.md`. Two execution options:

**1. Subagent-Driven (this session)** - I dispatch fresh subagent per task, review between tasks, fast iteration

**2. Parallel Session (separate)** - Open new session with executing-plans, batch execution with checkpoints

Continuing in this session with the same worktree unless redirected.
