"""Explicit CI dispatch only: build optimized harness, use verified model, retain results."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

p=argparse.ArgumentParser()
p.add_argument('--binary',default='target/debug/code-rcl.exe' if os.name=='nt' else 'target/debug/code-rcl')
p.add_argument('--model-dir')
p.add_argument('--output',default='target/indexing-speed')
a=p.parse_args()
with tempfile.TemporaryDirectory(prefix='code-rcl-speed-model-') as project:
    setup=[str(Path(a.binary).resolve()),'embeddings','setup','--project',project,'--json']
    if a.model_dir:setup+=['--model-dir',a.model_dir]
    model=json.loads(subprocess.check_output(setup,text=True,encoding='utf-8'))['data']['model_dir']
build=subprocess.run(['cargo','test','--locked','--release','--bin','code-rcl','--no-run','--message-format=json'],stdout=subprocess.PIPE,text=True,encoding='utf-8',check=True)
harness=None
for line in build.stdout.splitlines():
    record=json.loads(line)
    if record.get('reason')=='compiler-artifact' and record.get('executable') and record.get('profile',{}).get('test') and record.get('target',{}).get('name')=='code-rcl':harness=record['executable']
assert harness,'Cargo did not report a test executable'
subprocess.run([sys.executable,'scripts/benchmark-indexing.py','--harness',harness,'--model-dir',model,'--output',a.output,'--stage','all'],check=True)
