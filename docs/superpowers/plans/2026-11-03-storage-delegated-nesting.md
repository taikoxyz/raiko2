# Delegated Storage Nesting Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Verify shared code cannot collapse distinct delegated storage work in node pricing.
**Architecture:** Default-preserving common CALL/F4 helper, new16-case shared-code wrapper and bounded dual-context test interpreter; independently qualified/frozen observations and records.
**Tech Stack:** Managed Python/stdlib and unchanged native/quote/SDK6.3/ELFb76.
**Spec:** `docs/superpowers/specs/2026-11-03-storage-delegated-nesting.md`.

## Global Constraints

-16new STOP/REVERT×K16/1024×depth1/4×zero/full;4topstorageowners share1leafcodeaddress,4depth+1configuredaccounts.16oldCALLrefsnotnew.
- Sharedkeyword-only call_kind='call' default preserves16priorrequests/runtime/coremetadata; delegatecall variant changes F1→F4/6args/pre18,cold2600/post46/2664level. No duplicate kernel or bytecode patch.
- All4topmaps are IDENTICAL (phase0=65537,body65537..65537+K-1=1),sharing1populatedroot;storagecontext follows top throughallF4frames;codeowner/sharedleafempty,intermediateempty,topmaps originalphase/body. Actualcode/storageaddresses explicit,4(K+1)access/4Kwrites—notK+1bysharedcode.
- Childintrinsicremoved/userrefundcaponce;leafbefore2800K/usereffective0onFD;EIP150recurrence2618+max(ceil64T/63,T+46),leafT=C+2301;31Mheader/30Mcharged/Anchor167355/15Muser/allprefixsentry/caps/maxRtruthful.
- Keepmodelc893/native/quote/SDK6.3/ELFb76/Rust/guest/pinsunchanged,nobuild/price/merge/deploy/timer.16allgatesbefore576MiBactuals,6GiB/600s,retainedfailures/noimplicitretry.
-40diagnostics explicitcontextownerhashset/envelope markers andexactscaledvalues;literalnode/complete/actualseparate,2B/30Mpriorityrule,finiteclaims.

## Review Focus

- Codeaddress vsstoragecontext/cardinality collapse: Task1 dualcontext/sharedleaf tests+Task2auth/counters4owners.
- CALLdefault contamination/sharedhelperregression: Task1 frozenprior16request/runtime/fullcoremetadatagoldens+oldfocusedtests.
- F4sixstackargs/warmtarget/EIP150/refund: Task1 boundary/fullR+1/negative cases+Task2allnative/gas/receipts.
- Stale leaf-keyed phase/maps/rolefields: Task1 topowner map/schedule/emptycodeowner asserts+Task2wholeworld proofs.
- OldCALLrefs attribution/recount: Task2 exact16identityjoins/40flags/canonicaldedup andseparateglobal/codeaccountsupport.

### Task 1: Common F4 support, wrapper and regressions

**Files:** modify `bin/zkgas/bench/cases/storage_caught_nesting.py`,`bin/zkgas/bench/tests/storage_caught_vm.py`;create `bin/zkgas/bench/cases/storage_delegated_nesting.py`,`bin/zkgas/bench/tests/test_storage_delegated_nesting.py`;appendREADME;include suppliedspec/plan (7files). Do not edit dirty leaf or otheroldhelpers.
**Interfaces:** shared_wrapper/_wrapper_gas/gas_accounting/fits/full_rounds/_make_case/make_case gain default-preserving keywordcall_kind; newwrappermake_case/prepare/CLI per spec. VMrun_users interpretsactualF1/F4 code/storagecontext withoutproduceroracle;oldbehavior preserved.

- [x] Captureoriginal16CALLimmutablegoldens fromcompletedreviewedcatalog;write failing F4matrix/context/gas/refund/EIP150/strict/negative tests and runRED.
- [x] Minimal single-authority sharedhelper/F4wrapper/dualcontextVM implementation;all16byte trajectories andmetadata roles/maps/counters correct.
- [x] Allfull/R+1 andfeasiblebatch boundaries;wrongcontext/target/value/marker/status/childgas negatives rejected. Twofrozen-nativepositive+twoacceptedmutants, no builds.
- [x] RunnewGREEN+oldCALL/dirty/keyshift/journal focusedregressions;inspect7-filecompleteaddedlinehygiene/diffcheck;commitConventionalCommit,nopush.

### Task 2: Frozen delegated observations and records

**Files:** create `docs/research/2026-11-03-storage-delegated-nesting.md`;updatecoverage;appenduserworking.log. Newcampaign roots only;old16CALLreceiptsimmutable.
**Interfaces:** reviewed16catalog+old16referenceidentities -> independentlyqualifiedcontexts/world/roots/nativequotes/counters -> frozen16actuals ->40diagnostics/rawaudit/cohort/host/privatearchive/docs -> finalreview/ownDraft296readback.

- [ ] Independent source review and16allnative/auth/receipts/signedroots/witness/quotes/context-specific counters;bind16oldCALLwithoutreexecution.
- [ ] Freeze all16inputs/tools/quotes/source/resourcepolicy;sequentialactualonce,allfailures retained.
- [ ] Independentlyaudit16raw/40differences/contextsupportflags/exactceilings/oldrefjoins/cohortuniqueIDs/hostresources.
- [ ] Verifyfinitefindings/limits/usage/identities/private recovery/doclinks/hygiene;docscommitafterdatagate.
- [ ] Freshfinalwholeincrementreview;ownDraft296FFpush/body/readback,keepdo-not-merge;appendverifiedconclusionandnextstage,nohumancheckpoint.
