# Raiko2 Static External IP Design

## Summary

`raiko2` already exposes a public GKE `LoadBalancer` Service at `http://34.87.10.238:8080`.
The missing property is not reachability, but stability: the current external IP is only the
load balancer's assigned address and is not yet treated as a reserved, canonical static asset.

This design aligns `raiko2` with old `raiko`'s access model:

- keep a single public `LoadBalancer` Service
- keep a bare IP entrypoint
- reserve that IP in GCP as a regional static external IPv4 address
- bind the Kubernetes Service to that reserved address through the deployment manifest source of truth

The application does not gain a new "public URL" setting. This remains infrastructure-owned.

## Goals

- Give canonical `raiko2` one stable public IPv4 address that survives Service updates and rollouts.
- Match old `raiko`'s public access model as closely as possible.
- Keep the implementation in the primary infrastructure path instead of introducing app-level URL config.
- Preserve the existing external endpoint if possible to avoid downstream churn.

## Non-Goals

- No DNS name or HTTPS termination in this change.
- No Ingress or Gateway migration in this change.
- No second public endpoint for `sp1` or other backend-specific traffic.
- No change to `raiko2` HTTP routes or config schema.

## Current State

The current canonical deployment already uses a public Service:

- namespace: `tolba-raiko2-host`
- service: `raiko2`
- type: `LoadBalancer`
- current external address: `34.87.10.238`

This is functionally correct for public access, but operationally incomplete because the IP is not
yet modeled as an explicitly reserved regional address owned by infrastructure configuration.

## Design

### Canonical Ownership

The single source of truth for the public endpoint is the Kubernetes `Service` for canonical
`raiko2`, managed from the infrastructure repository (`raiko-k8s` or equivalent deployment
manifests), not from the Rust application repository.

`raiko2` continues to bind to `0.0.0.0:8080` internally. The public address remains an
infrastructure concern.

### Public Endpoint Model

The public endpoint remains:

- protocol: HTTP
- port: `8080`
- exposure mechanism: GKE external `LoadBalancer` Service

The desired canonical endpoint after this change is still:

- `http://34.87.10.238:8080`

but with `34.87.10.238` backed by a reserved regional static IP resource and explicitly referenced
by the Service manifest.

### Static IP Binding

Implementation should use the standard GKE `LoadBalancer` Service static IP path:

- reserve `34.87.10.238` as a regional external IPv4 address in the cluster region
- bind the Service to that reserved address using the Kubernetes Service manifest

For compatibility and minimum change, the first implementation should use `spec.loadBalancerIP`.
If the cluster is standardized on the newer GKE static IP annotation path, that can be adopted as
the canonical representation instead, but only if the deployment manifests and cluster version are
already prepared for it.

The important invariant is not the specific field; it is that the Service manifest explicitly owns
the static IP binding.

## Implementation Scope

### Infrastructure Changes

- Reserve the existing external IP in GCP if it is not already reserved.
- Update the canonical `raiko2` Service manifest to declare that static IP explicitly.
- Keep the Service type as `LoadBalancer`.

### Application Changes

No `raiko2` Rust code changes are required for this feature.

No new `server.public_url`, `server.external_ip`, or similar config should be introduced.

## Rollout Plan

1. Confirm the cluster region and project that own the current `raiko2` Service.
2. Verify whether `34.87.10.238` is already a reserved address resource.
3. If not reserved, reserve the existing IP as a regional external static IPv4 address.
4. Update the infrastructure manifest for the canonical `raiko2` Service to pin that IP.
5. Apply the Service change.
6. Verify the Service still resolves to `34.87.10.238`.
7. Verify `GET /health` and `GET /ready` over the public endpoint.

## Risks

### Wrong Region or Project

Static external addresses for GKE `LoadBalancer` Services must be regional and must live in the
same region and project as the Service's load balancer resources. Using the wrong scope causes
binding failure.

### Address Rebinding Risk

If the current IP is not reserved before Service mutation, GKE can reassign a different external
address. The reservation step must happen first.

### Drift Between Live Cluster and IaC

If the Service is patched only in-cluster and not recorded in the infrastructure source of truth,
the next apply can revert the static binding. The final state must live in the deployment manifests.

## Exit Criteria

This work is complete when all of the following are true:

- canonical `raiko2` has exactly one public `LoadBalancer` Service
- that Service is explicitly bound to a reserved regional static external IPv4 address
- the public endpoint remains reachable at a stable bare IP
- the public endpoint survives a Service reconcile or deployment rollout without IP drift
- the static IP ownership is recorded in the infrastructure source of truth
