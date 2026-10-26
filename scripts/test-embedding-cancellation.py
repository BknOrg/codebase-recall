"""Real console interrupt -> committed-file recovery -> semantic retrieval, isolated."""
import argparse
import json
import os
from pathlib import Path
import signal
import shutil
import subprocess
import sys
import tempfile
import time

p=argparse.ArgumentParser()
p.add_argument('--binary',required=True)
p.add_argument('--model-dir',required=True)
p.add_argument('--batching',choices=['file','length'],default='file')
a=p.parse_args()
binary=str(Path(a.binary).resolve())
model=str(Path(a.model_dir).resolve())
def interrupt(child, twice=False):
    if os.name != 'nt':
        child.send_signal(signal.SIGINT)
        if twice:
            time.sleep(.05)
            child.send_signal(signal.SIGINT)
    else:
        # Separate hidden console: never signal the user's shell or other MCP jobs.
        helper='''import ctypes,sys,time
k=ctypes.WinDLL('kernel32',use_last_error=True)
k.FreeConsole()
assert k.AttachConsole(int(sys.argv[1])),ctypes.get_last_error()
assert k.SetConsoleCtrlHandler(None,True)
assert k.GenerateConsoleCtrlEvent(0,0),ctypes.get_last_error()
if sys.argv[2] == 'twice':
 time.sleep(.05)
 assert k.GenerateConsoleCtrlEvent(0,0),ctypes.get_last_error()
time.sleep(.2)
'''
        subprocess.run([sys.executable,'-c',helper,str(child.pid),'twice' if twice else 'once'],check=True)

with tempfile.TemporaryDirectory(prefix='code-rcl-cancel-') as temp:
    project=Path(temp)
    # Use a private executable copy so concurrent development builds cannot collide
    # with Windows image-file locks. Native sidecars follow the copied executable.
    executable=Path(binary)
    bin_dir=project/'bin'
    bin_dir.mkdir()
    for file in executable.parent.iterdir():
        if file==executable or file.suffix.lower() in {'.dll','.so','.dylib'} or '.so.' in file.name:
            if file.is_file():
                shutil.copy2(file,bin_dir/file.name)
    binary=str(bin_dir/executable.name)
    (project/'.code-rcl').mkdir()
    (project/'.code-rcl/config.toml').write_text('[embeddings]\nmodel_dir='+json.dumps(model)+'\nthreads=1\nbatch_size=1\nbatching='+json.dumps(a.batching)+'\n',encoding='utf-8')
    for name in ['a','b','c']:
        (project/f'{name}.rs').write_text(f'pub fn cancel_{name}() {{\n'+''.join(f'let value_{i} = {i};\n' for i in range(500))+'}\n',encoding='utf-8')
    options={}
    if os.name=='nt':
        info=subprocess.STARTUPINFO()
        info.dwFlags|=subprocess.STARTF_USESHOWWINDOW
        info.wShowWindow=0
        options={'creationflags':subprocess.CREATE_NEW_CONSOLE,'startupinfo':info}
    with (project/'stderr.log').open('w',encoding='utf-8') as err:
        child=subprocess.Popen([binary,'embeddings','sync','--json','--project',str(project)],stdout=subprocess.PIPE,stderr=err,text=True,encoding='utf-8',**options)
        try:
            deadline=time.monotonic()+180
            while time.monotonic()<deadline:
                assert child.poll() is None, f'sync ended before interruption: {child.returncode}'
                try:
                    state=json.loads((project/'.code-rcl/embedding-operation.json').read_text(encoding='utf-8'))
                except (OSError,json.JSONDecodeError):
                    time.sleep(.05)
                    continue
                if state['phase']=='inference' and state['files_done']>=1:
                    # Exercise the actual cache owner, not only a synthetic lock.
                    status=subprocess.run([binary,'embeddings','status','--json','--project',str(project)],capture_output=True,text=True,encoding='utf-8',timeout=30)
                    assert status.returncode==0,(status.stdout,status.stderr)
                    live=json.loads(status.stdout)['data']
                    assert live['database_busy'] and live['operation']['active'],live
                    assert live['freshness']=='unverified',live
                    interrupt(child)
                    break
                time.sleep(.05)
            else:
                raise AssertionError('no committed-file/inference checkpoint within 180 seconds')
            stdout,_=child.communicate(timeout=120)
            assert child.returncode==130, (child.returncode,stdout,(project/'stderr.log').read_text(encoding='utf-8'))
            result=json.loads(stdout)
            assert result['metadata']['operation']['state']=='cancelled',result
        finally:
            if child.poll() is None:
                child.kill();child.wait()
    def call(*args):
        run=subprocess.run([binary,*args,'--json','--project',str(project)],capture_output=True,text=True,encoding='utf-8',timeout=240)
        assert run.returncode==0,(run.stdout,run.stderr)
        return json.loads(run.stdout)
    state=call('embeddings','status')
    assert state['data']['indexed_chunks']>0 and state['data']['pending_files']>0 and not state['data']['complete'],state
    resumed=call('embeddings','sync','--progress','off')
    assert resumed['data']['index']['complete'],resumed
    assert call('search','cancel operation','--semantic')['data']['matches']
    # Force exit during one long native batch, leaving a running snapshot.
    (project/'.code-rcl/config.toml').write_text('[embeddings]\nmodel_dir='+json.dumps(model)+'\nthreads=1\nbatch_size=32\nbatching='+json.dumps(a.batching)+'\n',encoding='utf-8')
    (project/'a.rs').write_text('pub fn cancel_a() {\n'+''.join(f'let forced_{i} = {i};\n' for i in range(1000))+'}\n',encoding='utf-8')
    with (project/'forced-stderr.log').open('w',encoding='utf-8') as err:
        child=subprocess.Popen([binary,'embeddings','sync','--json','--progress','off','--project',str(project)],stdout=subprocess.PIPE,stderr=err,text=True,encoding='utf-8',**options)
        try:
            deadline=time.monotonic()+180
            while time.monotonic()<deadline:
                assert child.poll() is None, 'forced-exit fixture completed too early'
                state=json.loads((project/'.code-rcl/embedding-operation.json').read_text(encoding='utf-8'))
                if state['state']=='running' and state['phase']=='inference':
                    interrupt(child, twice=True)
                    break
                time.sleep(.05)
            else:
                raise AssertionError('forced-exit inference checkpoint timed out')
            child.communicate(timeout=120)
            assert child.returncode==130,child.returncode
        finally:
            if child.poll() is None:
                child.kill();child.wait()
    state=call('embeddings','status')
    assert state['data']['operation']['state']=='interrupted',state
    assert not state['data']['complete'],state
    assert call('embeddings','sync','--progress','off')['data']['index']['complete']
    staging=project/'.code-rcl/embedding-staging'
    if staging.exists():assert not list(staging.iterdir()),'unfinished staging survived resume'
    assert call('search','cancel operation','--semantic')['data']['matches']
    print('Live busy status, real Ctrl+C, forced interrupt, retained commits, resume and semantic search passed.')
