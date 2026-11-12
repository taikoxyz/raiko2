# Node-observable block-count validation

Use the existing reviewed64-PGU candidate and frozen native/SP1/ELFb76 artifacts.
The primary quantity remains actual identical-proposal SP1 PGU divided by the literal
sum of node block estimates. Full-proposal quote and its separate deficit flag remain.
No new tariff, allocation policy, limit change, guest build, merge or deployment.

1. Reuse DUP16/POP and doubleSWAP16 validation runtimes with N0/N196608. Add one
   user CALL per block at B1/B4/B16/B32 using existing ProgramRequest. Preserve
   bytecode, per-user gas and minimal honest input construction; multiply total
   expected completed opcode counts byB. Do not repeat NUMBER/TIMESTAMP guards.
2. Preserve the four B1 canonical inputs/actuals as known references. Qualify the
   twelve new multi-block inputs independently, checking all user receipts/counts,
   outputs, per-block and aggregate reports. Reject wrong aggregate-count/OOG cases.
3. Freeze producer, request and input identities before actual sampling. Run native
   screening and sampling sequentially with existing guards. Retain failures and
   partial evidence; a resource-guard rejection is not a measured PGU value.
4. Recompute both estimate surfaces against actual. Compare N0/high at equalB and
   compareB growth separately, retaining literal deficits and both rounding rules.
   Separate any proposal-once remainder from scalable underpricing before drawing
   a node-closure conclusion. A fit from this validation panel would need a new
   explicitly declared candidate policy and untouched validation cohort.
5. Keep reusable producer/tests/usage in Raiko and archive verified data in a new
   Atlas revision. Obtain independent source review and native/actual verification.
