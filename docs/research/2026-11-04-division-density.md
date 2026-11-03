# Dense DIV/SDIV/MOD/SMOD operand classes

This separate panel measures 32 successful proposals: four operations × four divisor limb classes × own-zero/full, using two authenticated tagged code owners with empty selected persistent storage. All actuals completed once, in the frozen order of 16 controls then 16 full points. All 16 full points are near-full and their literal node estimates cover actuals. All 16 controls retain positive node gaps, maximum 153,487,477 PGU; complete-proposal gaps and >2B-PGU node gaps within 30M charged gas are zero. Full actual PGU spans 1,275,590,798–1,964,849,706; full node/actual ratios span 1.3107023266211473–1.9874724809672075. These observations do not recommend repricing.

All 16 own-zero→full increments are covered by both estimates. Eighteen of 24 signed divisor-class-minus-limb1 increments remain uncovered by both exact scaled estimates, maximum upward-rounded deficit 562,795,259 PGU. Covered repetition increments do not erase absolute control gaps or class contrasts. The source of absolute deficits remains unisolated: positivity neither proves nor excludes a startup-only cause or carried baseline gap. No isolated per-operation actual PGU is measured.

## Program and capacity contract

Numerators are encoded as fixed-width PUSH32 values; unsigned operations use U256 max and signed operations use the negative signed numerator retained below. Divisors are positive, nonzero four limb classes with exact bytes below. Operand order is EVM top-first. Independent actual-byte interpretation checks signed quotient truncation toward zero and remainder sign, zero divisors, minimum/-1 and limb boundaries. Tags distinguish owners without changing target operands. Each runtime executes one checked warmup target plus a pure 64-target body R times: target occurrences per user are 64R+1. R0 is not an empty control.

The validation interpreter caches only a proven immutable pure body, retains complete stack/protected memory and executes the counter/branch/guards explicitly. Direct R0/1/2 differentials check the oracle. This is a host validation optimization only; the actual guest executes the full kernel. All 65 target sites per program and exact ordered operands/results are checked independently. Native exports status/gas/routes and full/minimized execution-result equality, not direct stack/operand values.

Two signed empty-calldata user transactions retain the selected full envelope in own-zero controls. Equal envelope and code/input lengths are separate properties from equal bytes: all 40 comparisons have equal signed envelopes, but none has equal runtime code or whole selected input bytes. Tags, repetitions and operands remain bound to authenticated code; account/code/global witness work is outside selected-storage equality. Canonical input byte lengths and hashes appear below rather than implying complete-input identity.

Every selected full R is 16792, with 1,074,689 targets per user, charged block gas 29,998,763, user gas sum 29,831,408 and no refund. Gross gas includes 21000 intrinsic per user, ordinary opcodes, target cost 5 and memory expansion 6. The per-user 15M fixture cap, 16,777,216 EIP-7825 cap, 31M header ordered admission prefixes, 30M charged-block budget and 1000 continuation margin are separate checks. Full+1 is independently executed for both owners at sufficient gas; its charged block gas 30,000,537 violates the 30M budget while other caps pass. Maximal uniform R is not global packing optimality.

| Operation/class | Numerator hex | Divisor hex | Full R | Targets/user | Full charged gas | Full+1 R | Full+1 envelope/user | Full+1 charged gas | Fixture/EIP7825/header/repetition/runtime caps pass |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| div/limb1 | 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff | 0x0000000000000000000000000000000000000000000000000000000000000003 | 16792 | 1074689 | 29998763 | 16793 | 14917591 | 30000537 | yes/yes/yes/yes/yes |
| div/limb2 | 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff | 0x0000000000000000000000000000000000000000000000010000000000000001 | 16792 | 1074689 | 29998763 | 16793 | 14917591 | 30000537 | yes/yes/yes/yes/yes |
| div/limb3 | 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff | 0x0000000000000000000000000000000100000000000000000000000000000001 | 16792 | 1074689 | 29998763 | 16793 | 14917591 | 30000537 | yes/yes/yes/yes/yes |
| div/limb4 | 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff | 0x0000000000000001000000000000000000000000000000000000000000000001 | 16792 | 1074689 | 29998763 | 16793 | 14917591 | 30000537 | yes/yes/yes/yes/yes |
| sdiv/limb1 | 0x8000000000000000000000000000000000000000000000000000000000000001 | 0x0000000000000000000000000000000000000000000000000000000000000003 | 16792 | 1074689 | 29998763 | 16793 | 14917591 | 30000537 | yes/yes/yes/yes/yes |
| sdiv/limb2 | 0x8000000000000000000000000000000000000000000000000000000000000001 | 0x0000000000000000000000000000000000000000000000010000000000000001 | 16792 | 1074689 | 29998763 | 16793 | 14917591 | 30000537 | yes/yes/yes/yes/yes |
| sdiv/limb3 | 0x8000000000000000000000000000000000000000000000000000000000000001 | 0x0000000000000000000000000000000100000000000000000000000000000001 | 16792 | 1074689 | 29998763 | 16793 | 14917591 | 30000537 | yes/yes/yes/yes/yes |
| sdiv/limb4 | 0x8000000000000000000000000000000000000000000000000000000000000001 | 0x0000000000000001000000000000000000000000000000000000000000000001 | 16792 | 1074689 | 29998763 | 16793 | 14917591 | 30000537 | yes/yes/yes/yes/yes |
| mod/limb1 | 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff | 0x0000000000000000000000000000000000000000000000000000000000000003 | 16792 | 1074689 | 29998763 | 16793 | 14917591 | 30000537 | yes/yes/yes/yes/yes |
| mod/limb2 | 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff | 0x0000000000000000000000000000000000000000000000010000000000000001 | 16792 | 1074689 | 29998763 | 16793 | 14917591 | 30000537 | yes/yes/yes/yes/yes |
| mod/limb3 | 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff | 0x0000000000000000000000000000000100000000000000000000000000000001 | 16792 | 1074689 | 29998763 | 16793 | 14917591 | 30000537 | yes/yes/yes/yes/yes |
| mod/limb4 | 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff | 0x0000000000000001000000000000000000000000000000000000000000000001 | 16792 | 1074689 | 29998763 | 16793 | 14917591 | 30000537 | yes/yes/yes/yes/yes |
| smod/limb1 | 0x8000000000000000000000000000000000000000000000000000000000000001 | 0x0000000000000000000000000000000000000000000000000000000000000003 | 16792 | 1074689 | 29998763 | 16793 | 14917591 | 30000537 | yes/yes/yes/yes/yes |
| smod/limb2 | 0x8000000000000000000000000000000000000000000000000000000000000001 | 0x0000000000000000000000000000000000000000000000010000000000000001 | 16792 | 1074689 | 29998763 | 16793 | 14917591 | 30000537 | yes/yes/yes/yes/yes |
| smod/limb3 | 0x8000000000000000000000000000000000000000000000000000000000000001 | 0x0000000000000000000000000000000100000000000000000000000000000001 | 16792 | 1074689 | 29998763 | 16793 | 14917591 | 30000537 | yes/yes/yes/yes/yes |
| smod/limb4 | 0x8000000000000000000000000000000000000000000000000000000000000001 | 0x0000000000000001000000000000000000000000000000000000000000000001 | 16792 | 1074689 | 29998763 | 16793 | 14917591 | 30000537 | yes/yes/yes/yes/yes |

## Literal whole-proposal results

Actual PGU is exactly raw SDK gas ×10//191. Literal node columns sum the original per-block node estimates; complete quotes include proposal work. Signed actual-minus-estimate values are retained without clamping, including all negative full margins. No node admission limit is selected. The frozen high-priority rule is a positive actual-minus-node gap >2B PGU at <=30M charged block gas.

| Input | Charged gas | Raw SDK gas | Actual PGU | Literal node PGU | Complete quote PGU | Actual-node | Actual-complete |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| div-limb1-n0 | 209755 | 3080910923 | 161304236 | 7817213 | 162810299 | 153487023 | -1506063 |
| div-limb1-n16792 | 29998763 | 24363784244 | 1275590798 | 2535201608 | 2690194695 | -1259610810 | -1414603897 |
| div-limb2-n0 | 209755 | 3080917289 | 161304570 | 7817213 | 162810299 | 153487357 | -1505729 |
| div-limb2-n16792 | 29998763 | 27863256144 | 1458809222 | 2535201608 | 2690194695 | -1076392386 | -1231385473 |
| div-limb3-n0 | 209755 | 3080919586 | 161304690 | 7817213 | 162810299 | 153487477 | -1505609 |
| div-limb3-n16792 | 29998763 | 35045476438 | 1834841698 | 2535201608 | 2690194695 | -700359910 | -855352997 |
| div-limb4-n0 | 209755 | 3080915072 | 161304454 | 7817213 | 162810299 | 153487241 | -1505845 |
| div-limb4-n16792 | 29998763 | 30100343201 | 1575934199 | 2535201608 | 2690194695 | -959267409 | -1114260496 |
| sdiv-limb1-n0 | 209755 | 3080913703 | 161304382 | 7817413 | 162810499 | 153486969 | -1506117 |
| sdiv-limb1-n16792 | 29998763 | 26791795853 | 1402711824 | 2750139408 | 2905132495 | -1347427584 | -1502420671 |
| sdiv-limb2-n0 | 209755 | 3080864815 | 161301822 | 7817413 | 162810499 | 153484409 | -1508677 |
| sdiv-limb2-n16792 | 29998763 | 30340851058 | 1588526233 | 2750139408 | 2905132495 | -1161613175 | -1316606262 |
| sdiv-limb3-n0 | 209755 | 3080871651 | 161302180 | 7817413 | 162810499 | 153484767 | -1508319 |
| sdiv-limb3-n16792 | 29998763 | 37528629393 | 1964849706 | 2750139408 | 2905132495 | -785289702 | -940282789 |
| sdiv-limb4-n0 | 209755 | 3080866199 | 161301895 | 7817413 | 162810499 | 153484482 | -1508604 |
| sdiv-limb4-n16792 | 29998763 | 32583570629 | 1705946106 | 2750139408 | 2905132495 | -1044193302 | -1199186389 |
| mod-limb1-n0 | 209755 | 3080912164 | 161304301 | 7817213 | 162810299 | 153487088 | -1505998 |
| mod-limb1-n16792 | 29998763 | 24484204149 | 1281895505 | 2535201608 | 2690194695 | -1253306103 | -1408299190 |
| mod-limb2-n0 | 209755 | 3080912124 | 161304299 | 7817213 | 162810299 | 153487086 | -1506000 |
| mod-limb2-n16792 | 29998763 | 27983623184 | 1465111161 | 2535201608 | 2690194695 | -1070090447 | -1225083534 |
| mod-limb3-n0 | 209755 | 3080869108 | 161302047 | 7817213 | 162810299 | 153484834 | -1508252 |
| mod-limb3-n16792 | 29998763 | 35165801593 | 1841141444 | 2535201608 | 2690194695 | -694060164 | -849053251 |
| mod-limb4-n0 | 209755 | 3080913974 | 161304396 | 7817213 | 162810299 | 153487183 | -1505903 |
| mod-limb4-n16792 | 29998763 | 30237677844 | 1583124494 | 2535201608 | 2690194695 | -952077114 | -1107070201 |
| smod-limb1-n0 | 209755 | 3080911353 | 161304259 | 7817213 | 162810299 | 153487046 | -1506040 |
| smod-limb1-n16792 | 29998763 | 26194430483 | 1371436150 | 2535201608 | 2690194695 | -1163765458 | -1318758545 |
| smod-limb2-n0 | 209755 | 3080913291 | 161304360 | 7817213 | 162810299 | 153487147 | -1505939 |
| smod-limb2-n16792 | 29998763 | 29742553835 | 1557201771 | 2535201608 | 2690194695 | -977999837 | -1132992924 |
| smod-limb3-n0 | 209755 | 3080873293 | 161302266 | 7817213 | 162810299 | 153485053 | -1508033 |
| smod-limb3-n16792 | 29998763 | 36943819912 | 1934231409 | 2535201608 | 2690194695 | -600970199 | -755963286 |
| smod-limb4-n0 | 209755 | 3080916629 | 161304535 | 7817213 | 162810299 | 153487322 | -1505764 |
| smod-limb4-n16792 | 29998763 | 31998720695 | 1675325690 | 2535201608 | 2690194695 | -859875918 | -1014869005 |

| Input | GuestInput bytes | Canonical GuestInput SHA256 |
| --- | ---: | --- |
| div-limb1-n0 | 334305 | fb8ad6a7373ded05c2dcf9e335f567a604d2603b2f926c3ad9313f8931cb6b00 |
| div-limb1-n16792 | 334305 | adf3aa718f2e5858132f8c55234ae0b0a3ee72eaa90229c2f810fd0668e2ed74 |
| div-limb2-n0 | 334305 | e6f2bebfcb2065f6a2f989208c313c2e8473c1667c06fd91c0d432bceba46aeb |
| div-limb2-n16792 | 334305 | 470aeb5ae248f6865c9ad73fdd6e5384b14a100d6668a4a8f86aa1d997d3295a |
| div-limb3-n0 | 334305 | a0a6d352dce1f8614183dd29a35a08925f1ad67c611734c813d533ae6931f1f3 |
| div-limb3-n16792 | 334305 | 5945c19cec26678747f49c07613d8c56bc4e77d3a2d0c0f135ad6fdbbf4456c6 |
| div-limb4-n0 | 334305 | 8fdaaa9f57a8364fe38c5e78887c1a3db2c72f75d0a34e361b833ec1c3ffe484 |
| div-limb4-n16792 | 334305 | a8e288f114dc31e1437230e727a9f2b5c4dde7b43e3694b133c12212e2ee3c62 |
| sdiv-limb1-n0 | 334305 | ea9bc8172c53421ba3e910b052d868bb28ee3bc28b73109fcf783045d3e8ce55 |
| sdiv-limb1-n16792 | 334305 | 62b6345190fbbd8717e9f117dc2543551e4ef87e6096013564323eea69358b94 |
| sdiv-limb2-n0 | 334305 | 66117697a310cadb6e1c20a0cf6836714a7d831093465ea3cd87e8678fb1479e |
| sdiv-limb2-n16792 | 334305 | 3e3f24fd4f691c621a6e47ffc27ab53d43cbc806135cf00f0987444e76482286 |
| sdiv-limb3-n0 | 334305 | cea596bf4669e074863bdfb1b415769725d392ecf203a332b3ab8fa0fd3cfa9d |
| sdiv-limb3-n16792 | 334305 | f57b23a04a7601cdf34de155200e9813e9f4221243922485ab59224431e5d316 |
| sdiv-limb4-n0 | 334305 | 66ac24606b60d4d77e00a0a3ebe3e897565dcd6413d28045ea669074e351c66b |
| sdiv-limb4-n16792 | 334305 | c46d84adb25004a934084246c03a64b72f73e420fc61d5e3e1e66131477571c7 |
| mod-limb1-n0 | 334305 | dabddadc01ec323aee2aec0b02ec2f7c93b33791ce2e581138591226dce475b7 |
| mod-limb1-n16792 | 334305 | b855551da762916c4cd206cb3b73e5a96ab739bb40f1b094f15074fe34c62a2b |
| mod-limb2-n0 | 334305 | 7817350ef26161c92376e32822056ad0645a18505c3a545c6c5c8c7bc3b734dc |
| mod-limb2-n16792 | 334305 | 03961289ea362053b42034ad5d7735e71e1352a4f73e4d8ab1b08bcbdc6da640 |
| mod-limb3-n0 | 334305 | 7b86e7c083952a3f96605b4d1eb7202169b79cd1c2c7b2e32fd2a7cfb394422c |
| mod-limb3-n16792 | 334305 | 009985010cc54ba27dce7419a2d0f684971e174bc2adad911282f9d00a87ddbd |
| mod-limb4-n0 | 334305 | 69d022597819946d5ed51368344430ff8863fae8c168247398032f718dc146e4 |
| mod-limb4-n16792 | 334305 | 47767b4569f0248a7297b676ffc17ea49c000c7351ea78f9c93fff28d8eb36be |
| smod-limb1-n0 | 334305 | 01f8b05925d0980956aba9609b30fc5d6d2666789531f1f8cb152570c9d4cdee |
| smod-limb1-n16792 | 334305 | 169f9a6b7baac30ff4a54416d085ce0610894ff8067f3b8d77df3ecad62c21fb |
| smod-limb2-n0 | 334305 | c03b6818c16eef9d0206a43dc452857b96ad82ede0c17124b7f04c51893e78af |
| smod-limb2-n16792 | 334305 | 9930d135f8f3db104c5b697fa100b28c4a314999298fc58e9bbec842f46c2f97 |
| smod-limb3-n0 | 334305 | 3d0b359f87e81ae3c8ddd5f8cea2c527e5b34a59d7ca80b9b568c0db97a57aa8 |
| smod-limb3-n16792 | 334305 | feb940a98990f633ee22271f1e2300edb0e2da6b3e8cbb2c4e99a709765c761f |
| smod-limb4-n0 | 334305 | 4831942192255c78ead14ff129592f37b72676aa24df2e279aa67c1900c88a04 |
| smod-limb4-n16792 | 334305 | 250b07ab8d5ec16f814a16be60d9b143ab3f8870d451a4b9a8efe516819a586a |

## Signed exact-scaled diagnostics

The following 40 finite whole-proposal contrasts retain both signed actual and exact estimate deltas. Fractions are exact PGU using integer-scaled estimates (scale 10^18); they do not subtract rounded quote ceilings. Class contrasts include controls and full points; actual class deltas span -2,560–562,795,259 PGU. Whole-block quote operation aggregates include system/Anchor work: per-target Anchor route counts are not exported, so DIV/MOD aggregates cannot be attributed only to users. Full-minus-control quote-operation aggregates do not establish event counts or actual per-op PGU.

| From | To | Actual delta | Exact node delta | Exact complete delta | Actual-exact node delta | Actual-exact complete delta |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| div-limb1-n0 | div-limb1-n16792 | 1114286562 | 78980762345226404588818303/31250000000000000 | 78980762345226404588818303/31250000000000000 | -44159307282726404588818303/31250000000000000 | -44159307282726404588818303/31250000000000000 |
| div-limb2-n0 | div-limb2-n16792 | 1297504652 | 78980762345226404588818303/31250000000000000 | 78980762345226404588818303/31250000000000000 | -38433741970226404588818303/31250000000000000 | -38433741970226404588818303/31250000000000000 |
| div-limb3-n0 | div-limb3-n16792 | 1673537008 | 78980762345226404588818303/31250000000000000 | 78980762345226404588818303/31250000000000000 | -26682730845226404588818303/31250000000000000 | -26682730845226404588818303/31250000000000000 |
| div-limb4-n0 | div-limb4-n16792 | 1414629745 | 78980762345226404588818303/31250000000000000 | 78980762345226404588818303/31250000000000000 | -34773582813976404588818303/31250000000000000 | -34773582813976404588818303/31250000000000000 |
| sdiv-limb1-n0 | sdiv-limb1-n16792 | 1241407442 | 85697562345226404588818303/31250000000000000 | 85697562345226404588818303/31250000000000000 | -46903579782726404588818303/31250000000000000 | -46903579782726404588818303/31250000000000000 |
| sdiv-limb2-n0 | sdiv-limb2-n16792 | 1427224411 | 85697562345226404588818303/31250000000000000 | 85697562345226404588818303/31250000000000000 | -41096799501476404588818303/31250000000000000 | -41096799501476404588818303/31250000000000000 |
| sdiv-limb3-n0 | sdiv-limb3-n16792 | 1803547526 | 85697562345226404588818303/31250000000000000 | 85697562345226404588818303/31250000000000000 | -29336702157726404588818303/31250000000000000 | -29336702157726404588818303/31250000000000000 |
| sdiv-limb4-n0 | sdiv-limb4-n16792 | 1544644211 | 85697562345226404588818303/31250000000000000 | 85697562345226404588818303/31250000000000000 | -37427430751476404588818303/31250000000000000 | -37427430751476404588818303/31250000000000000 |
| mod-limb1-n0 | mod-limb1-n16792 | 1120591204 | 78980762345226404588818303/31250000000000000 | 78980762345226404588818303/31250000000000000 | -43962287220226404588818303/31250000000000000 | -43962287220226404588818303/31250000000000000 |
| mod-limb2-n0 | mod-limb2-n16792 | 1303806862 | 78980762345226404588818303/31250000000000000 | 78980762345226404588818303/31250000000000000 | -38236797907726404588818303/31250000000000000 | -38236797907726404588818303/31250000000000000 |
| mod-limb3-n0 | mod-limb3-n16792 | 1679839397 | 78980762345226404588818303/31250000000000000 | 78980762345226404588818303/31250000000000000 | -26485781188976404588818303/31250000000000000 | -26485781188976404588818303/31250000000000000 |
| mod-limb4-n0 | mod-limb4-n16792 | 1421820098 | 78980762345226404588818303/31250000000000000 | 78980762345226404588818303/31250000000000000 | -34548884282726404588818303/31250000000000000 | -34548884282726404588818303/31250000000000000 |
| smod-limb1-n0 | smod-limb1-n16792 | 1210131891 | 78980762345226404588818303/31250000000000000 | 78980762345226404588818303/31250000000000000 | -41164140751476404588818303/31250000000000000 | -41164140751476404588818303/31250000000000000 |
| smod-limb2-n0 | smod-limb2-n16792 | 1395897411 | 78980762345226404588818303/31250000000000000 | 78980762345226404588818303/31250000000000000 | -35358968251476404588818303/31250000000000000 | -35358968251476404588818303/31250000000000000 |
| smod-limb3-n0 | smod-limb3-n16792 | 1772929143 | 78980762345226404588818303/31250000000000000 | 78980762345226404588818303/31250000000000000 | -23576726626476404588818303/31250000000000000 | -23576726626476404588818303/31250000000000000 |
| smod-limb4-n0 | smod-limb4-n16792 | 1514021155 | 78980762345226404588818303/31250000000000000 | 78980762345226404588818303/31250000000000000 | -31667601251476404588818303/31250000000000000 | -31667601251476404588818303/31250000000000000 |
| div-limb1-n0 | div-limb2-n0 | 334 | 0/1 | 0/1 | 334/1 | 334/1 |
| div-limb1-n16792 | div-limb2-n16792 | 183218424 | 0/1 | 0/1 | 183218424/1 | 183218424/1 |
| div-limb1-n0 | div-limb3-n0 | 454 | 0/1 | 0/1 | 454/1 | 454/1 |
| div-limb1-n16792 | div-limb3-n16792 | 559250900 | 0/1 | 0/1 | 559250900/1 | 559250900/1 |
| div-limb1-n0 | div-limb4-n0 | 218 | 0/1 | 0/1 | 218/1 | 218/1 |
| div-limb1-n16792 | div-limb4-n16792 | 300343401 | 0/1 | 0/1 | 300343401/1 | 300343401/1 |
| sdiv-limb1-n0 | sdiv-limb2-n0 | -2560 | 0/1 | 0/1 | -2560/1 | -2560/1 |
| sdiv-limb1-n16792 | sdiv-limb2-n16792 | 185814409 | 0/1 | 0/1 | 185814409/1 | 185814409/1 |
| sdiv-limb1-n0 | sdiv-limb3-n0 | -2202 | 0/1 | 0/1 | -2202/1 | -2202/1 |
| sdiv-limb1-n16792 | sdiv-limb3-n16792 | 562137882 | 0/1 | 0/1 | 562137882/1 | 562137882/1 |
| sdiv-limb1-n0 | sdiv-limb4-n0 | -2487 | 0/1 | 0/1 | -2487/1 | -2487/1 |
| sdiv-limb1-n16792 | sdiv-limb4-n16792 | 303234282 | 0/1 | 0/1 | 303234282/1 | 303234282/1 |
| mod-limb1-n0 | mod-limb2-n0 | -2 | 0/1 | 0/1 | -2/1 | -2/1 |
| mod-limb1-n16792 | mod-limb2-n16792 | 183215656 | 0/1 | 0/1 | 183215656/1 | 183215656/1 |
| mod-limb1-n0 | mod-limb3-n0 | -2254 | 0/1 | 0/1 | -2254/1 | -2254/1 |
| mod-limb1-n16792 | mod-limb3-n16792 | 559245939 | 0/1 | 0/1 | 559245939/1 | 559245939/1 |
| mod-limb1-n0 | mod-limb4-n0 | 95 | 0/1 | 0/1 | 95/1 | 95/1 |
| mod-limb1-n16792 | mod-limb4-n16792 | 301228989 | 0/1 | 0/1 | 301228989/1 | 301228989/1 |
| smod-limb1-n0 | smod-limb2-n0 | 101 | 0/1 | 0/1 | 101/1 | 101/1 |
| smod-limb1-n16792 | smod-limb2-n16792 | 185765621 | 0/1 | 0/1 | 185765621/1 | 185765621/1 |
| smod-limb1-n0 | smod-limb3-n0 | -1993 | 0/1 | 0/1 | -1993/1 | -1993/1 |
| smod-limb1-n16792 | smod-limb3-n16792 | 562795259 | 0/1 | 0/1 | 562795259/1 | 562795259/1 |
| smod-limb1-n0 | smod-limb4-n0 | 276 | 0/1 | 0/1 | 276/1 | 276/1 |
| smod-limb1-n16792 | smod-limb4-n16792 | 303889540 | 0/1 | 0/1 | 303889540/1 | 303889540/1 |

The eight equality flags retain the following true counts across all 40 rows:

| Flag | True/40 |
| --- | ---: |
| same_operation | 40 |
| same_numerator | 40 |
| same_divisor | 16 |
| same_repetitions | 24 |
| same_target_occurrences | 24 |
| same_charged_gas | 24 |
| same_signed_envelope | 40 |
| same_authenticated_selected_storage_support | 40 |

Per-comparison flags below use T/F in the same eight-column order as the totals.

| From | To | Operation | Numerator | Divisor | R | Targets | Gas | Envelope | Owner support |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| div-limb1-n0 | div-limb1-n16792 | T | T | T | F | F | F | T | T |
| div-limb2-n0 | div-limb2-n16792 | T | T | T | F | F | F | T | T |
| div-limb3-n0 | div-limb3-n16792 | T | T | T | F | F | F | T | T |
| div-limb4-n0 | div-limb4-n16792 | T | T | T | F | F | F | T | T |
| sdiv-limb1-n0 | sdiv-limb1-n16792 | T | T | T | F | F | F | T | T |
| sdiv-limb2-n0 | sdiv-limb2-n16792 | T | T | T | F | F | F | T | T |
| sdiv-limb3-n0 | sdiv-limb3-n16792 | T | T | T | F | F | F | T | T |
| sdiv-limb4-n0 | sdiv-limb4-n16792 | T | T | T | F | F | F | T | T |
| mod-limb1-n0 | mod-limb1-n16792 | T | T | T | F | F | F | T | T |
| mod-limb2-n0 | mod-limb2-n16792 | T | T | T | F | F | F | T | T |
| mod-limb3-n0 | mod-limb3-n16792 | T | T | T | F | F | F | T | T |
| mod-limb4-n0 | mod-limb4-n16792 | T | T | T | F | F | F | T | T |
| smod-limb1-n0 | smod-limb1-n16792 | T | T | T | F | F | F | T | T |
| smod-limb2-n0 | smod-limb2-n16792 | T | T | T | F | F | F | T | T |
| smod-limb3-n0 | smod-limb3-n16792 | T | T | T | F | F | F | T | T |
| smod-limb4-n0 | smod-limb4-n16792 | T | T | T | F | F | F | T | T |
| div-limb1-n0 | div-limb2-n0 | T | T | F | T | T | T | T | T |
| div-limb1-n16792 | div-limb2-n16792 | T | T | F | T | T | T | T | T |
| div-limb1-n0 | div-limb3-n0 | T | T | F | T | T | T | T | T |
| div-limb1-n16792 | div-limb3-n16792 | T | T | F | T | T | T | T | T |
| div-limb1-n0 | div-limb4-n0 | T | T | F | T | T | T | T | T |
| div-limb1-n16792 | div-limb4-n16792 | T | T | F | T | T | T | T | T |
| sdiv-limb1-n0 | sdiv-limb2-n0 | T | T | F | T | T | T | T | T |
| sdiv-limb1-n16792 | sdiv-limb2-n16792 | T | T | F | T | T | T | T | T |
| sdiv-limb1-n0 | sdiv-limb3-n0 | T | T | F | T | T | T | T | T |
| sdiv-limb1-n16792 | sdiv-limb3-n16792 | T | T | F | T | T | T | T | T |
| sdiv-limb1-n0 | sdiv-limb4-n0 | T | T | F | T | T | T | T | T |
| sdiv-limb1-n16792 | sdiv-limb4-n16792 | T | T | F | T | T | T | T | T |
| mod-limb1-n0 | mod-limb2-n0 | T | T | F | T | T | T | T | T |
| mod-limb1-n16792 | mod-limb2-n16792 | T | T | F | T | T | T | T | T |
| mod-limb1-n0 | mod-limb3-n0 | T | T | F | T | T | T | T | T |
| mod-limb1-n16792 | mod-limb3-n16792 | T | T | F | T | T | T | T | T |
| mod-limb1-n0 | mod-limb4-n0 | T | T | F | T | T | T | T | T |
| mod-limb1-n16792 | mod-limb4-n16792 | T | T | F | T | T | T | T | T |
| smod-limb1-n0 | smod-limb2-n0 | T | T | F | T | T | T | T | T |
| smod-limb1-n16792 | smod-limb2-n16792 | T | T | F | T | T | T | T | T |
| smod-limb1-n0 | smod-limb3-n0 | T | T | F | T | T | T | T | T |
| smod-limb1-n16792 | smod-limb3-n16792 | T | T | F | T | T | T | T | T |
| smod-limb1-n0 | smod-limb4-n0 | T | T | F | T | T | T | T | T |
| smod-limb1-n16792 | smod-limb4-n16792 | T | T | F | T | T | T | T | T |

Selected support equality means the two authenticated owner-qualified empty persistent-storage sets, not absence of account/code/global authentication. Class comparisons change operand code bytes; own-R comparisons change counter code and charged gas. Signed envelopes alone cannot isolate causality.

## Qualification and retained evidence

Producer source is `433ed0355d67af4bf6238092c061d181a9ccc599`; independent source review passed before canonical export. Exactly 30 fresh guarded native invocations plus two exact original positive bridges qualify canonical32, followed by 32 fresh original quotes and 64 authenticated signatures. The author native3 receipts remain original; the native-accepted divisor5 mutant is excluded by ordered operand intent. No author/native/quote/SP1 execution was rerun for this documentation. Qualification retained 1024 synthetic arithmetic checks, eight explicit signed edges, 96 direct/cache differentials, 61 domain rejections, 44 real-byte semantic negatives, both-owner full+1 executions, witness/trie/code/receipt/public-output binding, retained-module provenance and original quote counter/resource reconciliation. Native user route counts reconcile 34,390,080 user targets across canonical32; whole-block system dispatch allowance 12363 and persistent access/write allowance 4 remain separate from empty owner storage.

Frozen source/input/tool/model/order identities predate every actual attempt. SDK 6.3.0 streaming estimator uses the unchanged experimental evm-gas-only ELF, not a proof. Its program manifest declares source revision `4580faf7dc009714b310fd625b5810667019d683`; this is not a build attestation or production parity proof. Independent raw32/40 closure passed and reconciled root-derived summaries after independent derivation. Source175 matched the frozen inventory and revision blobs; original590 and final593 artifact manifests preserve separate identities. All 128 unique attempt-owned shared-memory resources passed cleanup and a read-only host absence receipt in the same mount namespace. No host deletion was performed by the verifier.

Actual guards were 6144MiB sampled process-tree PSS, 600s, 256MiB runtime disk floor plus 320MiB retention preflight reserve, 192MiB output and 16MiB log. Maximum observed actual sampled PSS was 4,544,541,696 bytes and elapsed time 62.55589387193322s. Native/quote time caps were 180s/1800s; maxima were 12.1720759887s/3.3836830743s with sampled PSS 55,841,792/40,571,904 bytes. These are sampled execution/qualification observations, not proof/GPU performance or guaranteed peaks. No actual stage failed or retried. One preparation oracle memory-expansion correction and metadata adapters are retained in private reports; they did not rerun external measurements.

Evidence identities (SHA256, private artifacts retained outside public Git):

| Artifact | SHA256 |
| --- | --- |
| analysis.json | e5647210f90d673dd6bc0a0d934a7a0a8c9ffd33c1438c227705316d6e092b68 |
| gas-budget-report.json | a13b676bc919150e5fb5d8edaf0ff4aa72edbaa018908453f0a5a17f4343d2fa |
| actual-audit.json | 1f08979f3f552987b211a9d9c60743ca683e37abbf89f94bb1b6b4bdae6480b4 |
| raw-final-closure.json | 45f8c4dd25b4e20b2d3f59006bfd61701a573ce7c70ad8ed0b7227d8099b1fda |
| policy-freeze.json | 68bf69b80d083adbc20ab22c806fb83ee50583bab36c50e94b01603424bd0eb7 |
| canonical catalog | 03174c4f96f1563d38283d41d68a4dc5fe252f6a707051e83c17da2168a73811 |
| source-files-sha256.json | dbbcb621931e7b05f2da3a6b33855bdc60a0df4f9a0ecdbb301af7b363113504 |
| source-closure.json | 4fc10ad15a4b53668f96b09cb40e03e31dd892c1a94126597e55a91afb32a879 |
| host-shared-memory-audit.json | a94cbf94f8fb7ae2cc4bd9eaeb8c0664bfc046847155dbba06345b115371bf68 |
| original qualification manifest | 2b069a1a7738c8dfccbee04a33d9353460ce526209e5bcafbd75576bca8c7aca |
| final artifact manifest | dfa0295a1fbcfffcf4de390ecd4baf194ae615d9717d77f0c8be08638be13e3a |
| verification.json | f442250fd3de621815db9968fa59075ea555192416530e3a70308e15ba14403a |
| qualified input index | cc1cc5dc54b6614528a5728f0ade07743290c82116056784cafa8a02b15d24fb |
| counter-retention-audit.json | 416a3993c8335b32d80bdbf4aa3bc6b5369202426ecb6378e718e4c84b4ec289 |
| retained smoke bridge | 59df2d84bef774b763775790fed270b43c84e286ab3fe58875edca480abdbae4 |
| full-plus-one-both-owners.json | 68675676a84ee297b562bf271a66afcf732b7bc0d32f460ab7fb4708634ecdd6 |
| frozen elf | b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034 |
| frozen model | 4cb6c65c6ad28e25e1f30639803958f9555e43d51e6d35d2f1945b0757ce1ac7 |
| frozen native | ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec |
| frozen program_manifest | 8d5b4a482d26e3bdca94c5e66b9de7b33dc57bcfbaa9b9c24140bf030c1f93a9 |
| frozen quote | c10eae230681d9938b99b3cadf2b77c50d6af7ca8dbb934252ebf550cf10535e |
| frozen sp1 | 1998fb85e62fbbd190593ed1a71af044d197224174d45295f968da0d1a60330b |
| semantic model ID | c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef |
| generator source | 7379fc276abcc053b66e17df29387e22e818249f9b3b3fa7a7a0fbad183f0c24 |
| source review gate | 81ad78585195b0cf992fe926aeff24bed2ec60b8c233219fdcf6f7aba9130179 |
| private archive | 5fce88d73368692bbc4c251f59f09a1fbbc988c2776ac31b176e2c9f7683d5ae |

The verified private recovery archive is `division-density-evidence-20261104.tar.gz`: 3,983,873 compressed bytes, 1076 regular members, 49,003,597 payload bytes, 472 restorable duplicate aliases and four external runtime artifacts. Every streamed member hash was verified without extraction. External runtime identities remain explicit; an archive is not a source build or deployment attestation.

The prior persistent236, transient8, pairing18, defaultSHA18, linearSHA18, LOG30 and MCOPY12 cohorts remain seven separate immutable read-only contexts. Their inputs/resources were checked disjoint without rerunning or relabelling historical measurements; no new union is asserted. See the [coverage inventory](zkgas-workload-coverage.md) for remaining work. This panel closes only selected successful finite division/remainder operand classes and bounded repetition. Arbitrary operands/signs, stack/control-flow mixes, heterogeneous packing/order, state/memory interactions, cache behavior, proof/GPU execution, global bounds, exact production parity and natural-transaction usability remain open. Prices, model, ELF, SDK and pins are unchanged.
