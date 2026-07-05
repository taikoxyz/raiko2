# GCE TDX Smoke Runbook

Date: 2026-06-09

## Status

Draft runbook.

## Purpose

This runbook is for the first TDX workflow validation on a GCE TDX VM. It is
not a production trust procedure.

The immediate goal is to prove:

- The VM is a GCE Confidential VM with Intel TDX enabled.
- The kernel exposes TDX quote and TPM devices.
- The Nethermind TDX image workflow can be exercised in `DEV=true` mode.
- We can later replace the in-VM prover stack with `local L2 RPC :8545 +
  gaiko2-tdx`.

## Trust Boundary

An SSH-accessible Ubuntu VM is only valid for smoke testing. It does not prove
the final production image identity.

Production trust must use:

- A measured TDX VM image.
- A release measurement artifact.
- A live quote cross-check against the release measurements.
- No SSH/debug injection in the production image.
- `register-tdx --release-url ...` as a hard gate before trusting/registering
  the instance.

Do not run production `--trust` for a VM that was manually configured over SSH.

## Current Expected VM Shape

The first test VM is expected to look like:

- Provider: GCP
- VM type: GCE Confidential VM
- Machine type: `c3-standard-4`
- Zone: `us-central1-a`
- OS: Ubuntu 24.04
- TDX devices: `/dev/tdx_guest`, `/dev/tpm0`, `/dev/tpmrm0`

## Step 1: Confirm VM Type

Run on the VM:

```bash
hostnamectl
systemd-detect-virt
sudo dmidecode -s system-manufacturer
sudo dmidecode -s system-product-name
curl -fsS -H "Metadata-Flavor: Google" \
  http://metadata.google.internal/computeMetadata/v1/instance/machine-type || true
curl -fsS -H "Metadata-Flavor: Google" \
  http://metadata.google.internal/computeMetadata/v1/instance/zone || true
```

Expected:

- `Virtualization: google`
- `Hardware Model: Google Compute Engine`
- machine type like `projects/.../machineTypes/c3-standard-4`

If this is true, the machine is a GCE VM, not bare metal.

## Step 2: Confirm TDX Devices

Run on the VM:

```bash
ls -l /dev/tdx_guest /dev/tpm* 2>/dev/null || true
sudo dmesg | grep -iE 'tdx|confidential|tpm|vtpm' | tail -120
```

Expected:

- `/dev/tdx_guest`
- `/dev/tpm0`
- `/dev/tpmrm0`
- dmesg contains `tdx: Guest detected`
- dmesg contains `Memory Encryption Features active: Intel TDX`

## Step 3: Baseline Capacity

Run:

```bash
df -h
lsblk -f
free -h
nproc
git --version
go version || true
rustc --version || true
cargo --version || true
docker version || true
```

For devnet smoke, `4 vCPU / 16 GB RAM / 100-200 GB disk` is acceptable. For
long-running chain sync, increase storage. A full node style setup may require
hundreds of GB to TB-scale storage depending on network and retention.

## Step 4: Clone Reference Repos

Use a clean work directory:

```bash
mkdir -p ~/tdx-work
cd ~/tdx-work

git clone https://github.com/NethermindEth/nethermind-tdx.git
git clone https://github.com/NethermindEth/reth-tdx.git
git clone https://github.com/taikoxyz/gaiko2.git
git clone https://github.com/taikoxyz/raiko2.git
```

If testing Nethermind's current Taiko/Raiko2 branch:

```bash
cd ~/tdx-work/nethermind-tdx
git fetch origin
git checkout feat/taiko-raiko2
```

## Step 5: Nethermind Standard DEV Image Flow

This is the recommended way to validate the real image workflow. It creates a
new measured VM image and deploys a new GCE TDX VM.

Run from a machine with GCP credentials and access to the target project/bucket.
This can be the current VM or another control machine.

Build a debug image:

```bash
cd ~/tdx-work/nethermind-tdx
make build IMAGE=taiko-tdx-prover GCP=true DEV=true
```

Deploy it:

```bash
go run ./tools/deploy-gcp deploy \
  --id tdx-dev-1 \
  --project <gcp-project-id> \
  --bucket <gcs-bucket> \
  --disk-path build/<taiko-tdx-prover-gcp-dev-*.tar.gz> \
  --zone us-central1-a \
  --machine-type c3-standard-4 \
  --storage-gb 200 \
  --allowed-ip <your-ip>/32
```

`DEV=true` is only for debugging. It enables SSH injection/debug access and
must not be trusted as a production image.

## Step 6: Production Image Rule

After the DEV image proves the flow, rebuild without `DEV=true`:

```bash
make build IMAGE=taiko-tdx-prover GCP=true
```

Production image expectations:

- `ssh.strategy: none`
- no SSH/debug injection path
- release includes `*.measurements.json`
- GCP release includes `*.gcp_measurements.json`
- registration/trust must pass `--release-url`

If `register-tdx` allows continuing without `--release-url`, treat that as a
dev-only path. For production, the CLI should refuse `--trust` without an
explicit release measurement or an explicit unsafe/dev flag.

## Step 7: Our Target Replacement

Nethermind route:

```text
nethermind-surge local RPC + reth-tdx remote proof server
```

Our target route:

```text
local Taiko L2 node on 127.0.0.1:8545 + gaiko2-tdx remote proof server
```

Required properties:

- The local L2 RPC URL must be locked to `127.0.0.1:8545` inside the TDX image.
- The local node must bind RPC to localhost only.
- `gaiko2-tdx` exposes the remote prover API to `raiko2`.
- `raiko2` runs outside the VM and points a concrete `tdx-gaiko2` lane to
  `gaiko2-tdx`.
- The VM image measurement must cover the local node binary, gaiko2-tdx binary,
  service units, and startup scripts.

## Acceptance Gates

For an SSH smoke VM:

- TDX devices exist.
- Quote path works.
- Local services start.
- Health endpoints are green.
- Remote prover conformance can reach the provider.

For a trusted image:

- Image builds reproducibly enough to produce release measurements.
- DEV and production images have different measurements.
- Production image has no SSH/debug path.
- Live quote matches release measurements.
- On-chain trust/register flow uses the release measurement.
- A real `raiko2 -> provider` proposal smoke succeeds.
