# Prediction-Market-Friendly Chain To Investigate

## Status

Draft research note.

## Purpose

This document records an early feasibility analysis for making an EVM-compatible Taiko chain more
friendly to prediction markets.

The current working conclusion is:

- the chain probably does not need consensus or EVM changes
- the useful work is likely a prediction-market stack on top of the chain
- the hard problems are market standards, oracle resolution, liquidity, indexing, UX, and policy

This is not a product spec. It is a list of areas worth exploring before deciding whether the
direction is strategically useful.

## Initial Feasibility

Technically feasible.

Prediction markets can be built on a normal EVM chain using contracts for:

- collateral custody
- outcome token issuance
- trading
- oracle resolution
- disputes
- settlement and redemption

The harder question is whether Taiko should provide ecosystem primitives and infrastructure that
make prediction markets easier to launch and safer to operate.

The likely answer is yes for infrastructure, but probably no for protocol-level special cases.

## What Should Not Be Changed First

Avoid chain-level changes in the first phase:

- do not change EVM semantics
- do not put real-world event resolution in consensus
- do not make validators arbitrate outcomes
- do not put compliance or geo policy into the L2 protocol
- do not create special prediction-market opcodes or precompiles before there is proven demand

An EVM-compatible chain already has enough expressiveness for the core market mechanics. Changing
the chain first would add complexity before the real bottlenecks are understood.

## What "Canonical Market" Means

`Canonical market` should not mean a protocol-enforced monopoly market.

It should mean a recommended, audited, and indexer-friendly market contract stack that the ecosystem
can recognize.

A canonical stack could include:

- `MarketFactory`
- `MarketRegistry`
- binary and multi-outcome market contracts
- outcome token contracts or conditional-token adapters
- collateral vaults
- oracle adapters
- dispute modules
- settlement and redemption modules
- standard metadata schema

The purpose is to avoid every application inventing incompatible formats for:

- market ids
- outcome ids
- collateral custody
- oracle binding
- dispute windows
- invalid market handling
- redemption
- market status indexing

Canonical means "standard and supported", not "the only possible implementation".

## Prediction Market Primitives

`Prediction market primitives` are the reusable building blocks needed by multiple applications.

They are not a full consumer application.

Candidate primitives:

- `MarketFactory`
  - creates markets from a standard schema
- `MarketRegistry`
  - gives wallets, indexers, and frontends one place to discover standard markets
- `OutcomeToken`
  - represents YES/NO or multi-outcome claims
- `CollateralVault`
  - locks collateral, mints complete outcome sets, and handles redemption
- `OracleAdapter`
  - connects UMA, Chainlink, custom resolvers, DAO resolvers, or onchain conditions
- `DisputeModule`
  - defines challenge windows, bonds, escalation, and invalid market behavior
- `Settlement`
  - finalizes outcomes and lets winning claims redeem collateral
- `MarketMetadata`
  - standardizes question text, resolution source, deadlines, categories, and risk flags
- `TradingModule`
  - supports orderbook settlement or specialized automated market making
- `Indexer`
  - exposes markets, order state, trades, positions, PnL, status, and resolution history

These primitives are the main place where a chain ecosystem can help without changing consensus.

## Liquidity And Market Making

Prediction markets need liquidity, but "AMM" should be used carefully.

Generic `x*y=k` token AMMs are a poor fit for binary prediction markets because YES and NO are
not independent assets:

- one complete YES + NO set is backed by one unit of collateral
- at resolution, one side is worth one and the other side is worth zero
- prices should usually respect the probability relationship between outcomes
- the market must remain collateralized for redemption

Using separate generic pools such as `YES/USDC` and `NO/USDC` can create bad states:

- YES and NO prices may not sum to one
- liquidity provider risk becomes hard to reason about
- collateral conservation is no longer obvious
- arbitrage and UX become confusing

More appropriate market-making options:

- CLOB / offchain orderbook with onchain settlement
- LMSR-style cost-function market maker
- FPMM-style market maker over a complete outcome set
- managed market maker or house liquidity for curated markets

Recommended starting position:

- use orderbook-first design for important markets
- optionally support specialized prediction-market AMMs for long-tail or cold-start markets
- do not treat a generic DEX AMM as the canonical trading model

## Oracle And Resolution

Oracle resolution is the core product risk.

Every market needs a precise resolution contract:

- what source decides the outcome
- when the outcome is evaluated
- what exact value format is accepted
- what happens if the source is unavailable or ambiguous
- who can propose the result
- who can dispute the result
- what bond is required
- who has final authority after escalation
- whether invalid/refund is possible

Candidate oracle paths:

- onchain condition oracle
  - best for crypto-native markets
- optimistic oracle
  - useful when outcomes are objective but not directly onchain
- committee or DAO resolver
  - useful for curated or governance-linked markets
- external API adapter
  - high UX value but needs careful trust and fallback policy
- hybrid escalation
  - fast optimistic result with human/DAO fallback

The first market set should prefer objective and crypto-native outcomes.

Examples:

- governance proposal passes before a deadline
- token price crosses a threshold according to a specified oracle
- protocol TVL or onchain metric reaches a threshold
- a named contract emits a specific event
- a block or state transition condition occurs

Political, sports, celebrity, and legal outcomes should be deferred until the policy and resolution
model is mature.

## Compliance And Policy

Prediction markets can become regulated event-contract or binary-option products depending on
jurisdiction, market type, operator role, and user access.

This document is not legal advice. It does mean the architecture should avoid mixing policy into
the L2 protocol itself.

Policy should be handled at the application/operator layer:

- curated market lists
- category restrictions
- jurisdiction flags
- creator allowlists or creator bonds
- market removal policy
- frontend access rules
- oracle and dispute policy disclosure

The chain should provide neutral primitives. Applications and operators should decide which markets
they are willing to list and serve.

## User Experience Requirements

A prediction market chain experience needs low-friction trading:

- stablecoin collateral
- cheap deposits and withdrawals
- fast bridge path
- gas sponsorship or account abstraction
- session keys for repeated trading
- clear portfolio and PnL
- realtime orderbook or market data
- websocket APIs
- simple redemption flow

Without these, chain-level support does not matter much.

## Suggested MVP

Start with crypto-native binary markets.

Scope:

- one collateral asset, preferably a stablecoin
- binary markets only
- standard market registry
- standard market metadata
- outcome token or conditional-token adapter
- collateral vault
- one oracle adapter for onchain or objective crypto data
- one dispute path
- orderbook-first trading path
- optional specialized AMM only for curated long-tail markets
- indexer and API for market status, trades, positions, and settlement

Explicitly exclude:

- permissionless global market creation with no policy
- sports, elections, celebrity, and legal markets
- protocol-level resolution
- generic Uniswap-style YES/USDC and NO/USDC pools as canonical market makers

## Strategic Questions

Before investing deeply, answer:

1. Is the goal to attract third-party prediction market apps, or to build a first-party product?
2. Which market categories are acceptable for a Taiko-supported stack?
3. Is the initial collateral native, bridged stablecoin, or app-specific credit?
4. Is Taiko willing to operate or sponsor oracle/dispute infrastructure?
5. Is the product orderbook-first, AMM-first, or hybrid?
6. Who supplies initial liquidity?
7. What is the path from market creation to listing in frontends and indexers?
8. How are invalid or ambiguous outcomes handled?
9. What pieces should be protocol-neutral primitives vs application policy?
10. What would make a third-party app choose Taiko over an existing venue?

## Possible Architecture

```text
Market Creator
  -> MarketFactory
  -> MarketRegistry
  -> CollateralVault
  -> OutcomeToken
  -> TradingModule
       -> CLOB settlement
       -> optional LMSR/FPMM module
  -> OracleAdapter
  -> DisputeModule
  -> Settlement
  -> Indexer/API
  -> Frontends and market makers
```

The L2 provides cheap execution and final settlement. The application stack provides market logic,
resolution, and user-facing policy.

## Why This Might Be Worth Doing

Reasons this direction may be real:

- prediction markets are a high-frequency consumer use case
- they need cheap execution and frequent order updates
- they benefit from strong composability with stablecoins and DeFi liquidity
- they need standardized market data and settlement contracts
- most current implementations are application-specific rather than chain-level ecosystem
  primitives

## Why This Might Not Be Worth Doing

Reasons to be skeptical:

- the bottleneck may be regulation and distribution, not chain infrastructure
- liquidity is hard to bootstrap
- oracle disputes can dominate engineering and operations
- existing prediction market products may not want to migrate
- a general EVM chain may not gain much by branding around one regulated vertical

## Current Working Recommendation

Do not turn Taiko into a special prediction-market chain at the protocol level.

Explore a prediction-market-friendly stack:

- standard contracts
- standard market metadata
- oracle and dispute adapters
- indexer/API
- gas-sponsored UX
- liquidity bootstrapping tools

The first serious investigation should focus on:

1. oracle/resolution design
2. market contract standard
3. trading design: orderbook vs specialized AMM
4. compliance and operator policy boundary
5. indexer and UX requirements

The strongest near-term experiment is a curated set of crypto-native markets where outcomes can be
resolved objectively from chain data or well-defined oracle feeds.
