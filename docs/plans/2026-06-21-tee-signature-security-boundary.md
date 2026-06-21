# TEE Signature Security Boundary

This note records the security model for SGX and TDX proof signatures in the
remote prover path. The important distinction is that a recoverable signature is
meaningful only when the private key is held inside the trusted execution
boundary and is not readable by the untrusted host or by untrusted runtime code.

## SGX Remote Prover

For `raiko2-sgx`, the signer key is generated during bootstrap and persisted
through the TEE provider. The Gramine manifest mounts
`/var/lib/raiko2/sgx/secrets` as an encrypted filesystem keyed by the enclave
measurement. The proof result signs the Shasta input hash and exposes the public
key, instance address, optional quote, and the encoded proof bytes.

The SGX aggregate path relies on this invariant:

> If a child proof signature recovers to the expected registered SGX instance
> address, and the instance id/address match the current aggregate enclave, the
> child proof was produced by the same enclave-held signer key. An external
> process cannot forge such a child proof without extracting the enclave key.

This is different from treating the signer key as a public or operator-shared
key. If the private key is printed, copied out, persisted in a host-readable
plaintext volume, or otherwise exposed outside the enclave boundary, this
assumption is false.

The aggregate runtime must therefore enforce all of the following:

- Every child proof input equals `hash_shasta_subproof_input(proof_carry_data)`.
- Every child proof has the expected SGX proof byte shape.
- Every child proof instance id equals the current registered instance id.
- Every child proof embedded instance address equals the current signer address.
- Every child proof signature recovers to that same instance address over the
  child input hash.
- The carry-data vector remains internally consistent.

After these checks, the aggregate proof signs the aggregation input hash using
the current enclave signer. This is the SGX analogue of a ZK aggregation guest
verifying all child proofs before producing an aggregate proof.

## Comparison With ZK Aggregation

ZK aggregation and SGX aggregation have different trust bases:

- ZK aggregation verifies child proofs using cryptographic verification keys and
  image ids inside the aggregation circuit or guest.
- SGX aggregation verifies child proofs using the enclave-held signer identity,
  DCAP registration, and the non-exportability of the enclave signer key.

Both paths still need the same public-input binding:

- child proof input hash must match the child proof carry data
- carry data must form a valid Shasta proposal sequence
- aggregate output must be derived from the committed carry-data vector and the
  prover identity expected by the verifier

The SGX path is not secure because "anyone can sign". It is secure only under
the opposite assumption: only the measured enclave can use the registered
signer key.

## TDX Applicability

The same high-level signature assumption can apply to TDX, but only if the TDX
deployment gives the signer key the same effective protection that SGX gives the
enclave key.

For TDX, the trusted boundary is the measured VM, not a single process enclave.
Therefore, "same recovered signer" means "same measured VM-held key" only under
these conditions:

- The accepted TDX image measurement is registered and checked by verifiers.
- The VM is launched from the measured image, not from mutable host-mounted
  scripts, floating container tags, or runtime-pulled binaries.
- SSH, serial console command execution, cloud guest-agent command channels, and
  other operator mutation paths are disabled or outside the trusted production
  profile.
- The only externally reachable application port is the intended prover service
  API, for example port `8080`.
- The signer key is generated or provisioned inside the measured TDX VM and is
  not baked into a public image.
- The signer key is not stored on host-readable plaintext disk, copied into
  logs, exposed through debug endpoints, or readable through a non-TDX secret
  mount.
- The root filesystem and configuration are either measured, immutable, or
  otherwise bound into the attested launch state.
- The quote binds the expected report data, such as the instance address during
  bootstrap and the signed proof input hash during proof generation.
- `tdxs` is reachable only inside the VM and cannot be used by an external host
  process to mint quotes for arbitrary untrusted code.

Under those conditions, an external attacker cannot sign a fake proof because
the key is only usable by the measured TDX VM. If any condition fails, TDX
signature verification degrades to "a key on a server signed this", which is not
equivalent to SGX or ZK proof verification.

The current `gaiko2` TDX provider reads and writes the signer key as an ordinary
file under its configured secret directory. That is acceptable only if the TDX
image, disk, and secret-storage profile make that file unavailable to the
untrusted host and to untrusted runtime code. It is not enough for the code to
call the provider `tdx`.

The main difference from SGX is process isolation. SGX protects the enclave key
from other host processes by construction. TDX protects the VM from the host, but
processes inside the VM can still read ordinary files unless the image, service
layout, permissions, and sealed/encrypted storage model prevent that. No SSH and
only exposing port `8080` are necessary hardening steps, but they are not by
themselves sufficient unless the measured image and signer-key storage are also
part of the trust statement.

## Operational Rules

- Never print or return the signer private key.
- Never mount the signer secret directory as host-readable plaintext in
  production.
- Treat native or mock TEE modes as test-only; their fixed keys do not carry the
  TEE non-exportability assumption.
- Register and verify the release identity before accepting proofs from a new
  SGX enclave or TDX VM image.
- If the runtime image, enclave measurement, VM measurement, signer key, or
  quote-binding scheme changes, re-evaluate this document before deployment.
