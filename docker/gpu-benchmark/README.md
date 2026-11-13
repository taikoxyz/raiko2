# Local GPU benchmark container

Run one frozen-fixture job on one explicitly selected NVIDIA GPU. This recipe reuses an existing
local immutable CUDA image; it does not build or publish images. See [Dockerfile.cuda](../../Dockerfile.cuda)
and the existing `just release-image-cuda` workflow for the image source. The image must include
`guest-launcher` built with `--features cuda`, its matching SP1 GPU server, and the production guest
ELF pair. The host needs the NVIDIA driver and NVIDIA Container Toolkit compatible with that image.

The [single-L4 archive](results/single-l4-2026-11-12.json) records historical measurements. This
Compose recipe supports future local runs; it does not reproduce the archive's experimental driver
or cold/warm sequence. Its resource defaults match that machine's job budget: 14 CPUs, 56 GiB RAM,
and 16 GiB shared memory. Use it for one job at a time; it makes no multi-GPU throughput claim.

## Configure

Run these Bash examples from the repository root. Set `GPU_BENCHMARK_IMAGE` to an immutable image
already present locally: either `sha256:<64-character-image-ID>` or
`registry.example.com/project/raiko2-cuda@sha256:<64-character-repository-digest>`. Use the local image
ID for locally built or `docker save`/`docker load` images without a repository digest. Mutable tags
are not suitable for benchmark records. Compose requires this setting but does not validate its
form; `pull_policy: never` prevents automatic pulls. Load or obtain the image separately first.
Use existing local directories outside the checkout for private inputs and persistent outputs.

```bash
export GPU_BENCHMARK_IMAGE='sha256:<64-character-image-ID>'
export GPU_BENCHMARK_FIXTURES="$(realpath ../benchmark-fixtures)"
export GPU_BENCHMARK_RESULTS="$(realpath ../benchmark-results)"
export GPU_BENCHMARK_GPU='0' # One host GPU index or GPU UUID, never "all".

docker compose -f docker/gpu-benchmark/compose.yml config
```

The selected GPU is device `0` inside the container, matching the launcher's local SP1 default.
Optional resource overrides are `GPU_BENCHMARK_CPUS`, `GPU_BENCHMARK_MEMORY`, and
`GPU_BENCHMARK_SHM_SIZE`. Inputs mount read-only at `/fixtures`; outputs persist at `/results` after
`--rm`. The directories must already exist. Choose fresh output names for each run: the launcher
writes the named files and can overwrite existing results. Jobs have no network access and require
no RPC URL or keys when given complete frozen `GuestInput` JSON fixtures. Host environment variables
are not forwarded to the container.

## Full SP1 proposal

Place a complete proposal fixture at `$GPU_BENCHMARK_FIXTURES/proposal-1.json`. This generates and
locally verifies a compressed proof using the image's production guest pair:

```bash
docker compose -f docker/gpu-benchmark/compose.yml run --rm benchmark \
  guest-launcher --stage proposal --mode prove --proof-type sp1 --sp1-prover local \
  --proof-mode compressed --input /fixtures/proposal-1.json \
  --output /results/proposal-1.proof.json --json-out /results/proposal-1.report.json
```

Repeat with distinct fixture and output names to produce `proposal-2.proof.json` through
`proposal-n.proof.json`. Preserve the guest pair and image identity across the child proofs and
aggregation; use ordered, compatible child proofs for the intended proposal sequence.

## SP1 n-child aggregation

Plonk wrapping needs prewarmed SP1 circuit assets compatible with the image's pinned SDK version.
Prepare those separately before an offline run, or use an image that already contains them. For a
local cache, put the complete contents (including hidden completion markers) of the matching
`.sp1/circuits` tree in
`$GPU_BENCHMARK_FIXTURES/sp1-circuits`. Mount only that subtree, as shown below; mounting over
`/root/.sp1` would hide the image's pinned GPU server. An incomplete cache cannot be downloaded by
this network-isolated job.

This example aggregates two children; set `n` to the number of child proof files already produced.
The array expands to one argument per proof and supports any valid child count for the guest.

```bash
n=2
proofs=()
for ((i = 1; i <= n; i++)); do
  proofs+=("/results/proposal-${i}.proof.json")
done

docker compose -f docker/gpu-benchmark/compose.yml run --rm \
  --volume "${GPU_BENCHMARK_FIXTURES}/sp1-circuits:/root/.sp1/circuits:ro" \
  benchmark guest-launcher --stage proposal --mode prove --proof-type sp1 --sp1-prover local \
  --proof-mode plonk --aggregate "${proofs[@]}" \
  --output /results/aggregate.proof.json --json-out /results/aggregate.report.json
```

Omit `--volume` when the image already contains the required circuits. The launcher verifies the
resulting proof locally. See the existing [CUDA SDK smoke check](../../docs/development.md#cuda-sdk-smoke)
for a smaller wiring check before attempting full proposals.

## Archive boundaries

Current `guest-launcher` supports full RISC Zero proposal execution, but not full proposal proving.
ZisK is not integrated into the main runtime. Their archived full-proof results used separate
experimental drivers that are not distributed here; there is no generic RISC Zero or ZisK full-proof
command in this recipe. The tiny SDK smoke path is a separate check.

Keep fixtures, witnesses, ELFs, proofs, raw stdout/logs, and local caches out of Git. Only sanitized
summary results belong in this archive. Capture raw output locally when needed and review any
additional summary for host, account, registry, RPC, or other private identifiers before sharing it.
