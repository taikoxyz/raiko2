# Raiko2 Static External IP Design

This document uses placeholders such as `<assigned-external-ipv4>` for any live IPv4. Concrete
addresses are internal infrastructure only (GCP console / internal runbook); do not paste them
into revision-controlled text.

Access is **organization-internal** (private networking, VPN, allowlisted paths, etc.). This
design does **not** introduce or assume a customer-facing public DNS name.

## Summary

`raiko2` already has a GKE `LoadBalancer` Service reachable at
`http://<assigned-external-ipv4>:8080` from the intended network path.
The missing property is not reachability, but stability: the current external IP is only the
load balancer's assigned address and is not yet treated as a reserved, canonical static asset.

This design aligns `raiko2` with old `raiko`'s access model:

- keep a single `LoadBalancer` Service
- keep a stable IPv4 entrypoint (placeholder above—not a literal address in docs)
- reserve that IP in GCP as a regional static external IPv4 address
- bind the Kubernetes Service to that reserved address through the deployment manifest source of truth

The application does not gain a new externally visible URL knob in config. This remains infrastructure-owned.

## Goals

- Give canonical `raiko2` one stable externally assigned IPv4 that survives Service updates and rollouts.
- Match old `raiko`'s cluster-external access model as closely as possible.
- Keep the implementation in the primary infrastructure path instead of introducing app-level URL config.
- Preserve the existing external endpoint if possible to avoid downstream churn.

## Non-Goals

- No DNS name or HTTPS termination in this change.
- No Ingress or Gateway migration in this change.
- No second cluster-external endpoint for `sp1` or other backend-specific traffic.
- No change to `raiko2` HTTP routes or config schema.

## Current State

The current canonical deployment already uses a Service of type `LoadBalancer`:

- namespace: `tolba-raiko2-host`
- service: `raiko2`
- type: `LoadBalancer`
- current external address: `<assigned-external-ipv4>` (see GCP / infra repo)

This is functionally correct for the intended internal access path, but operationally incomplete because the IP is not
yet modeled as an explicitly reserved regional address owned by infrastructure configuration.

## Design

### Canonical Ownership

The single source of truth for the cluster-external endpoint is the Kubernetes `Service` for canonical
`raiko2`, managed from the infrastructure repository (`raiko-k8s` or equivalent deployment
manifests), not from the Rust application repository.

Inside the container, `raiko2` continues to listen on **port `8080` on all interfaces** (bind scope
is container-local). The IPv4 clients use to reach the Service remains an infrastructure concern.

### LoadBalancer Endpoint Model

The cluster-external endpoint remains:

- protocol: HTTP
- port: `8080`
- exposure mechanism: GKE external `LoadBalancer` Service

The desired canonical endpoint after this change is still:

- `http://<assigned-external-ipv4>:8080`

but with `<assigned-external-ipv4>` backed by a reserved regional static IP resource and explicitly referenced
by the Service manifest.

### Static IP Binding

Implementation should use the standard GKE `LoadBalancer` Service static IP path:

- reserve `<assigned-external-ipv4>` as a regional external IPv4 address in the cluster region
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
2. Verify whether `<assigned-external-ipv4>` is already a reserved address resource.
3. If not reserved, reserve the existing IP as a regional external static IPv4 address.
4. Update the infrastructure manifest for the canonical `raiko2` Service to pin that IP.
5. Apply the Service change.
6. Verify the Service still resolves to `<assigned-external-ipv4>`.
7. Verify `GET /health` and `GET /ready` over the assigned Service endpoint (see internal runbook for URL).

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

- canonical `raiko2` has exactly one `LoadBalancer` Service
- that Service is explicitly bound to a reserved regional static external IPv4 address
- the Service endpoint remains reachable at a stable `<assigned-external-ipv4>` (literal value only in runbooks)
- that endpoint survives a Service reconcile or deployment rollout without IP drift
- the static IP ownership is recorded in the infrastructure source of truth
