# Open Prover Platform Landscape To Investigate

## Status

Draft research note.

## Purpose

This document collects external projects that are relevant to the `raiko2` open prover platform
direction.

The goal is not to copy any one project wholesale. The goal is to answer a narrower question:

- is there real market demand for proof-supply abstraction and multi-provider proving infrastructure
- which projects are actually exploring adjacent structure
- which parts are worth studying more closely
- which parts are not worth overfitting to

## Quick Take

The external landscape suggests that the direction is real.

There is now visible activity in at least three adjacent areas:

1. requester-side proving abstraction and unified gateways
2. proving networks, proof markets, and outsourced proving infrastructure
3. multi-zkVM abstraction layers and reusable verifier/prover interfaces

What is still comparatively rare is the exact layer `raiko2` is discussing:

- statement-specific provider intake
- fixture/public-input conformance
- benchmark comparability across heterogeneous providers
- explicit security and mutation gates before admission

That means the direction does not look imaginary. It looks early and somewhat under-specified in the
broader ecosystem.

## How To Read This List

Projects below are grouped into three buckets:

1. `most relevant`
   - closest to our platform problem
2. `adjacent infrastructure`
   - useful, but solving a narrower or shifted layer
3. `structural analogs`
   - not proving platforms, but useful for interface and sidecar design instincts

For each project, the questions worth asking are:

- what layer does it actually own
- what does it standardize
- what remains provider-specific
- how much of the integration model is static vs dynamic
- whether it has any conformance, benchmark, or admission machinery

## Summary Matrix

| Project | Category | Closest overlap with us | Initial take |
| --- | --- | --- | --- |
| ZkBoost | unified proving gateway | request-side adapter layer | very relevant |
| Brevis ProverNet | proving marketplace | heterogeneous proof jobs and multistage proving | very relevant |
| Boundless | proving market | outsourced proving + aggregation + settlement | relevant but RISC0-specific core |
| Gevulot / ZkCloud | proving infrastructure | shared prover layer / universal proving infra | relevant |
| Ere | zkVM abstraction toolkit | unified prover/verifier/compiler interfaces | relevant for abstractions, not intake |
| Aligned | verification / aggregation layer | multi-proof-system verification and batching | adjacent |
| Succinct network | prover network around SP1 | outsourced proving with a zkVM-centered stack | adjacent |
| Nexus network | distributed prover network | open prover network around own zkVM/L1 | adjacent |
| MEV-Boost / Commit-Boost | structural analog | sidecar and standardized third-party interface | analogy only |

## 1. Most Relevant

### 1. ZkBoost

What it is:

- a local proving client / gateway for proof requesters
- a unified interface in front of multiple proving services
- explicitly not a proof market, payment system, SLA system, or workflow orchestrator

Why it matters:

- it is the clearest public articulation of the fragmentation problem on the requester side
- it validates the idea that proof consumers do not want custom integrations per provider
- its "1-in-1-out" boundary is useful because it keeps provider internals behind an adapter

What to study:

- adapter model and who owns adapter maintenance
- provider discovery semantics
- how much normalization it attempts at the request and proof-result layer
- what it refuses to standardize

What it does not solve for us:

- provider conformance to a shared statement
- fixture/public-input checks
- benchmark comparability
- admission security policy

Why it is highly relevant:

- if ZkBoost is the requester-side gateway, our direction can be thought of as part of the missing
  policy and conformance layer behind a similar abstraction boundary

Sources:

- https://blog.zkcloud.com/p/zkboost-what-it-is-and-what-it-is
- https://blog.zkcloud.com/p/zkboost-proof-supply-chain-abstraction
- https://www.ankr.com/blog/ankr-leading-future-of-zk-proof-generation-with-zk-boost-consortium/

### 2. Brevis ProverNet

What it is:

- a decentralized marketplace for ZK proof generation
- explicitly designed for heterogeneous workloads and multistage proving pipelines
- not limited to a single zkVM proof shape

Why it matters:

- it is one of the strongest external signals that heterogeneous proof supply is a real problem
- unlike more single-stack proving networks, it openly talks about diverse proof types and
  composite proving flows
- that is closer to the world we expect than a single-backend prover network

What to study:

- workload taxonomy and job model
- whether provider capabilities are represented structurally or just by auction eligibility
- how multistage pipelines are modeled
- whether there is any notion of conformance beyond successful job fulfillment

Open question for us:

- does Brevis treat proof generation mostly as market matching, or does it also define a real
  provider contract and capability schema

Sources:

- https://provernet-docs.brevis.network/
- https://brevis.network/whitepaper/provernet.pdf
- https://blog.brevis.network/2025/11/17/brevis-provernet-the-open-marketplace-for-zero-knowledge-proofs/

### 3. Boundless

What it is:

- a decentralized proving market built around the RISC Zero stack
- requestors submit proof requests, provers bid, fulfill, and proofs are aggregated and settled
- includes distinct components such as Broker and Bento

Why it matters:

- it is a serious production attempt at outsourced proving with market mechanics
- it has explicit lifecycle documentation from request to fulfillment to proof use
- it shows what a mature proving market stack looks like operationally

Why it is not the full template for us:

- it is still grounded in the RISC Zero ecosystem
- it standardizes market interaction and fulfillment, but not our statement-level conformance model
- it cares more about request/fulfillment economics than provider admission semantics

What to study:

- request lifecycle
- proof type negotiation
- aggregation semantics
- broker/prover split
- where provider-specific logic actually leaks through

Sources:

- https://docs.boundless.network/developers/what
- https://docs.boundless.network/developers/proof-lifecycle
- https://docs.boundless.network/provers/proving-stack

### 4. Gevulot / ZkCloud

What it is:

- universal or shared proving infrastructure
- prover network and proving workload coordination
- broader "prove anything" infrastructure story

Why it matters:

- it is one of the clearest market signals that third-party proving supply is becoming its own
  infrastructure layer
- ZkBoost itself is strongly associated with this ecosystem

What to study:

- workload routing model
- capacity verification and liveness assumptions
- how much of the system is "run arbitrary proving jobs" vs "standardized provider contract"
- whether the request layer assumes a single proof model or heterogeneous proof families

Sources:

- https://docs.gevulot.com/gevulot-docs
- https://docs.gevulot.com/gevulot-docs/zkcloud-design/transactions
- https://docs.gevulot.com/gevulot-docs/zkcloud-design/execution-guarantees

## 2. Adjacent Infrastructure

### 5. Ere

What it is:

- a unified zkVM interface and toolkit
- common compiler / prover / verifier / platform abstractions across zkVMs
- currently supports multiple backends such as OpenVM, RISC0, SP1, and Zisk

What it proves:

- there is real value in a shared abstraction layer over multiple proving backends

What it does not prove:

- that runtime provider intake is solved
- that provider registry, conformance, or benchmark policy is solved

Important distinction:

- `ere` uses a static catalog and explicit enum-based backend wiring
- it is closer to a unified toolkit than to an open provider admission platform

What to study:

- clean trait boundaries
- artifact shapes for proving and verification
- fixture packaging for verifier-side tests

Sources:

- https://github.com/eth-act/ere

### 6. Aligned

What it is:

- a proof verification and aggregation layer
- supports multiple proof systems and provides proof submission plus verification services

Why it matters:

- it is evidence that multi-proof-system infrastructure is not theoretical
- it standardizes a verification-side interface across several ecosystems
- its aggregation service is a useful adjacent pattern

Why it is only adjacent:

- its center of gravity is verification and batching, not proving provider intake
- it is downstream from proof generation rather than upstream at provider admission time

What to study:

- supported-verifier model
- proof submission artifact shape
- how proof-system-specific data is normalized

Sources:

- https://docs.alignedlayer.com/
- https://docs.alignedlayer.com/architecture/0_supported_verifiers
- https://docs.alignedlayer.com/architecture/2_aggregation_mode

### 7. Succinct Prover Network

What it is:

- a proving network organized around SP1 and Succinct infrastructure

Why it matters:

- it is a signal that outsourced proving is becoming a first-class product category
- even if the stack is centered around SP1, it still reinforces demand for third-party proof supply

Why it is only adjacent:

- the ecosystem center is still a single proving stack rather than heterogeneous provider admission

What to study:

- how much of the API is stack-specific vs generic
- what proof/job abstraction is exposed to requesters
- whether they expose any useful benchmark or reliability surface

Sources:

- https://docs.succinct.xyz/

### 8. Nexus Network

What it is:

- a distributed prover network tightly coupled to the Nexus zkVM and broader Nexus system

Why it matters:

- strong evidence that large-scale distributed proving itself is viewed as a category
- useful as a data point for "proof demand to proof supply" coordination language

Why it is only adjacent:

- it is bound to its own zkVM and network vision
- not an obvious template for heterogeneous third-party provider intake into another protocol

What to study:

- task routing model
- contributor model
- whether there is any concept similar to capability advertisement or provider profiling

Sources:

- https://docs.nexus.xyz/network/overview/system-overview
- https://docs.nexus.xyz/zkvm/architecture
- https://blog.nexus.xyz/nexus-launches-worlds-first-open-prover-network/

## 3. Structural Analogs

### 9. MEV-Boost / Commit-Boost

Why they matter:

- not ZK proving projects
- but very useful for thinking about sidecars, gateway software, adapter boundaries, and
  third-party protocol integration without changing the core client every time

Why they are only analogs:

- they do not solve proof generation, verifier artifacts, or zk backend heterogeneity

What to study:

- sidecar boundary
- module ownership
- operational control by the user rather than a central hosted service

Sources:

- https://github.com/flashbots/mev-boost
- https://github.com/Commit-Boost/commit-boost-client

## Out Of Scope For This Investigation

These are relevant to the broader ecosystem but are not the main targets for this document:

- standalone zkVM projects such as OpenVM or ZisK by themselves
- generic proving APIs that do not address multi-provider abstraction
- purely hosted single-provider proving services

Those matter when evaluating possible backend integrations, but they are not the primary reference
set for an open prover platform.

## What This Suggests About Our Direction

### Evidence That The Demand Is Real

The ecosystem now clearly contains:

- proving gateways
- proving networks
- proof markets
- multi-proof-system verification infrastructure
- unified zkVM toolkits

That is enough to reject the idea that "nobody needs this category."

### What Still Looks Underserved

The narrower layer we are discussing still appears underbuilt:

- provider registration as a governed platform concept
- fixture/public-input conformance before provider admission
- benchmark comparability across providers proving the same statement
- explicit security and mutation gates for provider acceptance

This is the strongest reason to continue investigating.

### Updated Hypothesis

The current best hypothesis is:

- the general direction is validated by market activity
- but most existing projects stop at routing, market matching, or verification infrastructure
- there is still room for a statement-aware proving platform with stronger admission semantics

That does not prove the effort is commercially justified. It does suggest the design is not based on
an imaginary problem.

## Recommended Investigation Order

If we only go deep on a few, the order should be:

1. ZkBoost
2. Brevis ProverNet
3. Boundless
4. Gevulot / ZkCloud
5. Aligned
6. Succinct network
7. Ere
8. Nexus

Reasoning:

- the first four are closest to proof-supply abstraction or outsourced proving coordination
- Aligned is useful for multi-proof-system interface and aggregation thinking
- Ere is useful for abstraction hygiene
- Nexus is more useful as a market-signal datapoint than a direct architecture template

## Next Research Questions

The next pass should answer:

1. Which projects define a real provider capability schema rather than a loose adapter boundary?
2. Which projects support heterogeneous proof families rather than just multiple deployments of one
   proving stack?
3. Which projects expose benchmark, latency, or reliability surfaces in a reusable way?
4. Which projects define any admission or conformance gate beyond "the proof verified"?
5. Which projects leave orchestration to providers, and which normalize multistage proof workflows?
