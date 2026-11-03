# LOG data, topics and receipt workloads

All 30 new execute-only observations completed once without failure or retry: ten own-zero controls, ten singles and ten maximal-uniform-full points across LOG1/LOG4 and five length/pattern domains. Twenty literal node gaps remain (ten controls and ten singles), with maximum 153,502,123 PGU. No full point has a node gap, no input has a complete-proposal gap and none crosses the frozen >2B actual-minus-node threshold within 30M charged gas. Maximum actual is 963,138,003 PGU. All ten full points are near-full. Full node/actual ratios are 1.07526566894–80.6074581853; complete-proposal/actual ratios are 1.28254606002–80.7760953309. Prices remain unchanged.

All 30 own-R increments are covered by both exact scaled estimates. Six of 12 signed tail contrasts and four of 15 topic-count contrasts remain uncovered, with maximum upward-rounded gaps 6,078 and 485,453 PGU respectively. Absolute proposal coverage does not erase these incremental TODOs. These are whole-input contrasts, not isolated LOG or receipt costs.

Persistent-storage 236, transient-storage 8, pairing 18, [default SHA-256 18](2026-11-04-sha256-bulk.md), [linear-regime SHA-256 18](2026-11-04-sha256-linear-regime.md) and new LOG 30 remain separate read-only cohorts. Historical actuals are neither rerun nor counted in a new union. See the [coverage inventory](zkgas-workload-coverage.md).

## Exact inputs and finite gas domain

The [producer](../../bin/zkgas/bench/cases/log_receipt_bulk.py) emits `unzen-log-receipt-bulk-20261104`: topics N=1/4 × (L0/zero, L64/zero, L64/a5, L262144/zero, L262144/a5) × own-zero/single/maximal-uniform-full. L0/a5 is rejected because empty data has no tail pattern. Two signed users call distinct authenticated code owners with fixed-width tags 0/1 and empty persistent storage. Every user receipt succeeds and terminates STOP; outer calldata is empty, with no storage instructions, calls, precompiles, refunds, creation or REVERT.

Counter and last-input guards use memory offsets 0/32, and payload begins at 64. Allocate `max(32,L)` once, seed and logarithmically MCOPY for both patterns even at R0; memory occupies `2+max(32,L)/32` words. Each iteration i=0..R-1 writes `BE256((tag<<128)|i)` as the first payload word. Nonempty log data is that word followed by `pattern_byte*(L-32)`; L0 log data is empty. Topic j is `BE256((tag<<128)|(j<<64)|i)`, j=0..N-1, pushed in reverse so topic0 is popped first. Exact ordered address/topic/data intent is independently interpreted from actual bytecode; uniqueness is within a proposal, not across its zero/single/full observations. The compact sequence SHA256 is a debug intent commitment, not a protocol receipt root.

Complete EVM LOG gas is `375+375*N+8*L` per event. The dynamic topic/data cost excludes static 375, which is charged exactly once alongside ordinary opcodes, intrinsic 21000, memory `3*w+w*w//512` and MCOPY `3+3*ceil(n/32)`. No EIP-150 forwarding or SSTORE sentry applies. The signed envelope adds a 1000-gas fixture margin to gross gas; zero/single retain matching full envelopes. The `evm-gas-only` profile does not apply default legacy admission pricing.

| Topics N | Tail | Bytes L | Full R/user | Charged gas/user | Signed envelope/user | Charged block gas | Occupancy |
| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | zero | 0 | 17462 | 14916214 | 14917214 | 29999783 | 99.999277% |
| 1 | a5 | 64 | 10912 | 14916026 | 14917026 | 29999407 | 99.998023% |
| 1 | zero | 64 | 10912 | 14916026 | 14917026 | 29999407 | 99.998023% |
| 1 | a5 | 262144 | 7 | 14887601 | 14888601 | 29942557 | 99.808523% |
| 1 | zero | 262144 | 7 | 14887601 | 14888601 | 29942557 | 99.808523% |
| 4 | zero | 0 | 7395 | 14914658 | 14915658 | 29996671 | 99.988903% |
| 4 | a5 | 64 | 5896 | 14914442 | 14915442 | 29996239 | 99.987463% |
| 4 | zero | 64 | 5896 | 14914442 | 14915442 | 29996239 | 99.987463% |
| 4 | a5 | 262144 | 7 | 14895728 | 14896728 | 29958811 | 99.862703% |
| 4 | zero | 262144 | 7 | 14895728 | 14896728 | 29958811 | 99.862703% |

Anchor contributes 167355 gas. The bounded solver enforces <=30M charged block gas, <=15M/user fixture cap, separate 16,777,216 EIP-7825 cap, a 31M header, every ordered admission prefix and 24576-byte code cap. Independent full+1 checks execute with sufficient interpreter gas before applying candidate constraints. All ten selected full points charge 29,942,557–29,999,783 gas and exceed the >=29.7M near-full threshold. Maximal uniform R does not establish a global packing maximum.

## Literal same-proposal results

Actual is entire-proposal SDK 6.3 `ExecutionReport.gas * 10 // 191`. Literal node PGU sums known-PGU per-block ceilings (one block here). Complete-proposal PGU separately includes proposal/input terms. Raw execution gas is not compared directly with PGU. Positive gaps are `max(actual-estimate,0)`; all complete-proposal gaps are zero.

| Topics N | Tail | Bytes L | R/user | Charged block gas | Actual PGU | Literal node PGU | Complete proposal PGU | Positive node gap |
| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | zero | 0 | 0 | 209611 | 161262768 | 7760645 | 162753731 | 153502123 |
| 1 | zero | 0 | 1 | 211317 | 161307981 | 7816366 | 162809453 | 153491615 |
| 1 | zero | 0 | 17462 | 29999783 | 963138003 | 1080275765 | 1235268851 | 0 |
| 1 | a5 | 64 | 0 | 209647 | 161258644 | 7763360 | 162756447 | 153495284 |
| 1 | a5 | 64 | 1 | 212377 | 161304445 | 7836178 | 162829265 | 153468267 |
| 1 | a5 | 64 | 10912 | 29999407 | 746194112 | 802356911 | 957349997 | 0 |
| 1 | zero | 64 | 0 | 209647 | 161258180 | 7763360 | 162756447 | 153494820 |
| 1 | zero | 64 | 1 | 212377 | 161313937 | 7836178 | 162829265 | 153477759 |
| 1 | zero | 64 | 10912 | 29999407 | 746193951 | 802356911 | 957349997 | 0 |
| 1 | a5 | 262144 | 0 | 570487 | 161672142 | 9431543 | 164424630 | 152240599 |
| 1 | a5 | 262144 | 1 | 4766497 | 178444266 | 110143082 | 265136168 | 68301184 |
| 1 | a5 | 262144 | 7 | 29942557 | 296890301 | 714412312 | 869405398 | 0 |
| 1 | zero | 262144 | 0 | 570487 | 161672267 | 9431543 | 164424630 | 152240724 |
| 1 | zero | 262144 | 1 | 4766497 | 178444397 | 110143082 | 265136168 | 68301315 |
| 1 | zero | 262144 | 7 | 29942557 | 296890198 | 714412312 | 869405398 | 0 |
| 4 | zero | 0 | 0 | 209611 | 161267832 | 7775621 | 162768707 | 153492211 |
| 4 | zero | 0 | 1 | 213639 | 161365401 | 17792918 | 172786004 | 143572483 |
| 4 | zero | 0 | 7395 | 29996671 | 919092205 | 74085686483 | 74240679569 | 0 |
| 4 | a5 | 64 | 0 | 209647 | 161277320 | 7778336 | 162771423 | 153498984 |
| 4 | a5 | 64 | 1 | 214699 | 161380710 | 17795633 | 172788720 | 143585077 |
| 4 | a5 | 64 | 5896 | 29996239 | 815513129 | 59069761087 | 59224754174 | 0 |
| 4 | zero | 64 | 0 | 209647 | 161277142 | 7778336 | 162771423 | 153498806 |
| 4 | zero | 64 | 1 | 214699 | 161374632 | 17795633 | 172788720 | 143578999 |
| 4 | zero | 64 | 5896 | 29996239 | 815513063 | 59069761087 | 59224754174 | 0 |
| 4 | a5 | 262144 | 0 | 570487 | 161680416 | 9446519 | 164439606 | 152233897 |
| 4 | a5 | 262144 | 1 | 4768819 | 179000561 | 110217112 | 265210199 | 68783449 |
| 4 | a5 | 262144 | 7 | 29958811 | 297291373 | 714840670 | 869833757 | 0 |
| 4 | zero | 262144 | 0 | 570487 | 161680539 | 9446519 | 164439606 | 152234020 |
| 4 | zero | 262144 | 1 | 4768819 | 179003880 | 110217112 | 265210199 | 68786768 |
| 4 | zero | 262144 | 7 | 29958811 | 297293662 | 714840670 | 869833757 | 0 |

LOG1/L64/a5/full is the closest full node ratio: 746,194,112 actual against 802,356,911 node and 957,349,997 proposal PGU (about 1.075× node/actual). LOG1/L0/full reaches the maximum actual 963,138,003 PGU, against 1,080,275,765 node and 1,235,268,851 proposal. LOG4/L0/full is heavily overquoted: 919,092,205 actual against 74,085,686,483 node and 74,240,679,569 proposal. The 256KiB full points are about 297M actual versus 714M node. These conservative synthetic quotes do not certify ordinary admission usability or select an admission limit.

## Whole-proposal contrasts and quote interpretation

The 57 retained diagnostics comprise ten zero→single, ten zero→full, ten single→full, 12 a5-minus-zero and 15 LOG4-minus-LOG1 contrasts. They add no requests or actual executions. Every signed actual difference and exact scaled estimate difference is retained below. `N/10^18` is the signed scaled numerator over the frozen scale before ceiling; comparisons use those rationals, not differences of rounded estimates.

Flags are `R/N/P/L/G/E/S`: equal repetitions / topic count / tail pattern / log-data length / charged block gas / complete signed user-transaction envelopes / authenticated selected persistent-storage support. True counts are 24/42/45/57/17/42/57. L compares length, not data equality. E compares the complete signed user transactions, not just gas limits; differing runtime/code and complete GuestInput remain separate. Topic contrasts can change full R, gas and signed envelopes; false flags are retained. S compares exactly two authenticated owners, each mapped to an empty storage-node set with SHA256(empty). Empty selected support does not remove account/code/global witness work.

| Contrast (to minus from) | Actual Δ PGU | Node Δ PGU (exact) | Proposal Δ PGU (exact) | R/N/P/L/G/E/S |
| --- | ---: | ---: | ---: | --- |
| own_zero_to_single: log1/a5/l262144/n0 → log1/a5/l262144/n1 | 16772124 | 100711538323905892568411882/10^18 | 100711538323905892568411882/10^18 | F/T/T/T/F/T/T |
| own_zero_to_single: log1/a5/l64/n0 → log1/a5/l64/n1 | 45801 | 72818323905892568411882/10^18 | 72818323905892568411882/10^18 | F/T/T/T/F/T/T |
| own_zero_to_single: log1/zero/l0/n0 → log1/zero/l0/n1 | 45213 | 55721248559967028994024/10^18 | 55721248559967028994024/10^18 | F/T/T/T/F/T/T |
| own_zero_to_single: log1/zero/l262144/n0 → log1/zero/l262144/n1 | 16772130 | 100711538323905892568411882/10^18 | 100711538323905892568411882/10^18 | F/T/T/T/F/T/T |
| own_zero_to_single: log1/zero/l64/n0 → log1/zero/l64/n1 | 55757 | 72818323905892568411882/10^18 | 72818323905892568411882/10^18 | F/T/T/T/F/T/T |
| own_zero_to_single: log4/a5/l262144/n0 → log4/a5/l262144/n1 | 17320145 | 100770592938749491032634424/10^18 | 100770592938749491032634424/10^18 | F/T/T/T/F/T/T |
| own_zero_to_single: log4/a5/l64/n0 → log4/a5/l64/n1 | 103390 | 10017296938749491032634424/10^18 | 10017296938749491032634424/10^18 | F/T/T/T/F/T/T |
| own_zero_to_single: log4/zero/l0/n0 → log4/zero/l0/n1 | 97569 | 10017296938749491032634424/10^18 | 10017296938749491032634424/10^18 | F/T/T/T/F/T/T |
| own_zero_to_single: log4/zero/l262144/n0 → log4/zero/l262144/n1 | 17323341 | 100770592938749491032634424/10^18 | 100770592938749491032634424/10^18 | F/T/T/T/F/T/T |
| own_zero_to_single: log4/zero/l64/n0 → log4/zero/l64/n1 | 97490 | 10017296938749491032634424/10^18 | 10017296938749491032634424/10^18 | F/T/T/T/F/T/T |
| own_zero_to_full: log1/a5/l262144/n0 → log1/a5/l262144/n7 | 135218159 | 704980768267341247978883174/10^18 | 704980768267341247978883174/10^18 | F/T/T/T/F/T/T |
| own_zero_to_full: log1/a5/l64/n0 → log1/a5/l64/n10912 | 584935468 | 794593550461099706510456384/10^18 | 794593550461099706510456384/10^18 | F/T/T/T/F/T/T |
| own_zero_to_full: log1/zero/l0/n0 → log1/zero/l0/n17462 | 801875235 | 1072515119892546208218719934/10^18 | 1072515119892546208218719934/10^18 | F/T/T/T/F/T/T |
| own_zero_to_full: log1/zero/l262144/n0 → log1/zero/l262144/n7 | 135217931 | 704980768267341247978883174/10^18 | 704980768267341247978883174/10^18 | F/T/T/T/F/T/T |
| own_zero_to_full: log1/zero/l64/n0 → log1/zero/l64/n10912 | 584935771 | 794593550461099706510456384/10^18 | 794593550461099706510456384/10^18 | F/T/T/T/F/T/T |
| own_zero_to_full: log4/a5/l262144/n0 → log4/a5/l262144/n7 | 135610957 | 705394150571246437228440968/10^18 | 705394150571246437228440968/10^18 | F/T/T/T/F/T/T |
| own_zero_to_full: log4/a5/l64/n0 → log4/a5/l64/n5896 | 654235809 | 59061982750866999128412563904/10^18 | 59061982750866999128412563904/10^18 | F/T/T/T/F/T/T |
| own_zero_to_full: log4/zero/l0/n0 → log4/zero/l0/n7395 | 757824373 | 74077910862052486186331565480/10^18 | 74077910862052486186331565480/10^18 | F/T/T/T/F/T/T |
| own_zero_to_full: log4/zero/l262144/n0 → log4/zero/l262144/n7 | 135613123 | 705394150571246437228440968/10^18 | 705394150571246437228440968/10^18 | F/T/T/T/F/T/T |
| own_zero_to_full: log4/zero/l64/n0 → log4/zero/l64/n5896 | 654235921 | 59061982750866999128412563904/10^18 | 59061982750866999128412563904/10^18 | F/T/T/T/F/T/T |
| single_to_full: log1/a5/l262144/n1 → log1/a5/l262144/n7 | 118446035 | 604269229943435355410471292/10^18 | 604269229943435355410471292/10^18 | F/T/T/T/F/T/T |
| single_to_full: log1/a5/l64/n1 → log1/a5/l64/n10912 | 584889667 | 794520732137193813942044502/10^18 | 794520732137193813942044502/10^18 | F/T/T/T/F/T/T |
| single_to_full: log1/zero/l0/n1 → log1/zero/l0/n17462 | 801830022 | 1072459398643986241189725910/10^18 | 1072459398643986241189725910/10^18 | F/T/T/T/F/T/T |
| single_to_full: log1/zero/l262144/n1 → log1/zero/l262144/n7 | 118445801 | 604269229943435355410471292/10^18 | 604269229943435355410471292/10^18 | F/T/T/T/F/T/T |
| single_to_full: log1/zero/l64/n1 → log1/zero/l64/n10912 | 584880014 | 794520732137193813942044502/10^18 | 794520732137193813942044502/10^18 | F/T/T/T/F/T/T |
| single_to_full: log4/a5/l262144/n1 → log4/a5/l262144/n7 | 118290812 | 604623557632496946195806544/10^18 | 604623557632496946195806544/10^18 | F/T/T/T/F/T/T |
| single_to_full: log4/a5/l64/n1 → log4/a5/l64/n5896 | 654132419 | 59051965453928249637379929480/10^18 | 59051965453928249637379929480/10^18 | F/T/T/T/F/T/T |
| single_to_full: log4/zero/l0/n1 → log4/zero/l0/n7395 | 757726804 | 74067893565113736695298931056/10^18 | 74067893565113736695298931056/10^18 | F/T/T/T/F/T/T |
| single_to_full: log4/zero/l262144/n1 → log4/zero/l262144/n7 | 118289782 | 604623557632496946195806544/10^18 | 604623557632496946195806544/10^18 | F/T/T/T/F/T/T |
| single_to_full: log4/zero/l64/n1 → log4/zero/l64/n5896 | 654138431 | 59051965453928249637379929480/10^18 | 59051965453928249637379929480/10^18 | F/T/T/T/F/T/T |
| tail_pattern: log1/zero/l262144/n0 → log1/a5/l262144/n0 | -125 | 0/10^18 | 0/10^18 | T/T/F/T/T/T/T |
| tail_pattern: log1/zero/l262144/n1 → log1/a5/l262144/n1 | -131 | 0/10^18 | 0/10^18 | T/T/F/T/T/T/T |
| tail_pattern: log1/zero/l262144/n7 → log1/a5/l262144/n7 | 103 | 0/10^18 | 0/10^18 | T/T/F/T/T/T/T |
| tail_pattern: log1/zero/l64/n0 → log1/a5/l64/n0 | 464 | 0/10^18 | 0/10^18 | T/T/F/T/T/T/T |
| tail_pattern: log1/zero/l64/n1 → log1/a5/l64/n1 | -9492 | 0/10^18 | 0/10^18 | T/T/F/T/T/T/T |
| tail_pattern: log1/zero/l64/n10912 → log1/a5/l64/n10912 | 161 | 0/10^18 | 0/10^18 | T/T/F/T/T/T/T |
| tail_pattern: log4/zero/l262144/n0 → log4/a5/l262144/n0 | -123 | 0/10^18 | 0/10^18 | T/T/F/T/T/T/T |
| tail_pattern: log4/zero/l262144/n1 → log4/a5/l262144/n1 | -3319 | 0/10^18 | 0/10^18 | T/T/F/T/T/T/T |
| tail_pattern: log4/zero/l262144/n7 → log4/a5/l262144/n7 | -2289 | 0/10^18 | 0/10^18 | T/T/F/T/T/T/T |
| tail_pattern: log4/zero/l64/n0 → log4/a5/l64/n0 | 178 | 0/10^18 | 0/10^18 | T/T/F/T/T/T/T |
| tail_pattern: log4/zero/l64/n1 → log4/a5/l64/n1 | 6078 | 0/10^18 | 0/10^18 | T/T/F/T/T/T/T |
| tail_pattern: log4/zero/l64/n5896 → log4/a5/l64/n5896 | 66 | 0/10^18 | 0/10^18 | T/T/F/T/T/T/T |
| topic_count: log1/a5/l262144/n0 → log4/a5/l262144/n0 | 8274 | 14976000000000000000000/10^18 | 14976000000000000000000/10^18 | T/F/T/T/T/F/T |
| topic_count: log1/a5/l262144/n1 → log4/a5/l262144/n1 | 556295 | 74030614843598464222542/10^18 | 74030614843598464222542/10^18 | T/F/T/T/F/F/T |
| topic_count: log1/a5/l262144/n7 → log4/a5/l262144/n7 | 401072 | 428358303905189249557794/10^18 | 428358303905189249557794/10^18 | T/F/T/T/F/F/T |
| topic_count: log1/a5/l64/n0 → log4/a5/l64/n0 | 18676 | 14976000000000000000000/10^18 | 14976000000000000000000/10^18 | T/F/T/T/T/F/T |
| topic_count: log1/a5/l64/n1 → log4/a5/l64/n1 | 76265 | 9959454614843598464222542/10^18 | 9959454614843598464222542/10^18 | T/F/T/T/F/F/T |
| topic_count: log1/a5/l64/n10912 → log4/a5/l64/n5896 | 69319017 | 58267404176405899421902107520/10^18 | 58267404176405899421902107520/10^18 | F/F/T/T/F/F/T |
| topic_count: log1/zero/l0/n0 → log4/zero/l0/n0 | 5064 | 14976000000000000000000/10^18 | 14976000000000000000000/10^18 | T/F/T/T/T/F/T |
| topic_count: log1/zero/l0/n1 → log4/zero/l0/n1 | 57420 | 9976551690189524003640400/10^18 | 9976551690189524003640400/10^18 | T/F/T/T/F/F/T |
| topic_count: log1/zero/l0/n17462 → log4/zero/l0/n7395 | -44045798 | 73005410718159939978112845546/10^18 | 73005410718159939978112845546/10^18 | F/F/T/T/F/F/T |
| topic_count: log1/zero/l262144/n0 → log4/zero/l262144/n0 | 8272 | 14976000000000000000000/10^18 | 14976000000000000000000/10^18 | T/F/T/T/T/F/T |
| topic_count: log1/zero/l262144/n1 → log4/zero/l262144/n1 | 559483 | 74030614843598464222542/10^18 | 74030614843598464222542/10^18 | T/F/T/T/F/F/T |
| topic_count: log1/zero/l262144/n7 → log4/zero/l262144/n7 | 403464 | 428358303905189249557794/10^18 | 428358303905189249557794/10^18 | T/F/T/T/F/F/T |
| topic_count: log1/zero/l64/n0 → log4/zero/l64/n0 | 18962 | 14976000000000000000000/10^18 | 14976000000000000000000/10^18 | T/F/T/T/T/F/T |
| topic_count: log1/zero/l64/n1 → log4/zero/l64/n1 | 60695 | 9959454614843598464222542/10^18 | 9959454614843598464222542/10^18 | T/F/T/T/F/F/T |
| topic_count: log1/zero/l64/n10912 → log4/zero/l64/n5896 | 69319112 | 58267404176405899421902107520/10^18 | 58267404176405899421902107520/10^18 | F/F/T/T/F/F/T |

Six positive tail differences exceed both zero estimate differences; six other tail differences are negative. Four topic-count contrasts exceed both estimate increments. Signed differences remain directional whole-program observations; changed code, preparation, count/packing and envelopes prevent isolated event-cost attribution or a two-sided component bound.

The frozen LOG1 route combines its accepted `log_piecewise` model with the 24-PGU-per-interpreter-raw-gas incremental route. LOG4 combines an accepted fixed 5M-PGU event route with the same incremental route; large 256KiB data selects an incremental contribution exceeding 48M PGU per event. The sealed artifact and its original coefficients remain unchanged. These source routes explain quote composition without allocating actual PGU per opcode. The quote API exposes aggregate operations, ordinary dispatch and unique resources, not per-LOG event charges. Ordinary dispatch is already included in aggregate operations and must not be added again. Independent aggregate counters/selectors and negatives check retained attempted work without proving per-event pricing.

## Qualification and retained evidence

See the [tool README](../../bin/zkgas/bench/README.md), [binding specification](../superpowers/specs/2026-11-04-log-receipt-bulk.md) and [implementation plan](../superpowers/plans/2026-11-04-log-receipt-bulk.md). Independent qualification checked all 30 inputs and 60 signed users, authenticated world/account/code/two empty roots/owner-qualified selected support, exact ordered log-byte intent, gas/counters/guards/memory/copy and full+1. Canonical native evidence is 28 fresh guarded invocations plus two exact retained original positive smokes bridged by source/helper/object/request/runtime/GuestInput identities. All 30 original real quotes are fresh. This does not claim 30 fresh native calls.

The focused producer module has five passing tests and one optional native skip; three separately guarded original native smokes ran once and passed in 11.409s. Two were positive originals. The third, an accepted changed-topic intent mutant, remains noncanonical evidence that native success/gas alone cannot qualify intent. Independent qualification retained 13 canonical semantic negatives and 16 domain negatives; byte-oracle self-checks cover 60 positive trajectories and 11 negatives, with 117022 ordered log intents across the canonical programs. No native/VM/quote/SP1 or test/build suite was rerun for this record.

Native JSON exports status and gas, not per-log data or topics. Independent actual-byte EVM interpretation establishes ordered log intent; native gas/status/routes, constructor full-versus-minimized complete `execution_result` equality, and same-GuestInput quote/SP1 public outputs bind stages. This is not a direct native per-log export, independently reconstructed receipt root, bloom noncollision proof, per-event tariff proof or actual per-op PGU allocation.

All actuals ran sequentially once, ten zero then ten single then ten full, with no implicit retry. Native/actual/quote timeouts were 180/600/1800s, quotes at most two workers. Guards retained 6GiB sampled process-tree PSS, 192MiB output and 16MiB logs; disk preflight required 256MiB floor plus 320MiB retention reserve. Peak sampled PSS was 4,363,661,312 bytes; maximum actual wall time was 18.962536670267582s. No resource-limited or unmeasured case remains.

Independent raw 30-row/57-diagnostic signed rational arithmetic, phase order, native/quote bridges, public values, gas conversion, identities and resources passed without execution reruns. A fresh host receipt confirms all 120 unique owned shared-memory names absent; file-only verification cannot establish host absence. Post-actual closure verifies 169 frozen tracked source files unchanged at the producer revision before documentation edits. The original 543-file qualification snapshot remains immutable; final independent manifest has 548 files.

A controller summary adapter initially copied three hardcoded counts of six from the preceding SHA template. Its source now derives lengths: full/single/control counts are ten each. The retained pre-correction snapshot and correction receipt independently prove only these three summary fields changed; all rows, PGU, quotes, diagnostics and execution artifacts are identical, with no execution replay. The corrected analysis identity below is authoritative.

| Identity | Value |
| --- | --- |
| Producer source | `76928c8ce14bdf89a0ae4dd9a12c715141ab8e53` |
| Declared guest source (not ELF attestation) | `4580faf7dc009714b310fd625b5810667019d683` |
| Frozen model ID | `c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef` |
| elf artifact | `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034` |
| model artifact | `4cb6c65c6ad28e25e1f30639803958f9555e43d51e6d35d2f1945b0757ce1ac7` |
| native artifact | `ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec` |
| quote artifact | `c10eae230681d9938b99b3cadf2b77c50d6af7ca8dbb934252ebf550cf10535e` |
| sp1 artifact | `1998fb85e62fbbd190593ed1a71af044d197224174d45295f968da0d1a60330b` |
| Frozen config | `4f7255c976f12ab1b7d9674f424c60133c0e82cd6fb6c95f867e954793cc0940` |
| Qualification verification | `62c0abf751941fbbd9b1fe1a6b18d81255d4bb6e741b99dbd0bf8ac91c7b7dd8` |
| Qualified input index | `6cc12eeb870f3e68b0e50c569b17226e7e5576cbac78b2f2426b278294b77404` |
| Retained-smoke bridge | `ff0783afb60ca96b74e261f00d7ef70a46398aa1ba211316b3230443114281da` |
| Original 543-file qualification manifest | `9ff9d309c6734505c997699bd5d3358e450757040c5d84fed654abc8fe8a37f5` |
| Pre-actual policy | `b957071ffd336a11b9f16b6899081ae2efe42aa02ab5f45f1dadcbe39fb5191a` |
| Corrected actual analysis | `2d9057d738d544694cc419acf6aaf240e56bdd6ea8d5e40d13634d87897cba56` |
| Pre-correction analysis | `aa8cd7ff2dcfbcf8d65cc73c94412d4ea0a10e491aea36bb9ad04be5ef5e7fc1` |
| Summary count correction receipt | `bd31058a5f2db3c2661aefc3d601566dda405657849b91b157c1fd3438df2e6a` |
| Gas-budget report | `10e4975b41c3b5f114f3ced498af2efc8e67d57f739e65d10a07d1f99156fa4e` |
| Counter retention audit | `861787014f6bbf02f5114e1024b625cb039eeb923c057a0b6c40233910155aff` |
| Host shared-memory receipt | `26d5ff3fdf62bfa86f37424030f2e9c6c855a036115d5d7f7966730e96b633c1` |
| Post-actual source closure | `d2a97c8a3b44089618ae564a85bbfa04c4e8cf52de7f126abc0dddc67b3c5e80` |
| Independent raw actual audit | `8ee1b87e85027638e677e54ca1dae1371567e355b963bc5f393f83ace827830b` |
| Independent raw final closure | `6aeb5fa4df4efa95627d7aa38e88d2307735f1cdc059d777a07578e1d5cc2109` |
| Independent raw final report | `3c8ff1aec325fd2994a66eafbe098d85bcdafaee4e2690d041f4dcccecb4a236` |
| Independent final artifact manifest | `ddfb0fec85af802a3d18e43a631ddec4e746c9c8ee7f7e26fcd65f6c21b3f32d` |

The verified private local recovery archive is SHA256 `102630b7acb995753b6ef1006adce245e801fd8d1c5f2eb0571059171d617b9d`, 12,586,467 compressed bytes, with 1001 regular members carrying 73,737,860 payload bytes, 442 restorable duplicate aliases and four external runtime artifacts identified by hash. All streamed member hashes passed verification without extraction. Recovery receipt SHA256 is `849f36b2f3d725b1a680d7ccf085ec64ba69528356d7a99df8d6eeaf58a1035d`. It retains canonical requests/inputs, original native/quote provenance and bridges, raw actuals, source/helper/runtime identities, controls, resources, the summary correction trail and independent audits. This is private recovery evidence, not a published standalone Atlas fixture or auditor runner.

## Interpretation and remaining work

Prices remain unchanged. Preserve every absolute gap, signed contrast and exact increment comparison. The frozen high priority remains positive actual-minus-literal-node >2B PGU within <=30M charged gas; smaller gaps remain explicit refinements. Startup-scale resemblance does not causally allocate cost. Covered absolute proposals and own-R increments do not erase six tail and four topic increment deficits.

Coverage improves only for these successful LOG1/LOG4 programs, selected lengths/patterns and bounded uniform repetition. LOG0/2/3, arbitrary sizes/data/topic schedules, exceptional exits/reverted logs, nested calls/rollback, heterogeneous packing/order, cache and receipt/bloom/trie interactions remain open. The declared source and SDK6.3/ELFb76 execute-only results do not attest the ELF build or exact production program. Exact production ELF/SDK parity and representative natural proposals require a separate promotion lane. Private recovery evidence is not a published standalone Atlas fixture or production attestation. No price fit, model/Rust/guest/SDK/ELF/pin change, proof generation, GPU-throughput claim, global/IID bound, merge, deployment or chain action follows.
