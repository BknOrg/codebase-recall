"""Small controlled native profile: cold/no-op/one-symbol/large-file/warm/restart."""
import argparse
import json
from pathlib import Path
import statistics
import subprocess
import tempfile
import time

p=argparse.ArgumentParser()
p.add_argument('--binary',required=True)
p.add_argument('--model-dir',required=True)
p.add_argument('--output',default='target/embedding-incremental-profile.json')
a=p.parse_args()
binary=str(Path(a.binary).resolve());model=str(Path(a.model_dir).resolve())
records=[]
with tempfile.TemporaryDirectory(prefix='code-rcl-incremental-') as temp:
    project=Path(temp)
    sources={'small.rs':'pub fn small_action() { let value = 1; }\n',
             'large.rs':'pub fn large_file_action() { let value = 1; }\n'+'// padding outside symbol\n'*5000}
    for path,source in sources.items(): (project/path).write_bytes(source.encode())
    def cli(label,*args):
        start=time.perf_counter()
        run=subprocess.run([binary,*args,'--json','--project',str(project)],capture_output=True,text=True,encoding='utf-8',timeout=180)
        assert run.returncode==0,(run.stdout,run.stderr)
        result=json.loads(run.stdout)
        records.append({'scenario':label,'elapsed_ms':(time.perf_counter()-start)*1000,'result':result})
        return result
    cli('model_setup','embeddings','setup','--model-dir',model)
    cli('cold_index','embeddings','sync','--progress','off')
    noop=cli('unchanged_sync','embeddings','sync','--progress','off')
    assert noop['data']['embedded_chunks']==0
    assert 'model_load' not in noop['metadata']['timings_ms']
    for path,label in [('small.rs','single_symbol'),('large.rs','large_file')]:
        (project/path).write_bytes(sources[path].replace('value = 1','value = 2').encode())
        changed=cli(label,'embeddings','sync','--progress','off')
        assert changed['data']['embedded_chunks']==1,changed
        (project/path).write_bytes(sources[path].encode())
        cli(label+'_restore','embeddings','sync','--progress','off')
    def mcp_profile(label,count):
        with tempfile.TemporaryFile(mode='w+',encoding='utf-8') as log:
            child=subprocess.Popen([binary,'mcp','--project',str(project)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=log,text=True,encoding='utf-8')
            def request(id,method,params):
                child.stdin.write(json.dumps({'jsonrpc':'2.0','id':id,'method':method,'params':params})+'\n');child.stdin.flush()
                while True:
                    response=json.loads(child.stdout.readline())
                    if response.get('id')==id: return response
            request(1,'initialize',{'protocolVersion':'2025-06-18'})
            child.stdin.write(json.dumps({'jsonrpc':'2.0','method':'notifications/initialized'})+'\n');child.stdin.flush()
            samples=[]
            for i in range(count):
                start=time.perf_counter()
                result=request(i+2,'tools/call',{'name':'code_rcl_search','arguments':{'query':'large file action','retrieval':'semantic','no_sync':True}})['result']
                assert not result.get('isError'),result
                value=result['structuredContent']
                assert value['data']['matches']
                elapsed=(time.perf_counter()-start)*1000
                samples.append(elapsed)
                records.append({'scenario':label,'iteration':i,'elapsed_ms':elapsed,'result':value})
            child.stdin.close();assert child.wait(timeout=30)==0
            return samples
    samples=mcp_profile('mcp_first_and_warm_queries',6)
    mcp_profile('mcp_process_restart',1)
    for path,source in sources.items(): assert (project/path).read_bytes()==source.encode()
    result={'fixture':'two functions; large file padded outside symbols; not the full BknDb corpus',
            'model_revision':'5090578d9565bb06545b4552f76e6bc2c93e4a66','threads':4,'batch_size':4,'dimensions':768,
            'warm_median_ms':statistics.median(samples[1:]),'warm_p95_ms':max(samples[1:]),'source_bytes_restored':True,'operations':records}
out=Path(a.output);out.parent.mkdir(parents=True,exist_ok=True)
out.write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
print(f'Native incremental/no-op and repeated/restarted query profile passed: {out}')
