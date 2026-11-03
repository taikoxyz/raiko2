# Modular Arithmetic Overflow and Modulus Density

Continue the finite search for max(actual same-proposal SP1 PGU / literal node block-known zkGas). Compare complete proposal estimates independently. High priority is actual minus literal node >2B PGU within <=30M charged EVM block gas. No pricing change is inferred from source or this design. Keep the current ADDMOD1100/MULMOD1600 floors and every frozen model/guest/SDK/ELF/pin unchanged. This is a benchmark tooling/evidence increment, not consensus or deployed-pricing work.

## Selected finite domain

16 original inputs: operation ADDMOD(0x08)/MULMOD(0x09), modulus class limb1=3, limb2=2^64+1, limb3=2^128+1, limb4=2^192+1, each own-zero/full. Every operation uses a=2^256-2,b=2^256-3 as unsigned256 inputs. Exact result is (a+b)%m or(a*b)%m with untruncated257/512bit intermediate. These values distinguish exact from premature256bit-wrap-before-modulo in every selected domain. They avoid a divisible by the limb2/3 moduli; no guarantee of worst operand or quotient correction behavior is asserted.

Each one-block proposal has two EIP1559 user transactions from one signer with nonces0/1, to two distinct code owners tagged0/1 at addresses BASE_ADDRESS+tag from unchanged storage_fanout. Both have empty persistent storage and empty outer calldata. No nested call/precompile/log/refund/create/revert. Runtime tags distinguish code identities, not a claim of per-target unique arithmetic inputs. STOP terminal and all receipts must succeed.

## Bytecode and limits

New compiler-free producer cases/modular_density.py, test oracle tests/modular_density_vm.py and focused tests/test_modular_density.py under bin/zkgas/bench. APIs: make_case(operation='mulmod',modulus_class='limb3',level='full'), _runtime(operation,modulus_class,repetitions,tag=0), gas_accounting(operation,modulus_class,repetitions), fits(operation,modulus_class,repetitions), full_repetitions(operation,modulus_class), prepare(output:Path,*,model_id=MODEL). CLI --output-dir and --model-id; invalid types/bools/domains/model/collidingoutput fail before artifacts. Source root bin/zkgas/bench with normalized source_sha256 keys cases/...; helpers unchanged and bound by file+snippet hashes. No importing current new KECCAK hash/reference dependency is needed for integer math.

Store counter at memory0 and owner tag32 once. Retain stack bottom-to-top[m,b,a]. One warmup DUP3/DUP3/DUP3/target checks exact fullwidth result evenR0. Fixed64-site pure body repeats DUP3/DUP3/DUP3/target/POP, preserving originals. Counter lives in memory and advances once per64-site repetition; final independent counter/tag/three original operands and empty stack guards, then STOP; failing guards use unreachable INVALID. Fixed PUSH32 for operands/results/R and fixed-width remaining addresses/immediates; runtime<24576. Both owners execute same opcode shapes/counts except tag values.

MAX_REPETITIONS=1,000,000; exact zero/full counts and full+1 both owners. Target operations/user=64R+1 (warmup1), target8gas plus threeDUP3 at3 and POP2 gives19gas/body site, plus separately derived loop/guards/intrinsic21000/memory expansion to2words6. Refund0/gross=charged. Sizing uses assembled multiplicities, never replays a huge body. Full is maximal uniform R satisfying every constraint: fixture transaction envelope<=15,000,000, EIP7825 envelope<=16,777,216 separately, header31,000,000 remaining gas before each user's admission covers its envelope, and whole charged block<=30,000,000 including the frozen fixture Anchor167355. Envelope is full usergross+1000 for zero andfull alike. full+1 must fail actual limiting constraint, full near99% is derived not an unsupported packing claim. Upper tool bounds are not protocol maxima.

The public byte interpreter may cache only a decoded proven pure stack-invariant body: no memory/control/other effect, actual ordered a/b/m, exact target count and result, stack preservation. Actual smallR0/1/2 direct interpreter differential, unchanged final guards and body mutation negatives qualify cache semantics. It must execute every surrounding instruction and derive gas/counts from actual bytes. Do not retain per-target event arrays or invoke production code as oracle. Actual EVM/SP1 executes the full kernel.

## Source and actual gates

TDD missing-producer RED followed by focused GREEN for all16/twoowners/gas/full+1/caps/metadata/normalized source paths/CLI/freshoutput/domains/ordered512math. Include exact-vs-wrapped results in all8domains and native-accepted same-result wrong-modulus intent negative; do not rely only on final arithmetic output. Existing helpers/default producer behavior unchanged; no broad legacy suite/guest build required.

Exactly3 source native smoke invocations once: MULMOD/limb3/full originalpositive, ADDMOD/limb1/zero originalpositive, and MULMOD/limb1/zero with actual modulus5 instead of intended3. For chosen operands both modulo3 and5 produce2: guard result can stay2, but independent actual-operand intent must reject wrongmodulus. Mutant preserves original intended request/row and is excluded fromcanonical/actual. Retain complete prelaunch source/helper/request/intent/native-build/commands/caps/outcome/input/publicoutput and accepted receipts. Native exports gas/status/routes and full/minimized execution-result equality plus same-GuestInput stage binding, not actual arithmetic operands/stack/results or per-op PGU.

Fresh independent source spec+quality gate then canonical16:14 fresh guarded native+2 exact originalpositive bridges,16 fresh original quotes (max2),32 independently authenticated signatures/account/code/empty ownerstorage/witness/roots/receipts, all16 actual byte/math/counter trajectories, both-owner full+1 for8domains, semantic/auth negatives. Verifier uses a separately written interpreter/math authority, not new public oracle or producer formulas. Bridge original native mutant remains excluded. Every original artifact/script frozen beforeactual; later stronger closure mayaddnewfiles but never rewrite original members.

Then root freezes source/tool/input/order and executes16actuals once in8zero→8full order with the existing SDK6.3,b76ELF,c893model,nativead,quotec10/SP11998. Raw ExecutionReport.gas*10//191 is normalized SP1 PGU; raw Unzen scores are not PGU. Use existing6GiB sampledprocess-tree PSS/600s actual/180s native/1800squote/256MiBruntime disk+320MiB retention reserve/192MiBoutput/16MiBlog guards, no implicit retries. Actual execute-only is not proof/GPU timing, buildattestation or currentproduction parity.

## Evidence and interpretation

20 exact signed whole-input diagnostics:8 ownzero→full and12 modulus contrasts(limb1→limb2/3/4 ×2op×2levels). Eight flags: same_operation,same_operand_a,same_operand_b,same_modulus,same_repetitions,same_charged_gas,same_signed_envelope,same_authenticated_selected_storage_support. Full signed envelope means complete transaction objects; selected two-owner empty storage support excludes account/code/global witness equality. Equal gas/code shape does not mean same input or per-op actual cost. Keep signed exact scaled deficits and literal whole-proposal gaps, includingcontrols; covered increments can't erase absolute shortfalls and positive full margins don't isolate startup causes.

Retain source inventory (prospective181 old178+3), original/final manifests, raw16/20/eight flags, actual64 owned-resource cleanup receipts and real-host absence. A sandbox-private /dev/shm absence is insufficient: match its device to every original actual resource's recorded device, use the actual host context, lstat exact owned names, no unrelated deletion. Preserve any corrected preliminary receipt with clear authority and no fabricated rerun. Stream-verified privatearchive identifies external runtime executables/aliases; it is not published standalone Atlas recovery. Maintained tools and usage stay in Raiko; immutablefixtures belong in Atlas batching, not bundled executables.

Document all16 literal rows,8 capacity/full+1 rows,20diagnostics/8flags, exact hashes/caveats, independent checks and completeincrement review. Prior persistent236/transient8/pairing18/defaultSHA18/linearSHA18/LOG30/MCOPY12/division32/KECCAK27 retain separate frozen identities; no new union or old rerun. Public docs have portable relative paths and no humanidentifying names. Update own held Draft296 only after fresh wholeincrement/body review/readback; keep do-not-merge/stacked dependencies, no merge/deploy. No-price conclusions go to existing working.log and next round starts without user pause. Other arithmetic operands/moduli, division correction cornercases, mixed-family/order graphs and exact production/natural usability remain open.

## Maintained metadata contract

The catalog corpus is `unzen-modular-density-20261104`; sample IDs are
`modular-density-{operation}-{modulus_class}-n{repetitions}`. Parameters use
`operation`, `modulus_class`, `operand_a_hex`, `operand_b_hex`, `modulus_hex`,
`expected_result_hex`, `workload_level`, `repetitions`, `full_repetitions`,
`unroll_factor`, `bulk_operations` and `target_operations_per_user`. Hex words
are lowercase with `0x` and64 digits. Common two-owner gas/storage/runtime fields
follow the existing division producer shape. The independent public byte oracle
exports `execute(code,envelope,*,operation,modulus_class,repetitions,tag,...)` and
`run_users(row)`; canonical verification uses a separately written authority.
