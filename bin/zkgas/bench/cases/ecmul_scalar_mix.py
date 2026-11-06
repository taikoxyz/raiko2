"""Compiler-free, guarded BN254 ECMUL scalar vectors for ordinary ProgramRequest."""
from functools import lru_cache

import initcode_preparation
from initcode_preparation import _Factory
import precompile_workload as curve
from precompile_workload import FIELD_MODULUS, GROUP_ORDER
import spawn_memory
from spawn_memory import digest, encoded
import storage_mutation as mutation
import top_level_create as original
import warm_read_catalogs as limits

CORPUS='unzen-ecmul-scalar-mix-20261106'
RECIPES=('low','high','mixed')
HIGH_SCALAR=(3<<252)-1
MAX_REPETITIONS=1_000_000
PRIMARY=limits.PRIMARY
SENTINEL=0xa55a
NATIVE_GAS=6000
WARM_ACCESS_GAS=100
ENVELOPE_RESERVE=1000


def _sequence(scalars):
    if type(scalars) not in (list,tuple) or not 1<=len(scalars)<=64:
        raise ValueError('scalar sequence must be a list/tuple of1..64 entries')
    for scalar in scalars:original.integer(scalar,0,(1<<256)-1,'scalar')
    return tuple(scalars)


def scalars_for(recipe):
    if type(recipe) is not str or recipe not in RECIPES:raise ValueError('unknown scalar recipe')
    return (0,)*31 if recipe=='low' else (HIGH_SCALAR,)*31 if recipe=='high' else (0,)*12+(HIGH_SCALAR,)*19


@lru_cache(maxsize=128)
def _reduced_point(scalar):
    if scalar==0:return (0,0)
    for name,value in curve.SCALARS.items():
        if scalar==value%GROUP_ORDER:return curve.expected_output(name)
    point=(1,2);result=None
    while scalar:
        if scalar&1:result=curve._point_add(result,point)
        point=curve._point_add(point,point);scalar>>=1
    return (0,0) if result is None else result


def expected_point(scalar):
    """Reuse the old pure curve helper without changing its named-input contract."""
    original.integer(scalar,0,(1<<256)-1,'scalar')
    return _reduced_point(scalar%GROUP_ORDER)


def payload(scalars):
    scalars=_sequence(scalars)
    return b''.join(value.to_bytes(32,'big') for scalar in scalars for value in (scalar,*expected_point(scalar)))


def _runtime(sequence_length,repetitions):
    original.integer(sequence_length,1,64,'sequence_length');original.integer(repetitions,0,MAX_REPETITIONS,'repetitions')
    asm=_Factory()
    def store(offset,value,times=1):asm.push(value,times=times);asm.push(offset,times=times);asm.op(0x52,times)
    def load(offset,times=1):asm.push(offset,times=times);asm.op(0x51,times)
    def check(value,times=1):mutation._check(asm,value,times)
    asm.op(0x36);check(96*sequence_length)
    for offset,value in ((0,1),(32,2),(64,0),(96,0),(128,0),(160,0),(192,0),(224,SENTINEL)):store(offset,value)
    visits=repetitions+1;asm.label('loop',visits);load(192,visits);asm.push(repetitions,times=visits);asm.op(0x14,visits);asm.target('done',visits);asm.op(0x57,visits)
    for index in range(sequence_length):
        asm.push(96*index,times=repetitions);asm.op(0x35,repetitions);asm.push(64,times=repetitions);asm.op(0x52,repetitions)
        store(96,0,repetitions);store(128,0,repetitions)
        for value in (64,96,96,0,7,NATIVE_GAS):asm.push(value,times=repetitions)
        asm.op(0xfa,repetitions);check(1,repetitions);asm.op(0x3d,repetitions);check(64,repetitions)
        for memory_offset,data_offset in ((96,96*index+32),(128,96*index+64)):
            load(memory_offset,repetitions);asm.push(data_offset,times=repetitions);asm.op(0x35,repetitions);asm.op(0x14,repetitions);mutation._require(asm,repetitions)
    load(192,repetitions);asm.push(1,times=repetitions);asm.op(1,repetitions);asm.push(192,times=repetitions);asm.op(0x52,repetitions);asm.target('loop',repetitions);asm.op(0x56,repetitions)
    asm.label('done')
    for offset,value in ((0,1),(32,2),(160,0),(192,repetitions),(224,SENTINEL)):load(offset);check(value)
    asm.op(0);asm.label('failure',0);asm.op(0xfe,0)
    return asm.finish(),asm.counts


def gas_accounting(scalars,repetitions):
    scalars=_sequence(scalars);code,counts=_runtime(len(scalars),repetitions);data=payload(scalars)
    costs={0:0,1:3,0x14:3,0x15:3,0x35:3,0x36:2,0x3d:2,0x51:3,0x52:3,0x56:8,0x57:10,0x5b:1,0xfa:0,0xfe:0}
    ordinary=sum(n*(3 if 0x60<=int(op[-2:],16)<=0x7f else costs[int(op[-2:],16)]) for op,n in counts.items())
    zero=data.count(0);nonzero=len(data)-zero;intrinsic=21000+4*zero+16*nonzero;floor=21000+10*(zero+4*nonzero)
    calls=len(scalars)*repetitions;native=NATIVE_GAS*calls;access=WARM_ACCESS_GAS*calls;gross=intrinsic+ordinary+24+native+access
    return dict(intrinsic_gas=intrinsic,eip7623_floor_gas=floor,eip7623_floor_active=gross<floor,
        ordinary_opcode_gas=ordinary,memory_expansion_gas=24,native_gas_per_call=NATIVE_GAS,
        warm_access_gas_per_call=WARM_ACCESS_GAS,precompile_calls=calls,precompile_native_gas=native,
        call_access_gas=access,gross_gas=gross,charged_gas=max(gross,floor),refund_counter=0,
        refund_applied=0,runtime_bytes=len(code))


def _failures(gas):
    envelope=gas['charged_gas']+ENVELOPE_RESERVE
    return [name for name,failed in (
        ('transaction_fixture_cap',envelope>limits.TRANSACTION_GAS_CAP),
        ('eip7825_cap',envelope>limits.EIP7825_GAS_CAP),
        ('charged_block_budget',limits.ANCHOR_GAS+gas['charged_gas']>limits.BLOCK_BUDGET),
        ('ordered_header_prefix',envelope>limits.HEADER_GAS_LIMIT-limits.ANCHOR_GAS),
        ('runtime_size',gas['runtime_bytes']>24576),
        ('eip150_reserve',ENVELOPE_RESERVE<(NATIVE_GAS+62)//63)) if failed]


def fits(scalars,repetitions):return not _failures(gas_accounting(scalars,repetitions))


def full_repetitions(scalars):
    scalars=_sequence(scalars)
    if not fits(scalars,0):raise ValueError('zero control cannot fit')
    low,high=0,MAX_REPETITIONS
    while low<high:
        middle=(low+high+1)//2
        if fits(scalars,middle):low=middle
        else:high=middle-1
    if not 0<low<MAX_REPETITIONS:raise ValueError('full reference outside supported range')
    return low


def _make_case(scalars,recipe,level,repetitions):
    scalars=_sequence(scalars)
    if type(level) is not str or level not in ('zero','probe','full'):raise ValueError('level must be zero/probe/full')
    maximum=full_repetitions(scalars)
    if level=='probe':original.integer(repetitions,1,maximum-1,'probe repetitions');r=repetitions
    else:
        if repetitions is not None:raise ValueError('repetitions override requires probe level')
        r=0 if level=='zero' else maximum
    data=payload(scalars);code,counts=_runtime(len(scalars),r);gas=gas_accounting(scalars,r);envelope=gas_accounting(scalars,maximum)['charged_gas']+ENVELOPE_RESERVE
    request=original.make_case('call','stop',count=1,gas_limit=envelope)['request'];program=request['program'];routes=dict.fromkeys(program['expected_route_counts'],0);routes.update(counts)
    program.update(code_hex='0x'+code.hex(),input_hex='0x'+data.hex(),expected_route_counts=routes)
    data_hash=digest(data);prefix=f'ecmul-scalar-mix-{recipe}-k{len(scalars)}-{data_hash}'
    reduced=[scalar%GROUP_ORDER for scalar in scalars]
    return dict(sample_id=prefix+f'-r{r}',baseline_sample_id=prefix+'-r0',family='ecmul-scalar-mix',corpus_id=CORPUS+'-'+data_hash,
        role='control' if r==0 else 'candidate',data_role='development_probe',holdout_only=False,actuals_used_for_fitting=False,
        scope='Successful fixed-point ECMUL scalar vectors; literal node/full proposal and actual PGU remain separate. No fit, exported route A/I/S, isolated operation cost or global bound.',
        completion_gate='Independent full byte/curve/gas/native signature/root/witness/quote gates before actuals.',
        parameters=dict(recipe=recipe,workload_level=level,sequence_length=len(scalars),repetitions=r,full_repetitions=maximum,
            scalars_hex=[f'0x{s:064x}' for s in scalars],reduced_scalars_hex=[f'0x{s:064x}' for s in reduced],
            source_derived_work_units=[s.bit_length()+s.bit_count() for s in reduced],
            expected_outputs_hex=['0x'+b''.join(v.to_bytes(32,'big') for v in expected_point(s)).hex() for s in scalars],
            precompile_address='0x'+'00'*19+'07',input_point=[1,2],expected_precompile_calls=len(scalars)*r,
            source_derived_native_gas_per_call=NATIVE_GAS,requested_gas_per_call=NATIVE_GAS,warm_access_gas_per_call=WARM_ACCESS_GAS,
            primary_code_owner=PRIMARY,primary_code_owner_count=1,transaction_kind='call',transaction_nonce=0,
            user_transactions_per_block=1,all_user_transactions=1,block_count=1,all_transactions_including_anchor=2,
            transaction_gas_limit=envelope,transaction_envelope_reserve=ENVELOPE_RESERVE,expected_anchor_gas=limits.ANCHOR_GAS,
            header_gas_limit=limits.HEADER_GAS_LIMIT,charged_gas_budget=limits.BLOCK_BUDGET,
            header_prefix_remaining_gas=[limits.HEADER_GAS_LIMIT-limits.ANCHOR_GAS],expected_persistent_storage_reads=0,expected_persistent_storage_writes=0,
            input_memory_offset=0,scalar_memory_offset=64,output_memory_offset=96,output_bytes=64,
            protected_zero_memory_offset=160,counter_memory_offset=192,protected_memory_offset=224,sentinel=SENTINEL,
            peak_memory_bytes=256,memory_words=8,runtime_bytes=len(code),runtime_sha256=digest(code),
            calldata_bytes=len(data),calldata_zero_bytes=data.count(0),calldata_nonzero_bytes=len(data)-data.count(0),calldata_sha256=data_hash,
            gas_accounting_per_user=gas,expected_user_gross_gas=gas['gross_gas'],expected_user_charged_gas=gas['charged_gas'],
            expected_user_refund_counter=0,expected_user_refund_applied=0,expected_block_charged_gas=limits.ANCHOR_GAS+gas['charged_gas'],
            full_plus_one_constraint_failures=_failures(gas_accounting(scalars,maximum+1)),
            expected_terminal='STOP',expected_user_success=True,expected_user_receipt_statuses=[True],expected_anchor_success=True,
            expected_exceptional_instructions=0,execution_profile='evm-gas-only',preload_created_accounts_or_code=False),request=request)


def make_case(recipe='mixed',level='full',*,repetitions=None):return _make_case(scalars_for(recipe),recipe,level,repetitions)


def make_custom_case(scalars,level='full',*,repetitions=None):return _make_case(scalars,'custom',level,repetitions)


def source_modules():
    return dict(ecmul_scalar_mix=__import__(__name__),initcode_preparation=initcode_preparation,
        precompile_workload=curve,spawn_memory=spawn_memory,storage_mutation=mutation,top_level_create=original,warm_read_catalogs=limits)


def helpers():
    return {f'ecmul_scalar_mix.{f.__name__}':f for f in (_sequence,scalars_for,_reduced_point.__wrapped__,expected_point,payload,
        _runtime,gas_accounting,_failures,fits,full_repetitions,_make_case,make_case,make_custom_case)}|{
        '_Factory':_Factory,'precompile_workload.expected_output':curve.expected_output,'precompile_workload._point_add':curve._point_add,
        'storage_mutation._check':mutation._check,'storage_mutation._require':mutation._require,
        'top_level_create.make_case':original.make_case,'top_level_create.integer':original.integer,
        'spawn_memory.digest':digest,'spawn_memory.encoded':encoded}
