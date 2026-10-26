"""Reproducible local-model retrieval evaluation, always in an isolated workspace.

No download, tuning or global integration changes. Corpus and grades are frozen.
Unjudged symbols count as zero for reported nDCG, not as proven irrelevant.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
from contextlib import nullcontext
import time
from retrieval_fixture import workspace, assert_restored
from retrieval_summary import metrics, summarize

p = argparse.ArgumentParser()
p.add_argument('--binary', required=True)
p.add_argument('--model-dir', required=True)
p.add_argument('--dataset', default='tests/fixtures/retrieval_bkndb')
p.add_argument('--output')
p.add_argument('--latency-repeats', type=int, default=5)
p.add_argument('--queries', type=int, default=80)
p.add_argument('--reuse-workspace', action='store_true')
a = p.parse_args()
assert 1 <= a.queries <= 80 and a.latency_repeats >= 1
binary, model = str(Path(a.binary).resolve()), str(Path(a.model_dir).resolve())
out = Path(a.output or 'target/retrieval-evaluation-baseline').resolve()
out.mkdir(parents=True, exist_ok=True)
root = Path(a.dataset)
manifest = json.loads((root / 'manifest.json').read_text(encoding='utf-8'))
raw = (root / 'corpus.json').read_bytes()
assert hashlib.sha256(raw).hexdigest() == manifest['corpus_sha256']
corpus = json.loads(raw)
queries = json.loads((root / 'queries.json').read_text(encoding='utf-8'))['queries'][:a.queries]
records, operations = [], []
def save():
    (out / 'rankings.json').write_text(json.dumps(records, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    (out / 'operations.json').write_text(json.dumps(operations, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
def cli(project, *args):
    start = time.perf_counter()
    with (out / 'cli-progress.log').open('a', encoding='utf-8') as log:
        run = subprocess.run([binary, *args, '--project', str(project), '--json'], stdout=subprocess.PIPE, stderr=log, encoding='utf-8')
    value = json.loads(run.stdout)
    operations.append({'command': args, 'elapsed_ms': (time.perf_counter()-start)*1000, 'exit_code': run.returncode, 'result': value})
    save()
    assert run.returncode == 0, value
    return value

class MCP:
    def __init__(self, project):
        self.stderr = (out / 'mcp-stderr.log').open('a', encoding='utf-8')
        self.child = subprocess.Popen([binary, 'mcp', '--project', str(project)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.stderr, text=True, encoding='utf-8')
        self.id = 0
        self.request('initialize', {'protocolVersion':'2025-06-18','capabilities':{},'clientInfo':{'name':'retrieval-evaluation','version':'1'}})
        self.child.stdin.write(json.dumps({'jsonrpc':'2.0','method':'notifications/initialized'})+'\n')
        self.child.stdin.flush()
    def request(self, method, params):
        self.id += 1
        self.child.stdin.write(json.dumps({'jsonrpc':'2.0','id':self.id,'method':method,'params':params})+'\n')
        self.child.stdin.flush()
        while True:
            line = self.child.stdout.readline()
            assert line, f'MCP exited: {self.child.poll()}'
            value = json.loads(line)
            if value.get('id') == self.id:
                assert 'error' not in value, value
                return value['result']
    def search(self, query, mode, **filters):
        start = time.perf_counter()
        result = self.request('tools/call', {'name':'code_rcl_search','arguments':{'query':query,'retrieval':mode,'no_sync':True,'limit':100,'max_output_kb':2048, **filters}})
        elapsed = (time.perf_counter()-start)*1000
        assert not result.get('isError'), result
        envelope = result['structuredContent']
        data = envelope['data']
        hits = data.get('matches') if mode != 'lexical' else data['exact_symbols'] or data['fuzzy_suggestions']
        return hits, elapsed, envelope['metadata']
    def close(self):
        self.child.stdin.close()
        assert self.child.wait(timeout=30) == 0
        self.stderr.close()

def identity(hit):
    return hit.get('path'), hit.get('name')

with nullcontext(workspace(out,corpus,manifest,a.reuse_workspace)) as project:
    cli(project,'embeddings','setup','--model-dir',model)
    cli(project,'embeddings','sync','--progress','off')
    cli(project,'embeddings','sync','--progress','off')
    # Incremental edit and a large-file edit are reversible within this fixture only.
    first = 'crates/bkndb-core/src/relational/search/vector.rs'
    for path, declaration in [(first,'pub fn pack_vector(v: &[f32]) -> PropValue {'),('bindings/uniffi/bkndb_ffi/bkndb_ffi.kt','public fun uniffiEnsureInitialized() {')]:
        dest = project / path
        assert declaration in corpus[path]
        dest.write_text(corpus[path].replace(declaration, declaration + '\n    // benchmark symbol-body edit', 1), encoding='utf-8', newline='\n')
        cli(project,'embeddings','sync','--progress','off')
        dest.write_text(corpus[path], encoding='utf-8', newline='\n')
        cli(project,'embeddings','sync','--progress','off')
    cli(project,'search',queries[0]['query'],'--semantic')
    client = MCP(project)
    try:
        for query in queries:
            for mode in ['lexical','semantic','hybrid']:
                hits, elapsed, metadata = client.search(query['query'],mode)
                records.append({'query':query,'mode':mode,'experiment':'baseline','elapsed_ms':elapsed,'metadata':metadata,'metrics':metrics(hits,query['labels']),'ranking':hits})
            save()
            print(f"evaluated {query['id']} ({query['split']})",flush=True)
        latency=[]
        for repeat in range(a.latency_repeats):
            for query in queries[:6]:
                _,elapsed,metadata=client.search(query['query'],'semantic')
                latency.append({'query':query['id'],'repeat':repeat,'elapsed_ms':elapsed,'engine_timings_ms':metadata.get('timings_ms')})
        operations.append({'warm_query_latency':latency})
    finally:
        client.close()
    # A fresh process separates cold model load from warm requests in one MCP process.
    client=MCP(project)
    try:
        _,elapsed,metadata=client.search(queries[0]['query'],'semantic')
        operations.append({'restarted_query_ms':elapsed,'metadata':metadata})
    finally:
        client.close()
    save()
    assert_restored(project,corpus)

summary=summarize(records,manifest,'baseline','source bytes restored and verified',subprocess.check_output([binary,'--version'],text=True).strip(),hashlib.sha256(Path(binary).read_bytes()).hexdigest())
summary['workspace_reused']=a.reuse_workspace
summary['indexing_measurement']='existing index' if a.reuse_workspace else 'cold isolated index'
(out/'summary.json').write_text(json.dumps(summary,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
save()
print(f'Wrote rankings, grades, phase timings and summary to {out}')
