#!/usr/bin/env python3
"""Generate24 finite repeated-initcode probes; use unchanged native/prover tooling."""
import argparse
from collections import Counter
import json
from pathlib import Path

import top_level_create as original
from spawn_memory import digest, encoded, write

MODEL='a2988920125e304121acb629b9b96fd1ad85ee707c89978240408c81e2a42f3e'
PATTERNS={'zero':b'\x00','jumpdest':b'\x5b','push32':b'\x7f'+b'\x5b'*32}


class _Factory:
    """Local assembler with explicit executed multiplicities, not a runtime interpreter."""
    def __init__(self):
        self.code=bytearray();self.counts=Counter();self.labels={};self.fixups=[]

    def op(self,opcode,times=1):
        self.code.append(opcode);self.counts[f'opcode:0x{opcode:02x}']+=times

    def push(self,value,width=3,times=1):
        self.op(0x5f+width,times);self.code.extend(value.to_bytes(width,'big'))

    def target(self,label,times=1):
        self.op(0x61,times);self.fixups.append((len(self.code),label));self.code.extend(b'\x00\x00')

    def label(self,name,times=1):
        self.labels[name]=len(self.code);self.op(0x5b,times)

    def finish(self):
        for offset,label in self.fixups:self.code[offset:offset+2]=self.labels[label].to_bytes(2,'big')
        return bytes(self.code)


def _write_byte(factory,offset,value):
    factory.push(value,1);factory.push(offset);factory.op(0x53)


def make_case(family='create',length=49152,repeats=16,*,pattern='jumpdest',child_output_bytes=0,outer_revert=False):
    if type(family) is not str or family not in ('create','create2'):raise ValueError('family must be create or create2')
    original.integer(length,0,49152,'initcode bytes');original.integer(repeats,0,2048,'repetitions')
    if type(pattern) is not str or pattern not in PATTERNS:raise ValueError('unknown tail pattern')
    if type(child_output_bytes) is not int or child_output_bytes not in (0,1):raise ValueError('child output bytes must be0or1')
    if type(outer_revert) is not bool:raise ValueError('outer_revert must be boolean')
    prefix=bytes.fromhex('60016000f3') if child_output_bytes else b'\x00'
    if length==0:
        if child_output_bytes or pattern!='zero':raise ValueError('empty initcode uses zero pattern and empty output')
        prefix=b''
    elif length<len(prefix):raise ValueError('initcode is shorter than child prefix')
    tail=length-len(prefix);chunk=PATTERNS[pattern];image=prefix+(chunk*((tail+len(chunk)-1)//len(chunk)))[:tail]
    asm=_Factory()
    # Build only the short seed and logarithmic memory copies, never an embedded
    # long transaction payload. Repeats0 still prepares this same memory image.
    seeded=min(tail,len(chunk))
    for offset,value in enumerate(chunk[:seeded],len(prefix)):_write_byte(asm,offset,value)
    filled=seeded
    while filled<tail:
        copied=min(filled,tail-filled)
        asm.push(copied);asm.push(len(prefix));asm.push(len(prefix)+filled);asm.op(0x5e)
        filled+=copied
    for offset,value in enumerate(prefix):_write_byte(asm,offset,value)
    # Stack holds the increasing child index. Both count and length use fixed
    # PUSH3 widths; changing N changes an immediate, not body length or unrolling.
    asm.push(0);asm.label('loop',repeats+1)
    asm.op(0x80,repeats+1);asm.push(repeats,times=repeats+1);asm.op(0x14,repeats+1)
    asm.target('done',repeats+1);asm.op(0x57,repeats+1)
    if family=='create2':asm.op(0x80,repeats)  # Salt=child index, unique in this CALL.
    asm.push(length,times=repeats);asm.push(0,1,repeats);asm.push(0,1,repeats)
    asm.op(0xf0 if family=='create' else 0xf5,repeats)
    asm.op(0x80,repeats);asm.op(0x15,repeats);asm.target('failure',repeats);asm.op(0x57,repeats)
    asm.op(0x50,repeats);asm.push(1,1,repeats);asm.op(0x01,repeats);asm.target('loop',repeats);asm.op(0x56,repeats)
    asm.label('done');asm.op(0x50)
    if outer_revert:asm.push(0,1);asm.push(0,1);asm.op(0xfd)
    else:asm.op(0x00)
    asm.label('failure',0);asm.op(0xfe,0)  # Failed creation can never reach the declared terminal.
    code=asm.finish()
    child_routes={'opcode:0x60':2*repeats,'opcode:0xf3':repeats} if child_output_bytes else {'opcode:0x00':repeats}
    routes=original.make_case('call','stop')['request']['program']['expected_route_counts']
    routes={key:0 for key in routes};routes.update(asm.counts)
    for key,value in child_routes.items():routes[key]=routes.get(key,0)+value
    row=original.make_case('call','stop',gas_limit=15_000_000)
    row['request']['expected_user_success']=not outer_revert
    row['request']['program'].update(code_hex='0x'+code.hex(),expected_route_counts=routes)
    sid=f'initcode-preparation-{family}-l{length}-{pattern}-n{repeats}-out{child_output_bytes}-'+('outer-revert' if outer_revert else 'outer-stop')
    row.update(sample_id=sid,family='repeated-initcode-preparation',baseline_sample_id=sid,role='control' if repeats==0 or length==0 else 'candidate',
        data_role='model_development_probe',holdout_only=False,
        completion_gate='all children return nonzero CREATE addresses; all completed child/parent opcode counts, user receipt status and actual factory nonce match',
        scope='Finite whole-proposal actual/node workload search. Complete quotes and matched deltas remain separate; no tariff refit, per-transaction actual, throughput or maximum-capacity claim.')
    row['parameters']={'creation_family':family,'initcode_bytes':length,'initcode_sha256':digest(image),'repetitions':repeats,'tail_pattern':pattern,
        'child_output_bytes':child_output_bytes,'outer_revert':outer_revert,'expected_user_success':not outer_revert,
        'expected_factory_nonce':0 if outer_revert else repeats,'expected_present_child_accounts':0 if outer_revert else repeats,
        'expected_caller_nonce_after':1,'expected_code_preparation':{'nonempty_events':repeats*child_output_bytes,'total_bytes':repeats*child_output_bytes},
        'factory_runtime_bytes':len(code),'factory_runtime_sha256':digest(code),'transaction_kind':'call','count_per_block':1,'block_count':1,
        'transaction_gas_limit':15_000_000,'outer_calldata_bytes':0,'initcode_location':'parent execution memory built with MCOPY',
        'create2_salts':'consecutive integers0throughN-1' if family=='create2' else None,'preload_created_accounts_or_code':False}
    return row


def prepare(output):
    plans=[]
    for family in ('create','create2'):
        plans += [make_case(family,length,n) for length in (4096,49152) for n in (1,16,128)]
        plans += [make_case(family,49152,16,pattern=p) for p in ('zero','push32')]
        plans += [make_case(family,49152,0),make_case(family,49152,16,outer_revert=True),
                  make_case(family,49152,16,child_output_bytes=1),make_case(family,0,16,pattern='zero')]
    ids={row['sample_id'] for row in plans};assert len(plans)==len(ids)==24
    for row in plans:
        p=row['parameters'];f,l,n=p['creation_family'],p['initcode_bytes'],p['repetitions']
        if p['outer_revert'] or p['child_output_bytes'] or p['tail_pattern']!='jumpdest' or l==0:
            row['baseline_sample_id']=make_case(f,4096 if l==0 else 49152,16)['sample_id']
            row['comparison_dimension']=('ancestor_rollback' if p['outer_revert'] else 'child_output' if p['child_output_bytes'] else 'initcode_length' if l==0 else 'tail_content')
        else:
            row['baseline_sample_id']=make_case(f,l,1)['sample_id'] if n>0 else row['sample_id']
            row['comparison_dimension']='repetition' if n>1 else 'absolute_control'
        if l==49152 and n in (1,16,128) and p['tail_pattern']=='jumpdest' and not p['outer_revert'] and not p['child_output_bytes']:
            row['same_count_short_length_reference_sample_id']=make_case(f,4096,n)['sample_id']
            row['same_image_zero_repetitions_reference_sample_id']=make_case(f,49152,0)['sample_id']
        assert row['baseline_sample_id'] in ids
    output.mkdir(parents=True,exist_ok=False);(output/'requests').mkdir()
    rows=[]
    for row in plans:
        raw=encoded(row.pop('request'));path=f"requests/{row['sample_id']}.json";write(output/path,raw)
        rows.append(row|{'request_path':path,'request_sha256':digest(raw)})
    write(output/'catalog.json',encoded({'schema_version':1,'purpose':'Repeated fresh initcode preparation with successful CREATE/CREATE2; finite model-development observations',
        'frozen_model_id':MODEL,'generator_sha256':digest(Path(__file__).read_bytes()),
        'source_sha256':{name:digest(Path(__file__).with_name(name).read_bytes()) for name in ('top_level_create.py','spawn_memory.py')},
        'interpretation':'Existing CREATE/CREATE2 wrapper prices remain active. Different content or output shapes can change parent work and CREATE2 trie addresses; they are not pure initcode-only deltas.',
        'cases':rows}))
    return len(rows)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output-dir',type=Path,required=True)
    print(json.dumps({'prepared':prepare(parser.parse_args().output_dir)}))
