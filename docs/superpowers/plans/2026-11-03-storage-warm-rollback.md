# Warm Storage Rollback Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Measure caller-prewarmed storage and ancestor rollback under frozen prices.
**Architecture:** Unchanged leaf, common call emitter, new guarded top-read/ancestor-exit producer and separate checkpoint-aware frame oracle;32qualified frozen observations plus64matched diagnostics.
**Tech Stack:** Managed Python/stdlib, unchanged native/quote/SDK6.3/ELFb76.
**Spec:** `docs/superpowers/specs/2026-11-03-storage-warm-rollback.md`.

## Global Constraints

-32new exit leaf|ancestor×K16|256×depth2|4×prewarmbool×zero/full,4topowners/commonmap/1leafcode,4depth+1accounts. Original32CALL/F4goldens unchanged.
- Existingwrapperrefactorto_emit_call only;testleafexecute initial_warm_keys keywordonlydefaultNone;nonewoldVMbehavior/defaultshape. Newcheckpoint-awareoracle separatefile;no duplicateleaf/callbuilder.
- TOPoptionalprewarm+mandatorypostverifyphase/body,leaf100vs2100coldfees/post100iffprewarm;account/storagewarmsets/state/refundrollback perframe,attemptedcountersretained. Globalrefund0,userbefore0,leaf/ancestorbeforeexplicit.
- F4pre18/cold2600/post46,topreadloops/memory32bytes/leaf192,ancestorFD6vsleafFD6samegross. Modepaircommonmaxenvelope/commonfullR,31Mheader/30MchargedAnchor167355/15Muser/allprefix/sentry/caps/R+1truthful.
- Model/Rust/guest/SDK/ELF/pinsunchanged,nobuild/price/merge/deploy/timer. Source reviewpass before32native/quotes;allgatesbefore32actualfreeze,6GiB/600s/576MiBdisk/log16/output192,retainedfailures/noimplicitretry.
-64exactscaleddiagnostics+5flags,literalnode/full/actualseparate,2B/30Mpriority;historical204readonly,nooldremeasure/recount,no globalclaim.

## Review Focus

- Childcommit thenancestorFD mustundo warmth/refund/state butretainwork:Task1 checkpointnegatives/literalstreams,Task2allnative/counters.
- Parentprewarm surviveslowerFD whereaschildwarmthdoesnot:Task1 readcoldflags/gas+Task2receipt/EIP150checks.
- Leaf/user/ancestorrefundscopes:Task1 explicitmodepairedasserts andTask2originalreportroles,alluserrefund0.
- Sharedrefactoreddefaultdrift:Task1 immutableold32request/runtime/coregoldens andfocusedoldtests.
- Gasbudget/controlfeasibility:K256not1024,commonenvelopeandfullR/allprefix/R+1,Task1concreteVM+Task2all32native/quotes.

### Task 1: Reusable warm/rollback producer and regressions

**Files:** modify `bin/zkgas/bench/cases/storage_caught_nesting.py`(extract_emit_call),`bin/zkgas/bench/tests/test_storage_mutation.py`(optionalwarmkeys);create `bin/zkgas/bench/cases/storage_warm_rollback.py`,`bin/zkgas/bench/tests/storage_warm_rollback_vm.py`,`bin/zkgas/bench/tests/test_storage_warm_rollback.py`;appendREADME;include suppliedspec/plan (8files). Existingleaf/currentCALL-F4VM untouched.
**Interfaces:** _emit_call(asm,target,expected_status,*,call_kind='call');execute(code,state,envelope,*,initial_warm_keys=None);newmake_case/gas_accounting/fits/full_rounds/prepare/CLI exactspec. Source labelsfailure definedbycalleronce.

- [ ] Captureimmutable32oldCALL/F4core goldens;RED newdomain/matrix/literalwarmness/refund/checkpoint/pairedgas testsbeforeproducer.
- [ ] Minimalsharedemitter/defaultwarmarg/newproducer/independentboundedframeoracle;all32literaltrajectories/route/metadata/maps/commongas/envelopes/maxR checks.
- [ ] Boundaries/fullR+1/negativecontext/warmness/value/terminal/gas;fourfrozennativepositive/acceptedmutants,no builds.
- [ ] NewGREEN+oldCALL/F4/dirty/keyshift/mutation/journalfocusedtests;addedlinepathhygiene/full8filediffcheck;commitConventionalCommit,nopush.

### Task 2: Frozen observations and record

**Files:** create `docs/research/2026-11-03-storage-warm-rollback.md`;updatecoverage;appenduserworking.log. NewQ/R/privatearchiveonly;old204dataimmutable.
**Interfaces:** reviewed32catalog -> independentlyqualified32actualframe/native/world/roots/witness/quotes/counters -> frozen32SP1 ->64diagnostics ->rawaudit/cohort/host128/archive/docs ->finalreview/Draft296/readback/log.

- [ ] Independent sourcereview,then32native/signatures128/allcontext/framegas/warmrefund/roots/quotes/counterchecks;nonewcanonicalmeasureuntilsourcefinal.
- [ ] Freeze32inputs/quotes/tools/source/resources thenactualsoncesequential;retainallfailures/noimplicitretry.
- [ ] Independentlyaudit32raw/64scaleddiagnostics/5flags/integerconversion/nodeceil/fullseparate/cohortdedup/host128closure.
- [ ] Recordfinitefindings/limits/usage/hashes/private recovery,verifyrows/links/hygiene,docscommitafterdatagate.
- [ ] Freshfinalwholeincrementreview;ownDraft296FFpush/body/readbackkeepdo-not-merge;verifiedno-priceconclusiondirectworking.log,no user checkpoint/roundnotification.
